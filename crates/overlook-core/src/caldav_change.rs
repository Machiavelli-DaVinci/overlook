//! Durable CalDAV resource changes. A claimed revision is immutable until reconciled.
use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalDavEdit {
    pub before: Option<Event>,
    pub after: Option<Event>,
    pub scope: RecurrenceScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CalDavChangeState {
    Pending,
    Checking,
    Conflict,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalDavRemote {
    pub etag: String,
    pub icalendar: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalDavChange {
    pub id: OperationId,
    pub connection_id: CalendarConnectionId,
    pub generation: u32,
    pub account_id: AccountId,
    pub calendar_id: CalendarId,
    pub href: String,
    pub revision: u32,
    pub state: CalDavChangeState,
    pub base: Option<CalDavRemote>,
    /// None is a resource tombstone. PUT bodies are frozen before the first request.
    pub desired: Option<String>,
    pub edits: Vec<CalDavEdit>,
    pub remote: Option<CalDavRemote>,
    pub note: Option<String>,
    pub timezone: String,
}

impl CalDavChange {
    pub fn title(&self) -> String {
        self.edits
            .last()
            .and_then(|e| e.after.as_ref().or(e.before.as_ref()))
            .map(Event::display_title)
            .unwrap_or_else(|| "Calendar event".into())
    }
}
