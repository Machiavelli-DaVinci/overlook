//! Provider adapters.
//!
//! A provider turns local intent (pending operations, queued drafts) into remote effects and
//! reports remote state back. The sync engine in the app drives providers; providers never
//! touch the store directly, which keeps "what we asked for" and "what the server has"
//! separate.

pub mod gmail;
pub mod sample;

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
    /// The connection dropped after the request was sent but before a response arrived.
    /// For non-idempotent operations (sending), the outcome must be reconciled before any
    /// retry.
    #[error("connection dropped while waiting for a response")]
    Uncertain,
}

/// Snapshot delivered by an initial or full synchronization.
#[derive(Debug, Clone, Default)]
pub struct MailboxSnapshot {
    pub labels: Vec<Label>,
    pub messages: Vec<Message>,
    pub checkpoint: Option<String>,
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

    /// Applies an idempotent mailbox operation.
    fn apply(&self, account: &Account, op: &PendingOperation, messages: &[Message]) -> BoxFuture<'_, Result<(), ProviderError>>;

    /// Submits a message. Only a returned receipt means "Sent".
    fn send(&self, account: &Account, draft: &Draft, sent_label: Option<&LabelId>) -> BoxFuture<'_, Result<SentReceipt, ProviderError>>;

    /// Determines whether an uncertain send was delivered, by Message-ID or draft token.
    fn reconcile_send(&self, account: &Account, draft: &Draft) -> BoxFuture<'_, Result<SendOutcome, ProviderError>>;
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
