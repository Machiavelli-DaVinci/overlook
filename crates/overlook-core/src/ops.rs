//! Pending operations: local intent waiting to be applied at the provider.
//!
//! Every mailbox change the user makes is applied to local state and recorded here in the
//! same transaction. The sync engine replays operations per account in `seq` order.

use crate::ids::*;
use crate::time::Timestamp;
use serde::{Deserialize, Serialize};

/// What the user asked for. Payloads reference local ids; the provider adapter maps them to
/// provider ids at replay time so that operations can be queued before a provider id exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "op")]
pub enum OperationKind {
    SetRead { message_ids: Vec<MessageId>, read: bool },
    SetStarred { message_ids: Vec<MessageId>, starred: bool },
    AddLabel { message_ids: Vec<MessageId>, label_id: LabelId },
    RemoveLabel { message_ids: Vec<MessageId>, label_id: LabelId },
    /// Remove from Inbox (Gmail: remove INBOX label).
    Archive { message_ids: Vec<MessageId> },
    /// Move to Trash.
    Trash { message_ids: Vec<MessageId> },
    /// Restore from Trash to Inbox.
    Untrash { message_ids: Vec<MessageId> },
    /// Submit a queued draft. The draft row holds the content.
    SendDraft { draft_id: DraftId },
    /// Answer an invitation.
    RespondToEvent { event_id: EventId, response: crate::calendar::ResponseStatus },
    /// Create an event that exists locally.
    CreateEvent { event_id: EventId },
    /// Push local edits of an event; `scope` widens it to the series.
    UpdateEvent {
        event_id: EventId,
        #[serde(default)]
        scope: crate::calendar::RecurrenceScope,
    },
    /// Delete an event (provider id kept so the row can already be gone locally). For a
    /// series the master id and the split point travel with the operation.
    DeleteEvent {
        event_id: EventId,
        calendar_id: CalendarId,
        provider_id: Option<ProviderId>,
        #[serde(default)]
        scope: crate::calendar::RecurrenceScope,
        #[serde(default)]
        recurring_event_id: Option<String>,
        #[serde(default)]
        original_start: Option<Timestamp>,
    },
    /// Push a contact created on this device.
    CreateContact { contact_id: String },
    /// Push local edits of a contact.
    UpdateContact { contact_id: String },
    /// Delete a contact at the provider.
    DeleteContact { contact_id: String, resource_name: Option<String> },
}

impl OperationKind {
    /// Short human description for status surfaces and logs.
    pub fn describe(&self) -> String {
        fn n(ids: &[MessageId]) -> String {
            if ids.len() == 1 { "1 message".into() } else { format!("{} messages", ids.len()) }
        }
        match self {
            OperationKind::SetRead { message_ids, read: true } => format!("Mark {} as read", n(message_ids)),
            OperationKind::SetRead { message_ids, read: false } => format!("Mark {} as unread", n(message_ids)),
            OperationKind::SetStarred { message_ids, starred: true } => format!("Star {}", n(message_ids)),
            OperationKind::SetStarred { message_ids, starred: false } => format!("Unstar {}", n(message_ids)),
            OperationKind::AddLabel { message_ids, .. } => format!("Label {}", n(message_ids)),
            OperationKind::RemoveLabel { message_ids, .. } => format!("Remove label from {}", n(message_ids)),
            OperationKind::Archive { message_ids } => format!("Archive {}", n(message_ids)),
            OperationKind::Trash { message_ids } => format!("Delete {}", n(message_ids)),
            OperationKind::Untrash { message_ids } => format!("Restore {}", n(message_ids)),
            OperationKind::SendDraft { .. } => "Send message".to_owned(),
            OperationKind::RespondToEvent { response, .. } => format!("Respond to invitation ({})", response.label()),
            OperationKind::CreateEvent { .. } => "Create event".to_owned(),
            OperationKind::UpdateEvent { .. } => "Update event".to_owned(),
            OperationKind::DeleteEvent { .. } => "Delete event".to_owned(),
            OperationKind::CreateContact { .. } => "Add contact".to_owned(),
            OperationKind::UpdateContact { .. } => "Update contact".to_owned(),
            OperationKind::DeleteContact { .. } => "Delete contact".to_owned(),
        }
    }

    pub fn is_contact(&self) -> bool {
        matches!(self, OperationKind::CreateContact { .. } | OperationKind::UpdateContact { .. } | OperationKind::DeleteContact { .. })
    }

    pub fn contact_id(&self) -> Option<&str> {
        match self {
            OperationKind::CreateContact { contact_id } | OperationKind::UpdateContact { contact_id } | OperationKind::DeleteContact { contact_id, .. } => Some(contact_id),
            _ => None,
        }
    }

    pub fn is_calendar(&self) -> bool {
        matches!(
            self,
            OperationKind::RespondToEvent { .. } | OperationKind::CreateEvent { .. } | OperationKind::UpdateEvent { .. } | OperationKind::DeleteEvent { .. }
        )
    }

    pub fn event_id(&self) -> Option<&EventId> {
        match self {
            OperationKind::RespondToEvent { event_id, .. } | OperationKind::CreateEvent { event_id } | OperationKind::UpdateEvent { event_id, .. } | OperationKind::DeleteEvent { event_id, .. } => Some(event_id),
            _ => None,
        }
    }

    /// Message ids this operation touches (empty for sends).
    pub fn message_ids(&self) -> &[MessageId] {
        match self {
            OperationKind::SetRead { message_ids, .. }
            | OperationKind::SetStarred { message_ids, .. }
            | OperationKind::AddLabel { message_ids, .. }
            | OperationKind::RemoveLabel { message_ids, .. }
            | OperationKind::Archive { message_ids }
            | OperationKind::Trash { message_ids }
            | OperationKind::Untrash { message_ids } => message_ids,
            OperationKind::SendDraft { .. }
            | OperationKind::RespondToEvent { .. }
            | OperationKind::CreateEvent { .. }
            | OperationKind::UpdateEvent { .. }
            | OperationKind::DeleteEvent { .. }
            | OperationKind::CreateContact { .. }
            | OperationKind::UpdateContact { .. }
            | OperationKind::DeleteContact { .. } => &[],
        }
    }

    /// Whether the operation moves messages between folders at the provider, so a message
    /// vanishing from its folder is expected rather than a deletion made elsewhere.
    pub fn is_move(&self) -> bool {
        matches!(self, OperationKind::AddLabel { .. } | OperationKind::RemoveLabel { .. } | OperationKind::Archive { .. } | OperationKind::Trash { .. } | OperationKind::Untrash { .. })
    }

    /// Whether replaying the operation twice is harmless at the provider. Sends are not.
    pub fn is_idempotent(&self) -> bool {
        !matches!(self, OperationKind::SendDraft { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationState {
    Pending,
    InFlight,
    /// A non-idempotent operation was interrupted; reconcile before retrying.
    Uncertain,
    Failed,
    Applied,
}

impl OperationState {
    pub fn as_key(self) -> &'static str {
        match self {
            OperationState::Pending => "pending",
            OperationState::InFlight => "in-flight",
            OperationState::Uncertain => "uncertain",
            OperationState::Failed => "failed",
            OperationState::Applied => "applied",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "pending" => OperationState::Pending,
            "in-flight" => OperationState::InFlight,
            "uncertain" => OperationState::Uncertain,
            "failed" => OperationState::Failed,
            "applied" => OperationState::Applied,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOperation {
    pub id: OperationId,
    pub account_id: AccountId,
    /// Monotonic per-account sequence; replay order.
    pub seq: i64,
    pub kind: OperationKind,
    pub state: OperationState,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
