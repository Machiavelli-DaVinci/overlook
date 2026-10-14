//! Synchronization engine for one account.
//!
//! Order of a cycle: probe → download remote changes (merged without clobbering pending
//! local intent) → replay pending operations in sequence → record the outcome. Provider
//! acceptance is required before anything is called Sent, and uncertain sends are
//! reconciled before any retry. Runs on a background thread; the window reloads from the
//! store afterwards.

use overlook_core::threading::assign_conversations;
use overlook_core::time::now;
use overlook_core::*;
use overlook_provider::{CalendarProvider, MailProvider, ProviderError, SendContext, SendOutcome};
use overlook_store::Store;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Default, Clone)]
pub struct SyncOutcome {
    pub state: Option<SyncState>,
    pub calendar_changes: usize,
    /// The provider asked us to slow down; the caller should wait longer before retrying.
    pub rate_limited: bool,
    pub applied: usize,
    pub downloaded: usize,
    pub sent: Vec<String>,
    pub uncertain: Vec<String>,
    pub failed: Vec<String>,
    pub stopped_offline: bool,
}

pub const CALENDAR_SIGNIN_WORDING: &str = "Sign in again to connect your calendar. Mail keeps working meanwhile.";

pub fn calendar_status_key(account: &AccountId) -> String {
    format!("cal_status:{}", account.0)
}

pub fn calendar_status(store: &Store, account: &AccountId) -> SyncState {
    store.setting::<SyncState>(&calendar_status_key(account)).ok().flatten().unwrap_or(SyncState::Offline)
}

fn set_calendar_status(store: &Store, account: &AccountId, state: &SyncState) {
    let _ = store.set_setting(&calendar_status_key(account), state);
}

fn calendar_token_key(calendar: &CalendarId) -> String {
    format!("cal_sync:{}", calendar.0)
}

pub fn contacts_synced_key(account: &AccountId) -> String {
    format!("contacts_synced_at:{}", account.0)
}

/// Refreshes the address book once a day. Failures are logged and retried next time; they
/// never affect the mail sync state.
async fn sync_contacts(store: &Store, provider: &dyn MailProvider, account: &Account) {
    const DAY: i64 = 24 * 3_600_000;
    let key = contacts_synced_key(&account.id);
    let last = store.setting::<i64>(&key).ok().flatten().unwrap_or(0);
    if now() - last < DAY {
        return;
    }
    match provider.fetch_contacts(account).await {
        Ok(Some(contacts)) => {
            if let Err(e) = store.replace_contacts(&account.id, &contacts) {
                warn!("could not store contacts for {}: {e}", account.email);
                return;
            }
            info!("synced {} contacts for {}", contacts.len(), account.email);
            let _ = store.set_setting(&key, &now());
        }
        Ok(None) => {
            let _ = store.set_setting(&key, &now());
        }
        Err(e) => warn!("contacts sync failed for {}: {e}", account.email),
    }
}

/// Downloads calendars and events. Errors are recorded in the calendar status only; they
/// never block mail.
fn sync_calendar(store: &Store, provider: &dyn CalendarProvider, account: &Account, outcome: &mut SyncOutcome) {
    let acct = &account.id;
    if !provider.has_calendar_access() {
        set_calendar_status(store, acct, &SyncState::SignInRequired);
        return;
    }
    set_calendar_status(store, acct, &SyncState::Syncing);
    let calendars = match pollster_block(provider.list_calendars(account)) {
        Ok(c) => c,
        Err(e) => {
            set_calendar_status(store, acct, &state_for(&e));
            return;
        }
    };
    if let Err(e) = store.replace_calendars(acct, &calendars) {
        set_calendar_status(store, acct, &SyncState::SyncFailed(e.to_string()));
        return;
    }
    // Bumped when the event mapping gains fields: existing sync tokens are dropped once so
    // the window is downloaded again with the new fields.
    const CALENDAR_COVERAGE: u32 = 3;
    let coverage_key = format!("cal_coverage:{}", acct.0);
    let refresh = store.setting::<u32>(&coverage_key).ok().flatten().unwrap_or(1) < CALENDAR_COVERAGE;
    let calendars = store.calendars(acct).unwrap_or_default();
    for calendar in calendars.iter().filter(|c| c.visible) {
        let key = calendar_token_key(&calendar.id);
        let token = if refresh { None } else { store.setting::<String>(&key).ok().flatten() };
        let changes = match pollster_block(provider.fetch_events(account, calendar, token.as_deref())) {
            Ok(c) => c,
            Err(ProviderError::CheckpointExpired) => match pollster_block(provider.fetch_events(account, calendar, None)) {
                Ok(c) => c,
                Err(e) => {
                    set_calendar_status(store, acct, &state_for(&e));
                    return;
                }
            },
            Err(e) => {
                set_calendar_status(store, acct, &state_for(&e));
                return;
            }
        };
        // Local intent wins until it has been replayed: a locally deleted or edited event
        // that the server still reports must not come back in this download.
        let protected = store.pending_event_ids(acct).unwrap_or_default();
        let upserts: Vec<_> = changes.upserts.iter().filter(|e| !protected.contains(&e.id)).cloned().collect();
        let _ = store.delete_events(&changes.deleted);
        if let Err(e) = store.upsert_events(&upserts) {
            set_calendar_status(store, acct, &SyncState::SyncFailed(e.to_string()));
            return;
        }
        outcome.calendar_changes += changes.upserts.len() + changes.deleted.len();
        if let Some(t) = changes.next_sync_token {
            let _ = store.set_setting(&key, &t);
        }
    }
    let _ = store.set_setting(&coverage_key, &CALENDAR_COVERAGE);
    set_calendar_status(store, acct, &SyncState::UpToDate);
}

pub const RATE_LIMIT_WORDING: &str = "Waiting for Gmail: it is limiting requests. Downloading continues in a moment.";

/// Messages downloaded per batch before progress is committed to the store.
const DOWNLOAD_BATCH: usize = 20;

/// Bump when the set of views the first download covers changes; accounts downloaded with
/// an older version list the new views and queue what is missing, keeping their checkpoint.
const COVERAGE_VERSION: u32 = 4;

fn coverage_key(account: &AccountId) -> String {
    format!("download_coverage:{}", account.0)
}

/// Adds snapshot ids to the backlog (de-duplicated). With `only_missing`, ids already on
/// disk are skipped; otherwise they are re-downloaded (used when the mapping gained fields).
fn enqueue(store: &Store, account: &AccountId, ids: Vec<ProviderId>, only_missing: bool) {
    let mut backlog = backlog(store, account);
    for pid in ids {
        let present = store.message_id_for_provider_id(account, &pid).ok().flatten().is_some();
        if (!only_missing || !present) && !backlog.contains(&pid) {
            backlog.push(pid);
        }
    }
    save_backlog(store, account, &backlog);
}

/// Queues bodies for messages a provider reported header-only (`RemoteChanges::message_ids`).
/// Unlike [`enqueue`], a row that exists without a cached body is still queued.
fn enqueue_bodies(store: &Store, account: &AccountId, ids: Vec<ProviderId>) {
    let mut backlog = backlog(store, account);
    for pid in ids {
        let cached = store
            .message_id_for_provider_id(account, &pid)
            .ok()
            .flatten()
            .and_then(|id| store.message(&id).ok().flatten())
            .is_some_and(|m| m.body.is_cached());
        if !cached && !backlog.contains(&pid) {
            backlog.push(pid);
        }
    }
    save_backlog(store, account, &backlog);
}

/// Lines downloaded messages up with what is already stored, for providers whose local
/// ids are minted on first sight and whose conversations are not supplied (IMAP, POP3).
/// Gmail ids and thread ids are deterministic, so every step is a no-op for Gmail.
///
/// 1. A message whose provider id is already stored under another local id adopts that id
///    (a body download completing a header-only row).
/// 2. A new message whose Message-ID belongs to a message vanishing in the same cycle is
///    that message after a move made elsewhere: it keeps the local id and leaves `deleted`.
/// 3. A message without a conversation keeps the conversation its row already has;
///    the rest are threaded by `In-Reply-To` / `References` against the store.
fn reconcile_incoming(store: &Store, account: &AccountId, messages: &mut [Message], deleted: &mut Vec<MessageId>) {
    let vanished: Vec<Message> = if deleted.is_empty() || messages.iter().all(|m| m.rfc_message_id.is_none()) {
        Vec::new()
    } else {
        deleted.iter().filter_map(|id| store.message(id).ok().flatten()).collect()
    };
    for m in messages.iter_mut() {
        let mut existing = store.message(&m.id).ok().flatten();
        if existing.is_none() {
            if let Some(pid) = &m.provider_id {
                if let Ok(Some(local)) = store.message_id_for_provider_id(account, pid) {
                    if local != m.id {
                        rename_message(m, local.clone());
                        existing = store.message(&local).ok().flatten();
                    }
                }
            }
        }
        if existing.is_none() {
            if let Some(rfc) = &m.rfc_message_id {
                if let Some(moved) = vanished.iter().find(|v| v.rfc_message_id.as_deref() == Some(rfc.as_str()) && v.provider_id != m.provider_id) {
                    info!("message {} moved at the provider; keeping local id {}", rfc, moved.id);
                    deleted.retain(|d| d != &moved.id);
                    rename_message(m, moved.id.clone());
                    existing = Some(moved.clone());
                }
            }
        }
        if m.conversation_id.0.is_empty() {
            if let Some(local) = &existing {
                m.conversation_id = local.conversation_id.clone();
            }
        }
    }
    if messages.iter().any(|m| m.conversation_id.0.is_empty()) {
        assign_conversations(messages, |rfc_id| store.conversation_for_rfc_id(account, rfc_id).ok().flatten());
    }
}

fn rename_message(m: &mut Message, id: MessageId) {
    for (n, a) in m.attachments.iter_mut().enumerate() {
        a.message_id = id.clone();
        a.id = AttachmentId(format!("{}:att-{n}", id.0));
    }
    m.id = id;
}

fn state_for(err: &ProviderError) -> SyncState {
    match err {
        ProviderError::Offline | ProviderError::Uncertain => SyncState::Offline,
        ProviderError::SignInRequired => SyncState::SignInRequired,
        ProviderError::RateLimited(_) => SyncState::Paused(RATE_LIMIT_WORDING.into()),
        ProviderError::Temporary(m) | ProviderError::Rejected(m) => SyncState::SyncFailed(m.clone()),
        ProviderError::CheckpointExpired => SyncState::SyncFailed("sync checkpoint expired".into()),
    }
}

fn backlog_key(account: &AccountId) -> String {
    format!("download_backlog:{}", account.0)
}

pub fn backlog(store: &Store, account: &AccountId) -> Vec<ProviderId> {
    store.setting::<Vec<String>>(&backlog_key(account)).ok().flatten().unwrap_or_default().into_iter().map(ProviderId).collect()
}

fn save_backlog(store: &Store, account: &AccountId, ids: &[ProviderId]) {
    let raw: Vec<&str> = ids.iter().map(|p| p.0.as_str()).collect();
    let _ = store.set_setting(&backlog_key(account), &raw);
}

/// Downloads queued message bodies in small batches, committing after each batch so an
/// interruption (quota, offline, crash) keeps what was fetched.
fn drain_backlog(store: &Store, provider: &dyn MailProvider, account: &Account, outcome: &mut SyncOutcome) -> Result<(), ProviderError> {
    let acct = &account.id;
    let mut ids = backlog(store, acct);
    if ids.is_empty() {
        return Ok(());
    }
    let labels = store.labels(acct).unwrap_or_default();
    while !ids.is_empty() {
        let protected = store.pending_message_ids(acct).unwrap_or_default();
        let take = ids.len().min(DOWNLOAD_BATCH);
        let batch: Vec<ProviderId> = ids[..take].to_vec();
        let mut messages = match pollster_block(provider.fetch_messages(account, &batch, &labels)) {
            Ok(m) => m,
            Err(e) => {
                save_backlog(store, acct, &ids);
                return Err(e);
            }
        };
        reconcile_incoming(store, acct, &mut messages, &mut Vec::new());
        store.merge_remote_messages(&messages, &protected).map_err(|e| ProviderError::Temporary(e.to_string()))?;
        outcome.downloaded += messages.len();
        ids.drain(..take);
        save_backlog(store, acct, &ids);
    }
    Ok(())
}

/// Downloads remote state. Returns the number of messages written.
fn download(store: &Store, provider: &dyn MailProvider, account: &Account, outcome: &mut SyncOutcome) -> Result<usize, ProviderError> {
    let acct = &account.id;
    let protected = store.pending_message_ids(acct).unwrap_or_default();
    let checkpoint = store.checkpoint(acct).ok().flatten();
    let have_messages = store.message_count(acct).unwrap_or(0) > 0 || !backlog(store, acct).is_empty();

    let full = |outcome: &mut SyncOutcome| -> Result<usize, ProviderError> {
        let mut snapshot = pollster_block(provider.fetch_snapshot(account))?;
        store.replace_labels(acct, &snapshot.labels).map_err(|e| ProviderError::Temporary(e.to_string()))?;
        reconcile_incoming(store, acct, &mut snapshot.messages, &mut Vec::new());
        store.merge_remote_messages(&snapshot.messages, &protected).map_err(|e| ProviderError::Temporary(e.to_string()))?;
        store.set_checkpoint(acct, snapshot.checkpoint.as_deref()).map_err(|e| ProviderError::Temporary(e.to_string()))?;
        outcome.downloaded += snapshot.messages.len();
        // Queue bodies that are not on disk yet; the backlog survives restarts.
        enqueue(store, acct, snapshot.message_ids, true);
        let _ = store.set_setting(&coverage_key(acct), &COVERAGE_VERSION);
        drain_backlog(store, provider, account, outcome)?;
        Ok(outcome.downloaded)
    };

    // An account downloaded before the current view coverage lists the new views and queues
    // what is missing. The existing checkpoint stays valid, so nothing is skipped.
    let extend_coverage = |outcome: &mut SyncOutcome| -> Result<(), ProviderError> {
        let covered = store.setting::<u32>(&coverage_key(acct)).ok().flatten().unwrap_or(1);
        if covered >= COVERAGE_VERSION {
            return Ok(());
        }
        let mut snapshot = pollster_block(provider.fetch_snapshot(account))?;
        store.replace_labels(acct, &snapshot.labels).map_err(|e| ProviderError::Temporary(e.to_string()))?;
        reconcile_incoming(store, acct, &mut snapshot.messages, &mut Vec::new());
        store.merge_remote_messages(&snapshot.messages, &protected).map_err(|e| ProviderError::Temporary(e.to_string()))?;
        // Version 3 added category/importance hints to the mapping: refresh what is stored.
        enqueue(store, acct, snapshot.message_ids, covered >= 4);
        let _ = store.set_setting(&coverage_key(acct), &COVERAGE_VERSION);
        info!("extended download coverage for {}", account.email);
        drain_backlog(store, provider, account, outcome)
    };

    match checkpoint {
        Some(cp) if have_messages => match pollster_block(provider.fetch_changes(account, &cp)) {
            Ok(mut changes) => {
                if let Some(labels) = &changes.labels {
                    store.replace_labels(acct, labels).map_err(|e| ProviderError::Temporary(e.to_string()))?;
                }
                // A message that vanished from one folder and appeared in another is a move,
                // not a deletion plus a new message.
                reconcile_incoming(store, acct, &mut changes.upserts, &mut changes.deleted);
                store.merge_remote_messages(&changes.upserts, &protected).map_err(|e| ProviderError::Temporary(e.to_string()))?;
                if !changes.deleted.is_empty() {
                    // Respect remote deletions even if a local change was pending on them.
                    let _ = store.delete_messages(&changes.deleted);
                }
                if let Some(new_cp) = changes.checkpoint.as_deref() {
                    store.set_checkpoint(acct, Some(new_cp)).map_err(|e| ProviderError::Temporary(e.to_string()))?;
                }
                outcome.downloaded += changes.upserts.len();
                if !changes.message_ids.is_empty() {
                    enqueue_bodies(store, acct, std::mem::take(&mut changes.message_ids));
                }
                drain_backlog(store, provider, account, outcome)?;
                extend_coverage(outcome)?;
                // One-time: re-fetch messages whose calendar part was stored before
                // invitation parsing existed.
                let invite_key = format!("invites_parsed:{}", acct.0);
                if store.setting::<bool>(&invite_key).ok().flatten() != Some(true) {
                    let ids = store.messages_needing_invitation_parse(acct).unwrap_or_default();
                    if !ids.is_empty() {
                        enqueue(store, acct, ids, false);
                        drain_backlog(store, provider, account, outcome)?;
                    }
                    let _ = store.set_setting(&invite_key, &true);
                }
                // One-time: re-fetch messages with cid: images stored before Content-ID was kept.
                let inline_key = format!("inline_cids_parsed:{}", acct.0);
                if store.setting::<bool>(&inline_key).ok().flatten() != Some(true) {
                    let ids = store.messages_needing_inline_parse(acct).unwrap_or_default();
                    if !ids.is_empty() {
                        enqueue(store, acct, ids, false);
                        drain_backlog(store, provider, account, outcome)?;
                    }
                    let _ = store.set_setting(&inline_key, &true);
                }
                Ok(outcome.downloaded)
            }
            Err(ProviderError::CheckpointExpired) => {
                info!("checkpoint expired for {}; full resynchronization", account.email);
                full(outcome)
            }
            Err(e) => Err(e),
        },
        _ => full(outcome),
    }
}

pub async fn run_sync(store: Arc<Store>, provider: Arc<dyn MailProvider>, calendar: Option<Arc<dyn CalendarProvider>>, account: Account) -> SyncOutcome {
    let mut outcome = SyncOutcome::default();
    let acct = account.id.clone();

    if let Err(err) = provider.probe().await {
        let state = state_for(&err);
        let _ = store.set_sync_state(&acct, &state, None);
        outcome.state = Some(state);
        outcome.stopped_offline = true;
        return outcome;
    }
    let _ = store.set_sync_state(&acct, &SyncState::Syncing, None);

    if let Err(err) = download(&store, provider.as_ref(), &account, &mut outcome) {
        warn!("download paused for {}: {err}", account.email);
        outcome.rate_limited = matches!(err, ProviderError::RateLimited(_));
        let state = state_for(&err);
        let _ = store.set_sync_state(&acct, &state, None);
        outcome.state = Some(state);
        outcome.stopped_offline = matches!(err, ProviderError::Offline);
        return outcome;
    }

    if let Some(cal) = &calendar {
        sync_calendar(&store, cal.as_ref(), &account, &mut outcome);
    }
    sync_contacts(&store, provider.as_ref(), &account).await;

    let labels = store.labels(&acct).unwrap_or_default();
    let sent_label = labels.iter().find(|l| l.kind == LabelKind::Sent).map(|l| l.id.clone());
    let calendars = store.calendars(&acct).unwrap_or_default();
    let ops = match store.pending_operations(&acct) {
        Ok(ops) => ops,
        Err(e) => {
            let state = SyncState::SyncFailed(e.to_string());
            let _ = store.set_sync_state(&acct, &state, None);
            outcome.state = Some(state);
            return outcome;
        }
    };

    let mut final_state = SyncState::UpToDate;
    for op in ops {
        if op.state == OperationState::Applied {
            continue;
        }
        let _ = store.set_operation_state(&op.id, OperationState::InFlight, None);
        match &op.kind {
            OperationKind::SendDraft { draft_id } => {
                let Some(draft) = store.draft(draft_id).ok().flatten() else {
                    let _ = store.set_operation_state(&op.id, OperationState::Applied, None);
                    continue;
                };
                let subject = draft.display_subject();
                let _ = store.set_draft_state(&draft.id, &DraftState::Sending);
                let context = SendContext {
                    sent_label: sent_label.clone(),
                    in_reply_to: draft.in_reply_to.as_ref().and_then(|id| store.message(id).ok().flatten()),
                    labels: labels.clone(),
                };
                let needs_reconcile = op.state == OperationState::Uncertain || draft.state == DraftState::Uncertain;
                let send_result = if needs_reconcile {
                    match provider.reconcile_send(&account, &draft).await {
                        Ok(SendOutcome::Delivered(provider_id)) => {
                            let rfc = overlook_provider::draft_message_id(&draft);
                            Ok(overlook_provider::SentReceipt {
                                message: overlook_provider::sample::sent_message(&account, &draft, context.in_reply_to.as_ref(), &provider_id, &rfc, sent_label.as_ref(), now()),
                                provider_id,
                                rfc_message_id: Some(rfc),
                            })
                        }
                        Ok(SendOutcome::NotDelivered) => provider.send(&account, &draft, context).await,
                        Err(e) => Err(e),
                    }
                } else {
                    provider.send(&account, &draft, context).await
                };
                match send_result {
                    Ok(receipt) => {
                        if let Err(e) = store.complete_send(&draft.id, &op.id, &receipt.message) {
                            warn!("complete_send failed: {e}");
                            let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&e.to_string()));
                            let _ = store.set_draft_state(&draft.id, &DraftState::Failed(e.to_string()));
                            outcome.failed.push(subject);
                        } else {
                            info!("sent {subject}");
                            outcome.sent.push(subject);
                            outcome.applied += 1;
                        }
                    }
                    Err(ProviderError::Uncertain) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Uncertain, Some("connection dropped during sending"));
                        let _ = store.set_draft_state(&draft.id, &DraftState::Uncertain);
                        outcome.uncertain.push(subject);
                        final_state = SyncState::Offline;
                        break;
                    }
                    Err(ProviderError::Rejected(reason)) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&reason));
                        let _ = store.set_draft_state(&draft.id, &DraftState::Failed(reason));
                        outcome.failed.push(subject);
                    }
                    Err(err) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, Some(&err.to_string()));
                        let _ = store.set_draft_state(&draft.id, &DraftState::Queued);
                        final_state = state_for(&err);
                        outcome.stopped_offline = matches!(err, ProviderError::Offline);
                        outcome.rate_limited = matches!(err, ProviderError::RateLimited(_));
                        break;
                    }
                }
            }
            kind if kind.is_calendar() => {
                let Some(cal) = calendar.as_ref().filter(|c| c.has_calendar_access()) else {
                    let _ = store.set_operation_state(&op.id, OperationState::Pending, Some("calendar access needed"));
                    set_calendar_status(&store, &acct, &SyncState::SignInRequired);
                    continue;
                };
                let event = kind.event_id().and_then(|id| store.event(id).ok().flatten());
                let find_calendar = |id: &CalendarId| calendars.iter().find(|c| &c.id == id).cloned();
                let result: Result<(), ProviderError> = match kind {
                    OperationKind::RespondToEvent { response, .. } => match event.as_ref().and_then(|e| find_calendar(&e.calendar_id).map(|c| (e, c))) {
                        Some((e, c)) => cal.respond(&account, &c, e, *response).await,
                        None => Ok(()),
                    },
                    OperationKind::CreateEvent { .. } => match event.as_ref().and_then(|e| find_calendar(&e.calendar_id).map(|c| (e, c))) {
                        Some((e, c)) => cal.create_event(&account, &c, e).await.and_then(|created| store.upsert_events(&[created]).map_err(|e| ProviderError::Temporary(e.to_string()))),
                        None => Ok(()),
                    },
                    OperationKind::UpdateEvent { scope, .. } => match event.as_ref().and_then(|e| find_calendar(&e.calendar_id).map(|c| (e, c))) {
                        Some((e, c)) => cal.update_event_scoped(&account, &c, e, *scope).await.and_then(|updated| store.upsert_events(&[updated]).map_err(|e| ProviderError::Temporary(e.to_string()))),
                        None => Ok(()),
                    },
                    OperationKind::DeleteEvent { calendar_id, provider_id, scope, recurring_event_id, original_start, .. } => match (find_calendar(calendar_id), provider_id) {
                        (Some(c), Some(pid)) => cal.delete_event_scoped(&account, &c, pid, *scope, recurring_event_id.as_deref(), *original_start).await,
                        _ => Ok(()),
                    },
                    _ => Ok(()),
                };
                match result {
                    Ok(()) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Applied, None);
                        outcome.applied += 1;
                    }
                    Err(ProviderError::Rejected(reason)) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&reason));
                        outcome.failed.push(kind.describe());
                    }
                    Err(err) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, Some(&err.to_string()));
                        final_state = state_for(&err);
                        outcome.stopped_offline = matches!(err, ProviderError::Offline);
                        outcome.rate_limited = matches!(err, ProviderError::RateLimited(_));
                        break;
                    }
                }
            }
            kind if kind.is_contact() => {
                let contact = kind.contact_id().and_then(|id| store.contact_by_id(&acct, id).ok().flatten());
                let result: Result<(), ProviderError> = match kind {
                    OperationKind::CreateContact { contact_id } => match &contact {
                        Some(c) => provider.create_contact(&account, c).await.and_then(|created| store.mark_contact_synced(&acct, contact_id, &created).map_err(|e| ProviderError::Temporary(e.to_string()))),
                        None => Ok(()),
                    },
                    OperationKind::UpdateContact { contact_id } => match &contact {
                        Some(c) => provider.update_contact(&account, c).await.and_then(|updated| store.mark_contact_synced(&acct, contact_id, &updated).map_err(|e| ProviderError::Temporary(e.to_string()))),
                        None => Ok(()),
                    },
                    OperationKind::DeleteContact { resource_name, .. } => match resource_name {
                        Some(r) => provider.delete_contact(&account, r).await,
                        None => Ok(()),
                    },
                    _ => Ok(()),
                };
                match result {
                    Ok(()) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Applied, None);
                        outcome.applied += 1;
                    }
                    Err(ProviderError::Rejected(reason)) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&reason));
                        outcome.failed.push(kind.describe());
                    }
                    Err(ProviderError::SignInRequired) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, Some("sign in again to change contacts"));
                    }
                    Err(err) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, Some(&err.to_string()));
                        final_state = state_for(&err);
                        outcome.stopped_offline = matches!(err, ProviderError::Offline);
                        outcome.rate_limited = matches!(err, ProviderError::RateLimited(_));
                        break;
                    }
                }
            }
            kind => {
                let messages: Vec<Message> = kind.message_ids().iter().filter_map(|id| store.message(id).ok().flatten()).collect();
                match provider.apply(&account, &op, &messages, &labels).await {
                    Ok(applied) => {
                        if let Err(e) = store.apply_provider_id_changes(&applied.provider_ids) {
                            warn!("could not record provider id changes: {e}");
                        }
                        let _ = store.set_operation_state(&op.id, OperationState::Applied, None);
                        outcome.applied += 1;
                    }
                    Err(ProviderError::Rejected(reason)) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&reason));
                        outcome.failed.push(kind.describe());
                        final_state = SyncState::SyncFailed(reason);
                        break;
                    }
                    Err(err) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, Some(&err.to_string()));
                        final_state = state_for(&err);
                        outcome.stopped_offline = matches!(err, ProviderError::Offline);
                        outcome.rate_limited = matches!(err, ProviderError::RateLimited(_));
                        break;
                    }
                }
            }
        }
    }

    let success_at = if final_state == SyncState::UpToDate { Some(now()) } else { None };
    let _ = store.set_sync_state(&acct, &final_state, success_at);
    let _ = store.prune_applied(now() - 7 * 86_400_000);
    outcome.state = Some(final_state);
    outcome
}

/// Minimal executor for provider futures inside the already-blocking download step.
/// Provider futures do blocking I/O and never return `Pending`.
fn pollster_block<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut f = Box::pin(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use overlook_provider::sample::{sample_mailbox, SampleProvider};
    use std::time::Duration;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        pollster_block(f)
    }

    fn seeded() -> (Arc<Store>, Account, SampleProvider) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let account = Account {
            id: AccountId::from("sample"),
            provider: ProviderKind::Sample,
            display_name: "Jason Alexander".into(),
            email: "jason@example.com".into(),
            created_at: 0,
        };
        store.upsert_account(&account).unwrap();
        let snap = sample_mailbox(&account, now());
        for l in &snap.labels {
            store.upsert_label(l).unwrap();
        }
        store.upsert_messages(&snap.messages).unwrap();
        store.set_checkpoint(&account.id, snap.checkpoint.as_deref()).unwrap();
        (store, account, SampleProvider::new().with_latency(Duration::ZERO))
    }

    fn queued_draft(store: &Store, account: &Account) -> Draft {
        let mut draft = Draft::new(account.id.clone(), DraftKind::New, now());
        draft.to = vec![Address::named("Maya Chen", "maya@northstar.example")];
        draft.subject = "Agenda".into();
        draft.body_text = "A few things to cover.".into();
        store.save_draft(&draft).unwrap();
        store.queue_send(&draft).unwrap();
        draft
    }

    fn arc(p: &SampleProvider) -> Arc<dyn MailProvider> {
        Arc::new(p.clone())
    }

    #[test]
    fn offline_sync_leaves_everything_queued() {
        let (store, account, provider) = seeded();
        let first = store.messages_for_account(&account.id).unwrap()[0].id.clone();
        store.apply_change(&account.id, overlook_store::LocalChange::SetStarred { message_ids: vec![first], starred: true }).unwrap();
        queued_draft(&store, &account);
        let outcome = block_on(run_sync(store.clone(), arc(&provider), None, account.clone()));
        assert!(outcome.stopped_offline);
        assert_eq!(store.pending_count(&account.id).unwrap(), 2);
        assert_eq!(store.account_status(&account.id).unwrap().state, SyncState::Offline);
        assert_eq!(store.drafts(&account.id, true).unwrap()[0].state, DraftState::Queued);
    }

    #[test]
    fn first_sync_downloads_snapshot_when_store_is_empty() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let account = Account {
            id: AccountId::from("sample"),
            provider: ProviderKind::Sample,
            display_name: "Jason".into(),
            email: "jason@example.com".into(),
            created_at: 0,
        };
        store.upsert_account(&account).unwrap();
        let provider = SampleProvider::new().with_latency(Duration::ZERO);
        provider.set_online(true);
        let outcome = block_on(run_sync(store.clone(), arc(&provider), None, account.clone()));
        assert!(outcome.downloaded > 0);
        assert!(store.message_count(&account.id).unwrap() > 0);
        assert_eq!(store.checkpoint(&account.id).unwrap().as_deref(), Some("sample-1"));
        assert_eq!(store.account_status(&account.id).unwrap().state, SyncState::UpToDate);
    }

    #[test]
    fn replay_is_ordered_and_send_needs_provider_acceptance() {
        let (store, account, provider) = seeded();
        let first = store.messages_for_account(&account.id).unwrap()[0].id.clone();
        store.apply_change(&account.id, overlook_store::LocalChange::SetStarred { message_ids: vec![first.clone()], starred: true }).unwrap();
        let draft = queued_draft(&store, &account);
        store.apply_change(&account.id, overlook_store::LocalChange::Archive { message_ids: vec![first] }).unwrap();
        provider.set_online(true);
        let outcome = block_on(run_sync(store.clone(), arc(&provider), None, account.clone()));
        assert_eq!(outcome.applied, 3);
        assert_eq!(outcome.sent, vec!["Agenda".to_owned()]);
        assert_eq!(store.pending_count(&account.id).unwrap(), 0);
        assert!(store.draft(&draft.id).unwrap().is_none(), "draft row removed after acceptance");
        let sent = store.conversations(&account.id, &account.email, &overlook_store::MailboxView::Kind(LabelKind::Sent), overlook_store::ListFilter::All, None).unwrap();
        assert!(sent.iter().any(|c| c.subject == "Agenda"));
        assert_eq!(store.account_status(&account.id).unwrap().state, SyncState::UpToDate);
        assert_eq!(provider.applied_operations().len(), 2, "two idempotent operations reached the provider in order");
    }

    #[test]
    fn dropped_connection_during_send_is_uncertain_then_reconciled_without_duplicates() {
        let (store, account, provider) = seeded();
        let draft = queued_draft(&store, &account);
        provider.set_online(true);
        provider.drop_next_send();
        let outcome = block_on(run_sync(store.clone(), arc(&provider), None, account.clone()));
        assert_eq!(outcome.uncertain, vec!["Agenda".to_owned()]);
        assert_eq!(store.draft(&draft.id).unwrap().unwrap().state, DraftState::Uncertain);
        assert_eq!(store.pending_operations(&account.id).unwrap()[0].state, OperationState::Uncertain);

        let outcome = block_on(run_sync(store.clone(), arc(&provider), None, account.clone()));
        assert_eq!(outcome.sent, vec!["Agenda".to_owned()]);
        assert!(store.draft(&draft.id).unwrap().is_none());
        let sent: Vec<_> = store
            .messages_for_account(&account.id)
            .unwrap()
            .into_iter()
            .filter(|m| m.is_outgoing && m.subject == "Agenda")
            .collect();
        assert_eq!(sent.len(), 1, "exactly one Sent copy");
        assert_eq!(store.account_status(&account.id).unwrap().state, SyncState::UpToDate);
    }

    #[test]
    fn cancelled_send_is_not_sent() {
        let (store, account, provider) = seeded();
        let draft = queued_draft(&store, &account);
        store.cancel_send(&draft.id).unwrap();
        provider.set_online(true);
        let outcome = block_on(run_sync(store.clone(), arc(&provider), None, account.clone()));
        assert!(outcome.sent.is_empty());
        assert_eq!(store.drafts(&account.id, false).unwrap().len(), 1);
    }

    // ---------------------------------------------------------------- scripted provider

    use overlook_provider::{AppliedChanges, BoxFuture, MailboxSnapshot, RemoteChanges, SentReceipt};
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Mutex;

    /// A provider shaped like IMAP: local ids minted on first sight, no thread ids, header-only
    /// changes with bodies downloaded later, and provider ids that change on move.
    #[derive(Default)]
    struct Scripted {
        snapshot: Mutex<MailboxSnapshot>,
        changes: Mutex<Option<RemoteChanges>>,
        bodies: Mutex<HashMap<String, Message>>,
        applied: Mutex<AppliedChanges>,
        fetched: Mutex<Vec<String>>,
    }

    impl MailProvider for Scripted {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Imap
        }
        fn probe(&self) -> BoxFuture<'_, Result<(), ProviderError>> {
            Box::pin(async { Ok(()) })
        }
        fn fetch_snapshot(&self, _account: &Account) -> BoxFuture<'_, Result<MailboxSnapshot, ProviderError>> {
            let snap = self.snapshot.lock().unwrap().clone();
            Box::pin(async move { Ok(snap) })
        }
        fn fetch_changes(&self, _account: &Account, checkpoint: &str) -> BoxFuture<'_, Result<RemoteChanges, ProviderError>> {
            let changes = self.changes.lock().unwrap().take().unwrap_or_else(|| RemoteChanges { checkpoint: Some(checkpoint.to_owned()), ..Default::default() });
            Box::pin(async move { Ok(changes) })
        }
        fn fetch_messages(&self, _account: &Account, ids: &[ProviderId], _labels: &[Label]) -> BoxFuture<'_, Result<Vec<Message>, ProviderError>> {
            let bodies = self.bodies.lock().unwrap();
            self.fetched.lock().unwrap().extend(ids.iter().map(|p| p.0.clone()));
            let found: Vec<Message> = ids.iter().filter_map(|p| bodies.get(&p.0).cloned()).collect();
            Box::pin(async move { Ok(found) })
        }
        fn apply(&self, _account: &Account, _op: &PendingOperation, _messages: &[Message], _labels: &[Label]) -> BoxFuture<'_, Result<AppliedChanges, ProviderError>> {
            let applied = std::mem::take(&mut *self.applied.lock().unwrap());
            Box::pin(async move { Ok(applied) })
        }
        fn send(&self, _account: &Account, _draft: &Draft, _context: SendContext) -> BoxFuture<'_, Result<SentReceipt, ProviderError>> {
            Box::pin(async { Err(ProviderError::Rejected("not in this test".into())) })
        }
        fn reconcile_send(&self, _account: &Account, _draft: &Draft) -> BoxFuture<'_, Result<SendOutcome, ProviderError>> {
            Box::pin(async { Ok(SendOutcome::NotDelivered) })
        }
    }

    fn imap_account() -> Account {
        Account { id: AccountId::from("imap:j"), provider: ProviderKind::Imap, display_name: "Jason".into(), email: "jason@example.com".into(), created_at: 0 }
    }

    fn folder(kind: LabelKind, path: &str) -> Label {
        Label { id: LabelId(format!("imap:j:{path}")), account_id: AccountId::from("imap:j"), kind, name: path.into(), provider_id: Some(ProviderId(path.into())), color: None, sort_order: 0 }
    }

    fn raw_message(uid: &str, folder: &str, rfc: &str, in_reply_to: Option<&str>, references: &[&str], date: i64, cached: bool) -> Message {
        Message {
            id: MessageId(format!("imap:j:{}", MessageId::generate())),
            account_id: AccountId::from("imap:j"),
            conversation_id: ConversationId(String::new()),
            provider_id: Some(ProviderId(format!("{folder}\u{1f}1\u{1f}{uid}"))),
            from: Address::named("Maya Chen", "maya@northstar.example"),
            to: vec![Address::named("Jason", "jason@example.com")],
            cc: vec![],
            bcc: vec![],
            reply_to: None,
            subject: format!("Message {uid}"),
            snippet: String::new(),
            body: if cached { Body { text: Some(format!("body {uid}")), html: None } } else { Body::default() },
            date,
            is_read: false,
            is_starred: false,
            is_outgoing: false,
            label_ids: [LabelId(format!("imap:j:{folder}"))].into_iter().collect(),
            attachments: vec![],
            rfc_message_id: Some(rfc.into()),
            in_reply_to: in_reply_to.map(str::to_owned),
            references: references.iter().map(|r| r.to_string()).collect(),
            category: None,
            is_important: false,
            focus_override: None,
            invitation: None,
        }
    }

    fn imap_seeded() -> (Arc<Store>, Account, Arc<Scripted>) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let account = imap_account();
        store.upsert_account(&account).unwrap();
        let provider = Arc::new(Scripted::default());
        *provider.snapshot.lock().unwrap() = MailboxSnapshot {
            labels: vec![folder(LabelKind::Inbox, "INBOX"), folder(LabelKind::Archive, "Archive")],
            messages: vec![
                raw_message("1", "INBOX", "<a@x>", None, &[], 1_000, true),
                raw_message("2", "INBOX", "<b@x>", Some("<a@x>"), &["<a@x>"], 2_000, true),
                raw_message("3", "INBOX", "<z@x>", None, &["<p@x>"], 3_000, true),
            ],
            message_ids: vec![],
            checkpoint: Some("cursor-1".into()),
        };
        (store, account, provider)
    }

    fn by_rfc(store: &Store, rfc: &str) -> Message {
        store.message_by_rfc_id(&AccountId::from("imap:j"), rfc).unwrap().unwrap_or_else(|| panic!("no message {rfc}"))
    }

    #[test]
    fn threads_messages_for_providers_without_thread_ids() {
        let (store, account, provider) = imap_seeded();
        let outcome = block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));
        assert_eq!(outcome.state, Some(SyncState::UpToDate));
        let (a, b, z) = (by_rfc(&store, "<a@x>"), by_rfc(&store, "<b@x>"), by_rfc(&store, "<z@x>"));
        assert_eq!(a.conversation_id.as_str(), "imap:j:<a@x>");
        assert_eq!(b.conversation_id, a.conversation_id, "the reply joined its parent");
        assert_ne!(z.conversation_id, a.conversation_id);
        assert!(store.messages_for_account(&account.id).unwrap().iter().all(|m| !m.conversation_id.0.is_empty()));

        // Next cycle: a reply to b arrives, and the parent z referenced shows up late.
        *provider.changes.lock().unwrap() = Some(RemoteChanges {
            upserts: vec![raw_message("4", "INBOX", "<c@x>", Some("<b@x>"), &["<a@x>", "<b@x>"], 4_000, true), raw_message("5", "INBOX", "<p@x>", None, &[], 500, true)],
            checkpoint: Some("cursor-2".into()),
            ..Default::default()
        });
        block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));
        assert_eq!(by_rfc(&store, "<c@x>").conversation_id, a.conversation_id);
        assert_eq!(by_rfc(&store, "<p@x>").conversation_id, z.conversation_id, "a parent downloaded after its reply joins the reply's conversation");
        assert_eq!(store.messages_in_conversation(&account.id, &a.conversation_id).unwrap().len(), 3);
    }

    #[test]
    fn move_seen_elsewhere_keeps_the_local_message() {
        let (store, account, provider) = imap_seeded();
        block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));
        let before = by_rfc(&store, "<b@x>");
        assert!(!before.is_starred);

        // Another client starred and archived it: the UID vanished from INBOX and a new one
        // appeared in Archive. (With a local operation still pending on the message, the
        // store would keep the local flags and labels until it replays; that rule is tested
        // in the store crate and is unchanged here.)
        let mut reappeared = raw_message("9", "Archive", "<b@x>", Some("<a@x>"), &["<a@x>"], 2_000, false);
        reappeared.is_starred = true;
        *provider.changes.lock().unwrap() = Some(RemoteChanges { upserts: vec![reappeared], deleted: vec![before.id.clone()], checkpoint: Some("cursor-2".into()), ..Default::default() });
        block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));

        let all = store.messages_by_rfc_id(&account.id, "<b@x>").unwrap();
        assert_eq!(all.len(), 1, "no duplicate after the move");
        let after = &all[0];
        assert_eq!(after.id, before.id, "the local id survives the move");
        assert_eq!(after.provider_id.as_ref().unwrap().as_str(), "Archive\u{1f}1\u{1f}9");
        assert_eq!(after.label_ids.iter().map(|l| l.as_str()).collect::<Vec<_>>(), vec!["imap:j:Archive"]);
        assert_eq!(after.conversation_id, before.conversation_id);
        assert!(after.is_starred, "flags come from the server copy");
        assert_eq!(after.body.text.as_deref(), Some("body 2"), "a header-only reappearance keeps the cached body");
        assert_eq!(store.message_count(&account.id).unwrap(), 3);
    }

    #[test]
    fn header_only_changes_queue_bodies_that_complete_the_same_row() {
        let (store, account, provider) = imap_seeded();
        block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));
        let header = raw_message("7", "INBOX", "<h@x>", None, &[], 7_000, false);
        let mut full = raw_message("7", "INBOX", "<h@x>", None, &[], 7_000, true);
        full.id = MessageId::from("imap:j:some-other-uuid");
        provider.bodies.lock().unwrap().insert("INBOX\u{1f}1\u{1f}7".into(), full);
        *provider.changes.lock().unwrap() = Some(RemoteChanges { upserts: vec![header.clone()], message_ids: vec![ProviderId("INBOX\u{1f}1\u{1f}7".into())], checkpoint: Some("cursor-2".into()), ..Default::default() });
        let outcome = block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));
        assert_eq!(outcome.state, Some(SyncState::UpToDate));
        assert_eq!(provider.fetched.lock().unwrap().as_slice(), ["INBOX\u{1f}1\u{1f}7"]);
        let rows = store.messages_by_rfc_id(&account.id, "<h@x>").unwrap();
        assert_eq!(rows.len(), 1, "the body download updated the header row instead of adding one");
        assert_eq!(rows[0].id, header.id);
        assert_eq!(rows[0].body.text.as_deref(), Some("body 7"));
        assert!(!rows[0].conversation_id.0.is_empty());
        assert!(backlog(&store, &account.id).is_empty());
    }

    #[test]
    fn applied_changes_rewrite_provider_ids() {
        let (store, account, provider) = imap_seeded();
        block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));
        let m = by_rfc(&store, "<z@x>");
        store.apply_change(&account.id, overlook_store::LocalChange::Archive { message_ids: vec![m.id.clone()] }).unwrap();
        *provider.applied.lock().unwrap() = AppliedChanges { provider_ids: vec![(m.id.clone(), ProviderId("Archive\u{1f}1\u{1f}3".into()), LabelId::from("imap:j:Archive"))] };
        let outcome = block_on(run_sync(store.clone(), provider.clone(), None, account.clone()));
        assert_eq!(outcome.applied, 1);
        let moved = store.message(&m.id).unwrap().unwrap();
        assert_eq!(moved.provider_id.as_ref().unwrap().as_str(), "Archive\u{1f}1\u{1f}3");
        assert_eq!(moved.label_ids.iter().map(|l| l.as_str()).collect::<BTreeSet<_>>(), ["imap:j:Archive"].into_iter().collect());
        assert_eq!(store.pending_count(&account.id).unwrap(), 0);
    }
}
