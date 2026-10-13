//! `MailProvider` for Gmail.

use super::api::{ApiError, GmailClient};
use super::mapping::{self, base64url_encode, gmail_id, label_from_json, message_from_json};
use super::mime::build_raw;
use crate::{draft_message_id, BoxFuture, MailProvider, MailboxSnapshot, ProviderError, RemoteChanges, SendContext, SendOutcome, SentReceipt};
use overlook_core::*;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tracing::{info, warn};

/// How many recent messages the first download fetches. Progressive history download is a
/// phase 1 item; this keeps the first sync to a few minutes under Gmail's quota.
pub const INITIAL_MESSAGES: usize = 150;

/// Views covered by the first download: (Gmail search, how many newest messages). Gmail's
/// list excludes Trash and Spam unless asked, and a plain "newest 150" would rarely reach
/// archived or trashed mail, so each view is listed on its own.
pub const INITIAL_VIEWS: &[(&str, usize)] = &[
    ("in:inbox -in:chats", 100),
    ("in:sent", 40),
    ("-in:inbox -in:sent -in:trash -in:spam -in:drafts -in:chats", 40),
    ("is:starred -in:trash", 20),
    ("in:trash", 20),
    ("in:spam", 10),
];

pub struct GmailProvider {
    client: Arc<GmailClient>,
    initial_messages: usize,
    inbox_limit: usize,
}

impl GmailProvider {
    pub fn new(client: Arc<GmailClient>) -> Self {
        Self { client, initial_messages: INITIAL_MESSAGES, inbox_limit: 100 }
    }

    /// Newest Inbox messages to list on first download; other views scale with it.
    pub fn with_inbox_limit(mut self, inbox_limit: usize) -> Self {
        self.inbox_limit = inbox_limit.max(20);
        self.initial_messages = self.initial_messages.max(inbox_limit * 2);
        self
    }

    pub fn client(&self) -> &Arc<GmailClient> {
        &self.client
    }

    fn labels(&self, account: &Account) -> Result<Vec<Label>, ApiError> {
        let v = self.client.get("labels", &[])?;
        let mut labels: Vec<Label> = v
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| arr.iter().enumerate().filter_map(|(i, l)| label_from_json(&account.id, l, i as i32)).collect())
            .unwrap_or_default();
        // Stable order: system labels first in a fixed order, then user labels by name.
        labels.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        for (i, l) in labels.iter_mut().enumerate() {
            l.sort_order = i as i32;
        }
        Ok(labels)
    }

    fn label_index(labels: &[Label]) -> HashMap<String, LabelId> {
        labels
            .iter()
            .filter_map(|l| l.provider_id.as_ref().map(|p| (p.0.clone(), l.id.clone())))
            .collect()
    }

    fn fetch_message(&self, account: &Account, id: &str, labels: &HashMap<String, LabelId>) -> Result<Option<Message>, ApiError> {
        let mut message = match self.client.get(&format!("messages/{id}"), &[("format", "full")]) {
            Ok(v) => message_from_json(account, &v, labels),
            Err(ApiError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        // Gmail often ships the calendar part as an attachment reference instead of inline
        // text; fetch it so invitations are recognised.
        if let Some(m) = message.as_mut() {
            if m.invitation.is_none() {
                let ics = m
                    .attachments
                    .iter()
                    .find(|a| a.mime_type.starts_with("text/calendar") || a.mime_type.starts_with("application/ics") || a.filename.to_ascii_lowercase().ends_with(".ics"))
                    .and_then(|a| a.provider_id.clone());
                if let Some(aid) = ics {
                    if let Ok(v) = self.client.get(&format!("messages/{id}/attachments/{}", aid.0), &[]) {
                        if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                            let text = String::from_utf8_lossy(&mapping::base64url_decode(data)).into_owned();
                            m.invitation = overlook_core::ics::parse_invitation(&text);
                        }
                    }
                }
            }
        }
        Ok(message)
    }

    /// Lists ids for every view in [`INITIAL_VIEWS`], newest first, de-duplicated.
    fn list_initial_ids(&self) -> Result<Vec<String>, ApiError> {
        let mut seen = BTreeSet::new();
        let mut ids = Vec::new();
        let scale = self.inbox_limit as f32 / 100.;
        for (search, limit) in INITIAL_VIEWS {
            let limit = ((*limit as f32) * scale).round().max(10.) as usize;
            for id in self.list_ids(search, limit)? {
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }

    fn list_ids(&self, search: &str, limit: usize) -> Result<Vec<String>, ApiError> {
        let mut ids = Vec::new();
        let mut page: Option<String> = None;
        while ids.len() < limit {
            let remaining = (limit - ids.len()).min(100).to_string();
            let mut query = vec![("maxResults", remaining.as_str()), ("q", search), ("includeSpamTrash", "true")];
            if let Some(p) = page.as_deref() {
                query.push(("pageToken", p));
            }
            let v = self.client.get("messages", &query)?;
            let batch: Vec<String> = v
                .get("messages")
                .and_then(|m| m.as_array())
                .map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_owned)).collect())
                .unwrap_or_default();
            if batch.is_empty() {
                break;
            }
            ids.extend(batch);
            page = v.get("nextPageToken").and_then(|t| t.as_str()).map(str::to_owned);
            if page.is_none() {
                break;
            }
        }
        ids.truncate(limit);
        Ok(ids)
    }

    fn batch_modify(&self, ids: &[String], add: &[&str], remove: &[&str]) -> Result<(), ApiError> {
        if ids.is_empty() {
            return Ok(());
        }
        for chunk in ids.chunks(500) {
            self.client.post("messages/batchModify", &json!({ "ids": chunk, "addLabelIds": add, "removeLabelIds": remove }))?;
        }
        Ok(())
    }

    fn gmail_ids(account: &Account, messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|m| m.provider_id.as_ref().map(|p| p.0.clone()).or_else(|| gmail_id(&account.id, m.id.as_str())))
            .collect()
    }
}

impl MailProvider for GmailProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gmail
    }

    fn probe(&self) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async move {
            self.client.get("profile", &[]).map(|_| ()).map_err(ProviderError::from)
        })
    }

    fn fetch_snapshot(&self, account: &Account) -> BoxFuture<'_, Result<MailboxSnapshot, ProviderError>> {
        let account = account.clone();
        Box::pin(async move {
            // Take the history position before listing so no change is missed.
            let profile = self.client.get("profile", &[])?;
            let checkpoint = profile.get("historyId").and_then(|h| h.as_str().map(str::to_owned).or_else(|| h.as_u64().map(|n| n.to_string())));
            let labels = self.labels(&account)?;
            let mut ids = self.list_initial_ids()?;
            ids.truncate(self.initial_messages.max(ids.len().min(self.inbox_limit * 3)));
            info!("gmail: {} recent messages queued for download", ids.len());
            Ok(MailboxSnapshot {
                labels,
                messages: Vec::new(),
                message_ids: ids.into_iter().map(ProviderId).collect(),
                checkpoint,
            })
        })
    }

    fn fetch_messages(&self, account: &Account, ids: &[ProviderId], labels: &[Label]) -> BoxFuture<'_, Result<Vec<Message>, ProviderError>> {
        let account = account.clone();
        let ids: Vec<String> = ids.iter().map(|p| p.0.clone()).collect();
        let index = Self::label_index(labels);
        Box::pin(async move {
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                if let Some(m) = self.fetch_message(&account, id, &index)? {
                    out.push(m);
                }
            }
            Ok(out)
        })
    }

    fn fetch_changes(&self, account: &Account, checkpoint: &str) -> BoxFuture<'_, Result<RemoteChanges, ProviderError>> {
        let account = account.clone();
        let checkpoint = checkpoint.to_owned();
        Box::pin(async move {
            let mut added: BTreeSet<String> = BTreeSet::new();
            let mut changed: BTreeSet<String> = BTreeSet::new();
            let mut deleted: BTreeSet<String> = BTreeSet::new();
            let mut labels_touched = false;
            let mut new_checkpoint = checkpoint.clone();
            let mut page: Option<String> = None;
            loop {
                let mut query = vec![("startHistoryId", checkpoint.as_str()), ("maxResults", "500")];
                if let Some(p) = page.as_deref() {
                    query.push(("pageToken", p));
                }
                let v = match self.client.get("history", &query) {
                    Ok(v) => v,
                    Err(ApiError::NotFound) => return Err(ProviderError::CheckpointExpired),
                    Err(e) => return Err(e.into()),
                };
                if let Some(h) = v.get("historyId").and_then(|h| h.as_str()) {
                    new_checkpoint = h.to_owned();
                }
                for record in v.get("history").and_then(|h| h.as_array()).into_iter().flatten() {
                    let ids_in = |key: &str| -> Vec<String> {
                        record
                            .get(key)
                            .and_then(|a| a.as_array())
                            .map(|a| a.iter().filter_map(|x| x.get("message").and_then(|m| m.get("id")).and_then(|i| i.as_str()).map(str::to_owned)).collect())
                            .unwrap_or_default()
                    };
                    added.extend(ids_in("messagesAdded"));
                    deleted.extend(ids_in("messagesDeleted"));
                    let label_changes: Vec<String> = ids_in("labelsAdded").into_iter().chain(ids_in("labelsRemoved")).collect();
                    if !label_changes.is_empty() {
                        labels_touched = true;
                    }
                    changed.extend(label_changes);
                }
                page = v.get("nextPageToken").and_then(|t| t.as_str()).map(str::to_owned);
                if page.is_none() {
                    break;
                }
            }
            for id in &deleted {
                added.remove(id);
                changed.remove(id);
            }
            let labels = if labels_touched || !added.is_empty() { Some(self.labels(&account)?) } else { None };
            let index = match &labels {
                Some(l) => Self::label_index(l),
                None => HashMap::new(),
            };
            let mut upserts = Vec::new();
            for id in added.iter().chain(changed.iter()) {
                match self.fetch_message(&account, id, &index) {
                    Ok(Some(m)) => upserts.push(m),
                    Ok(None) => {}
                    Err(e) => {
                        warn!("gmail: could not fetch {id}: {e:?}");
                        return Err(e.into());
                    }
                }
            }
            Ok(RemoteChanges {
                labels,
                upserts,
                deleted: deleted.iter().map(|id| mapping::local_message_id(&account.id, id)).collect(),
                checkpoint: Some(new_checkpoint),
            })
        })
    }

    fn apply(&self, account: &Account, op: &PendingOperation, messages: &[Message], labels: &[Label]) -> BoxFuture<'_, Result<(), ProviderError>> {
        let account = account.clone();
        let op = op.clone();
        let ids = Self::gmail_ids(&account, messages);
        let labels = labels.to_vec();
        Box::pin(async move {
            let provider_label = |id: &LabelId| -> Result<String, ProviderError> {
                labels
                    .iter()
                    .find(|l| &l.id == id)
                    .and_then(|l| l.provider_id.as_ref().map(|p| p.0.clone()))
                    .ok_or_else(|| ProviderError::Rejected("label no longer exists".into()))
            };
            match &op.kind {
                OperationKind::SetRead { read: true, .. } => self.batch_modify(&ids, &[], &["UNREAD"])?,
                OperationKind::SetRead { read: false, .. } => self.batch_modify(&ids, &["UNREAD"], &[])?,
                OperationKind::SetStarred { starred: true, .. } => self.batch_modify(&ids, &["STARRED"], &[])?,
                OperationKind::SetStarred { starred: false, .. } => self.batch_modify(&ids, &[], &["STARRED"])?,
                OperationKind::Archive { .. } => self.batch_modify(&ids, &[], &["INBOX"])?,
                OperationKind::AddLabel { label_id, .. } => {
                    let l = provider_label(label_id)?;
                    self.batch_modify(&ids, &[l.as_str()], &[])?
                }
                OperationKind::RemoveLabel { label_id, .. } => {
                    let l = provider_label(label_id)?;
                    self.batch_modify(&ids, &[], &[l.as_str()])?
                }
                OperationKind::Trash { .. } => {
                    for id in &ids {
                        match self.client.post(&format!("messages/{id}/trash"), &Value::Null) {
                            Ok(_) | Err(ApiError::NotFound) => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                OperationKind::Untrash { .. } => {
                    for id in &ids {
                        match self.client.post(&format!("messages/{id}/untrash"), &Value::Null) {
                            Ok(_) | Err(ApiError::NotFound) => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                OperationKind::SendDraft { .. } => return Err(ProviderError::Rejected("sends go through send()".into())),
                kind if kind.is_calendar() => return Err(ProviderError::Rejected("calendar operations go through the calendar provider".into())),
                _ => {}
            }
            Ok(())
        })
    }

    fn fetch_attachment(&self, account: &Account, message: &Message, attachment: &Attachment) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let message_id = message.provider_id.as_ref().map(|p| p.0.clone()).or_else(|| gmail_id(&account.id, message.id.as_str()));
        let attachment_id = attachment.provider_id.as_ref().map(|p| p.0.clone());
        Box::pin(async move {
            let (Some(mid), Some(aid)) = (message_id, attachment_id) else {
                return Err(ProviderError::Rejected("this attachment has no Gmail id".into()));
            };
            let v = self.client.get(&format!("messages/{mid}/attachments/{aid}"), &[])?;
            let data = v.get("data").and_then(|d| d.as_str()).ok_or_else(|| ProviderError::Rejected("Gmail returned no attachment data".into()))?;
            Ok(mapping::base64url_decode(data))
        })
    }

    fn send(&self, account: &Account, draft: &Draft, context: SendContext) -> BoxFuture<'_, Result<SentReceipt, ProviderError>> {
        let account = account.clone();
        let draft = draft.clone();
        Box::pin(async move {
            let rfc_id = draft_message_id(&draft);
            let raw = build_raw(&account, &draft, context.in_reply_to.as_ref(), &rfc_id).map_err(ProviderError::Rejected)?;
            let mut body = json!({ "raw": base64url_encode(&raw) });
            if let Some(source) = &context.in_reply_to {
                if let Some(thread) = gmail_id(&account.id, source.conversation_id.as_str()) {
                    body["threadId"] = Value::String(thread);
                }
            }
            let response = match self.client.post("messages/send", &body) {
                Ok(v) => v,
                // A transport failure after the request left this machine is not a clean
                // failure: Gmail may have accepted the message.
                Err(ApiError::Transport(_)) => return Err(ProviderError::Uncertain),
                Err(e) => return Err(e.into()),
            };
            let id = response
                .get("id")
                .and_then(|i| i.as_str())
                .ok_or_else(|| ProviderError::Rejected("Gmail did not return a message id".into()))?
                .to_owned();
            let index = Self::label_index(&context.labels);
            let mut message = match self.fetch_message(&account, &id, &index) {
                Ok(Some(m)) => m,
                _ => {
                    // Accepted but not re-fetchable right now: build the Sent copy locally.
                    let mut m = crate::sample::sent_message(&account, &draft, context.in_reply_to.as_ref(), &ProviderId(id.clone()), &rfc_id, context.sent_label.as_ref(), overlook_core::time::now());
                    m.id = mapping::local_message_id(&account.id, &id);
                    if let Some(thread) = response.get("threadId").and_then(|t| t.as_str()) {
                        m.conversation_id = mapping::local_conversation_id(&account.id, thread);
                    }
                    m
                }
            };
            if let Some(sent) = &context.sent_label {
                message.label_ids.insert(sent.clone());
            }
            message.is_read = true;
            message.is_outgoing = true;
            Ok(SentReceipt { provider_id: ProviderId(id), rfc_message_id: Some(rfc_id), message })
        })
    }

    fn reconcile_send(&self, _account: &Account, draft: &Draft) -> BoxFuture<'_, Result<SendOutcome, ProviderError>> {
        let rfc_id = draft_message_id(draft);
        Box::pin(async move {
            let query = format!("rfc822msgid:{}", rfc_id.trim_matches(|c| c == '<' || c == '>'));
            let v = self.client.get("messages", &[("q", query.as_str()), ("maxResults", "1")])?;
            let found = v
                .get("messages")
                .and_then(|m| m.as_array())
                .and_then(|a| a.first())
                .and_then(|m| m.get("id"))
                .and_then(|i| i.as_str())
                .map(str::to_owned);
            Ok(match found {
                Some(id) => SendOutcome::Delivered(ProviderId(id)),
                None => SendOutcome::NotDelivered,
            })
        })
    }
}
