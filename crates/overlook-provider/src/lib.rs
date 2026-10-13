//! Provider adapters.
//!
//! A provider turns local intent (pending operations, queued drafts) into remote effects and
//! reports remote state back. The sync engine in the app drives providers; providers never
//! touch the store directly, which keeps "what we asked for" and "what the server has"
//! separate.

pub mod calendar;
pub mod gmail;
pub mod sample;

pub use calendar::{CalendarProvider, EventChanges};

use overlook_core::*;
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// The provider cannot be reached. Local work stays queued.
    #[error("offline")]
    Offline,
    /// Credentials expired or were revoked. Local work is preserved; the user must sign in.
    #[error("sign-in required")]
    SignInRequired,
    /// Temporary failure; retry with backoff.
    #[error("temporary failure: {0}")]
    Temporary(String),
    /// The provider rejected the operation. It will not succeed on retry without a change.
    #[error("{0}")]
    Rejected(String),
    /// The provider asked us to slow down (quota or rate limit). Retry after a longer pause.
    #[error("rate limited: {0}")]
    RateLimited(String),
    /// The connection dropped after the request was sent but before a response arrived.
    /// For non-idempotent operations (sending), the outcome must be reconciled before any
    /// retry.
    #[error("connection dropped while waiting for a response")]
    Uncertain,
    /// The saved sync checkpoint is no longer valid; a full resynchronization is required.
    #[error("sync checkpoint expired")]
    CheckpointExpired,
}

/// Snapshot delivered by an initial or full synchronization.
///
/// Providers may return messages directly (cheap local sources) or only `message_ids`,
/// which the sync engine then downloads in small persisted batches with
/// [`MailProvider::fetch_messages`], so quota limits and interruptions never lose progress.
#[derive(Debug, Clone, Default)]
pub struct MailboxSnapshot {
    pub labels: Vec<Label>,
    pub messages: Vec<Message>,
    /// Provider ids of recent messages still to download, newest first.
    pub message_ids: Vec<ProviderId>,
    pub checkpoint: Option<String>,
}

/// Incremental changes since a checkpoint.
#[derive(Debug, Clone, Default)]
pub struct RemoteChanges {
    /// Complete label list when the provider reports label changes (replaces the local list).
    pub labels: Option<Vec<Label>>,
    /// New or changed messages. A message without a cached body keeps the local body.
    pub upserts: Vec<Message>,
    /// Messages removed at the provider (local ids).
    pub deleted: Vec<MessageId>,
    pub checkpoint: Option<String>,
}

/// Everything a provider needs to submit a draft besides the draft itself.
#[derive(Debug, Clone, Default)]
pub struct SendContext {
    pub sent_label: Option<LabelId>,
    /// The message being replied to or forwarded, for threading headers.
    pub in_reply_to: Option<Message>,
    /// The account's labels, for provider id lookups.
    pub labels: Vec<Label>,
}

/// What the provider recorded after accepting a send.
#[derive(Debug, Clone)]
pub struct SentReceipt {
    pub provider_id: ProviderId,
    pub rfc_message_id: Option<String>,
    /// The provider's copy of the message, ready to store in Sent.
    pub message: Message,
}

/// Result of reconciling an uncertain send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// The provider has the message; do not resend.
    Delivered(ProviderId),
    /// The provider never received it; safe to resend.
    NotDelivered,
}

pub trait MailProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// Checks actual provider reachability (not just network availability).
    fn probe(&self) -> BoxFuture<'_, Result<(), ProviderError>>;

    /// Downloads labels and recent messages for first use or full resynchronization.
    fn fetch_snapshot(&self, account: &Account) -> BoxFuture<'_, Result<MailboxSnapshot, ProviderError>>;

    /// Downloads changes since `checkpoint`.
    fn fetch_changes(&self, account: &Account, checkpoint: &str) -> BoxFuture<'_, Result<RemoteChanges, ProviderError>>;

    /// Downloads full messages for provider ids listed by a snapshot. Ids that no longer
    /// exist are skipped.
    fn fetch_messages(&self, _account: &Account, _ids: &[ProviderId], _labels: &[Label]) -> BoxFuture<'_, Result<Vec<Message>, ProviderError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Applies an idempotent mailbox operation.
    fn apply(&self, account: &Account, op: &PendingOperation, messages: &[Message], labels: &[Label]) -> BoxFuture<'_, Result<(), ProviderError>>;

    /// Downloads one attachment's bytes.
    /// Whether automatic replies can be read and changed at the provider.
    fn has_settings_access(&self) -> bool {
        false
    }

    fn fetch_vacation(&self, _account: &Account) -> BoxFuture<'_, Result<Option<VacationSettings>, ProviderError>> {
        Box::pin(async { Ok(None) })
    }

    fn set_vacation(&self, _account: &Account, _settings: &VacationSettings) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async { Err(ProviderError::Rejected("this account cannot store automatic replies".into())) })
    }

    /// Whether the stored credentials allow reading contacts.
    fn has_contacts_access(&self) -> bool {
        false
    }

    /// The account's contacts: address book entries and people corresponded with. `None`
    /// when the provider has no contacts, or the sign-in lacks the scope.
    fn fetch_contacts(&self, _account: &Account) -> BoxFuture<'_, Result<Option<Vec<Contact>>, ProviderError>> {
        Box::pin(async { Ok(None) })
    }

    /// The message as RFC 822 bytes, for "Save as .eml" and "View source". The default
    /// rebuilds it from the stored headers and body.
    fn fetch_raw(&self, _account: &Account, message: &Message) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let bytes = message_to_raw(message);
        Box::pin(async move { Ok(bytes) })
    }

    /// Searches the whole mailbox at the provider. Returns provider ids, newest first.
    /// The default has no server-side search.
    fn search(&self, _account: &Account, _query: &str, _limit: usize) -> BoxFuture<'_, Result<Vec<ProviderId>, ProviderError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn fetch_attachment(&self, _account: &Account, _message: &Message, _attachment: &Attachment) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        Box::pin(async { Err(ProviderError::Rejected("this account cannot download attachments".into())) })
    }

    /// Submits a message. Only a returned receipt means "Sent".
    fn send(&self, account: &Account, draft: &Draft, context: SendContext) -> BoxFuture<'_, Result<SentReceipt, ProviderError>>;

    /// Determines whether an uncertain send was delivered, by Message-ID.
    fn reconcile_send(&self, account: &Account, draft: &Draft) -> BoxFuture<'_, Result<SendOutcome, ProviderError>>;
}

/// Deterministic Message-ID for a draft, so an interrupted send can be reconciled.
pub fn draft_message_id(draft: &Draft) -> String {
    format!("<{}@overlook.lastrefuge.ai>", draft.id)
}

/// Backoff schedule for temporary failures: 2s, 5s, 15s, 60s, then every 5 minutes.
pub fn backoff_seconds(attempt: u32) -> u64 {
    match attempt {
        0 | 1 => 2,
        2 => 5,
        3 => 15,
        4 => 60,
        _ => 300,
    }
}

/// Rebuilds an RFC 822 message from what is stored locally. Used when the provider cannot
/// hand back the original bytes.
pub fn message_to_raw(message: &Message) -> Vec<u8> {
    use lettre::message::header::ContentType;
    use lettre::message::{Mailbox, MultiPart};
    let mailbox = |a: &Address| -> Option<Mailbox> { a.email.parse().ok().map(|e| Mailbox::new(a.name.clone(), e)) };
    let mut builder = lettre::Message::builder().subject(message.subject.clone()).date(std::time::UNIX_EPOCH + std::time::Duration::from_millis(message.date.max(0) as u64));
    if let Some(from) = mailbox(&message.from) {
        builder = builder.from(from);
    }
    for a in &message.to {
        if let Some(m) = mailbox(a) {
            builder = builder.to(m);
        }
    }
    for a in &message.cc {
        if let Some(m) = mailbox(a) {
            builder = builder.cc(m);
        }
    }
    if let Some(id) = &message.rfc_message_id {
        builder = builder.message_id(Some(id.clone()));
    }
    let text = message.body.text.clone().unwrap_or_else(|| message.snippet.clone());
    let built = match &message.body.html {
        Some(html) => builder.multipart(MultiPart::alternative_plain_html(text, html.clone())),
        None => builder.header(ContentType::TEXT_PLAIN).body(text),
    };
    built.map(|m| m.formatted()).unwrap_or_else(|_| format!("Subject: {}\n\n{}", message.subject, message.body.text.clone().unwrap_or_default()).into_bytes())
}
