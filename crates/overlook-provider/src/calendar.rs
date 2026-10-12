//! Calendar provider trait.

use crate::{BoxFuture, ProviderError};
use overlook_core::*;

#[derive(Debug, Clone, Default)]
pub struct EventChanges {
    pub upserts: Vec<Event>,
    pub deleted: Vec<EventId>,
    pub next_sync_token: Option<String>,
}

pub trait CalendarProvider: Send + Sync {
    /// Whether the stored credentials include calendar access.
    fn has_calendar_access(&self) -> bool;

    fn list_calendars(&self, account: &Account) -> BoxFuture<'_, Result<Vec<Calendar>, ProviderError>>;

    /// Events for a calendar. With a sync token only changes are returned; without one the
    /// provider returns the configured window. `CheckpointExpired` means: start over.
    fn fetch_events(&self, account: &Account, calendar: &Calendar, sync_token: Option<&str>) -> BoxFuture<'_, Result<EventChanges, ProviderError>>;

    fn respond(&self, account: &Account, calendar: &Calendar, event: &Event, response: ResponseStatus) -> BoxFuture<'_, Result<(), ProviderError>>;

    /// Creates the event; returns the provider's copy.
    fn create_event(&self, account: &Account, calendar: &Calendar, event: &Event) -> BoxFuture<'_, Result<Event, ProviderError>>;

    fn update_event(&self, account: &Account, calendar: &Calendar, event: &Event) -> BoxFuture<'_, Result<Event, ProviderError>>;

    fn delete_event(&self, account: &Account, calendar: &Calendar, provider_id: &ProviderId) -> BoxFuture<'_, Result<(), ProviderError>>;
}
