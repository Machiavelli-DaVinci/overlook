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
