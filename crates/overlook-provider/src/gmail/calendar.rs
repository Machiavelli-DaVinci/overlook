//! Google Calendar through the Calendar API, sharing the account's OAuth client.

use super::api::{ApiError, GmailClient};
use super::oauth::{url_encode, CALENDAR_SCOPE_MARKER};
use crate::{BoxFuture, CalendarProvider, EventChanges, ProviderError};
use overlook_core::time::Timestamp;
use overlook_core::tz;
use overlook_core::*;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

pub const CALENDAR_BASE: &str = "https://www.googleapis.com/calendar/v3";
/// Window for the first download: 60 days back, one year ahead.
pub const PAST_DAYS: i32 = 60;
pub const FUTURE_DAYS: i32 = 366;

pub struct GoogleCalendarProvider {
    client: Arc<GmailClient>,
}

impl GoogleCalendarProvider {
    pub fn new(client: Arc<GmailClient>) -> Self {
        Self { client }
    }

    fn events_url(calendar: &Calendar, suffix: &str) -> String {
        let cal_id = calendar.provider_id.as_ref().map(|p| p.0.as_str()).unwrap_or("primary");
        format!("{CALENDAR_BASE}/calendars/{}/events{suffix}", url_encode(cal_id))
    }
}

pub fn local_calendar_id(account: &AccountId, gcal_id: &str) -> CalendarId {
    CalendarId(format!("{}:cal:{gcal_id}", account.0))
}

pub fn local_event_id(calendar: &CalendarId, gevent_id: &str) -> EventId {
    EventId(format!("{}:{gevent_id}", calendar.0))
}

pub fn calendar_from_json(account: &AccountId, v: &Value, sort_order: i32) -> Option<Calendar> {
    let id = v.get("id")?.as_str()?;
    let role = v.get("accessRole").and_then(|r| r.as_str()).unwrap_or("reader");
    Some(Calendar {
        id: local_calendar_id(account, id),
        account_id: account.clone(),
        provider_id: Some(ProviderId(id.to_owned())),
        name: v.get("summaryOverride").or_else(|| v.get("summary")).and_then(|s| s.as_str()).unwrap_or(id).to_owned(),
        color: v.get("backgroundColor").and_then(|c| c.as_str()).map(str::to_owned),
        is_primary: v.get("primary").and_then(|p| p.as_bool()).unwrap_or(false),
        can_edit: matches!(role, "owner" | "writer"),
        visible: v.get("selected").and_then(|s| s.as_bool()).unwrap_or(true),
        sort_order,
    })
}

fn parse_when(v: Option<&Value>) -> Option<(Timestamp, bool)> {
    let v = v?;
    if let Some(dt) = v.get("dateTime").and_then(|d| d.as_str()) {
        return tz::parse_rfc3339(dt).map(|t| (t, false));
    }
    let date = v.get("date").and_then(|d| d.as_str())?;
    tz::parse_date(date).map(|d| (tz::date_start(d), true))
}

pub fn event_from_json(account: &Account, calendar: &Calendar, v: &Value) -> Option<Event> {
    let id = v.get("id")?.as_str()?;
    let status = match v.get("status").and_then(|s| s.as_str()).unwrap_or("confirmed") {
        "cancelled" => EventStatus::Cancelled,
        "tentative" => EventStatus::Tentative,
        _ => EventStatus::Confirmed,
    };
    let local_id = local_event_id(&calendar.id, id);
    if status == EventStatus::Cancelled {
        // Cancelled instances carry no times; the caller deletes them by id.
        return Some(Event {
            id: local_id,
            account_id: account.id.clone(),
            calendar_id: calendar.id.clone(),
            provider_id: Some(ProviderId(id.to_owned())),
            ical_uid: v.get("iCalUID").and_then(|s| s.as_str()).map(str::to_owned),
            summary: String::new(),
            description: None,
            location: None,
            start: 0,
            end: 0,
            all_day: false,
            status,
            organizer: None,
            attendees: Vec::new(),
            html_link: None,
            conference_url: None,
            recurring_event_id: None,
            updated: 0,
            event_type: None,
            pending: false,
        });
    }
    let (start, all_day) = parse_when(v.get("start"))?;
    let (end, _) = parse_when(v.get("end")).unwrap_or((start + 3_600_000, all_day));
    let attendees: Vec<Attendee> = v
        .get("attendees")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some(Attendee {
                        email: a.get("email")?.as_str()?.to_owned(),
                        name: a.get("displayName").and_then(|n| n.as_str()).map(str::to_owned),
                        response: match a.get("responseStatus").and_then(|r| r.as_str()).unwrap_or("needsAction") {
                            "accepted" => ResponseStatus::Accepted,
                            "tentative" => ResponseStatus::Tentative,
                            "declined" => ResponseStatus::Declined,
                            _ => ResponseStatus::NeedsAction,
                        },
                        is_self: a.get("self").and_then(|s| s.as_bool()).unwrap_or(false),
                        is_organizer: a.get("organizer").and_then(|s| s.as_bool()).unwrap_or(false),
                        optional: a.get("optional").and_then(|s| s.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let organizer = v.get("organizer").and_then(|o| {
        Some(Address { name: o.get("displayName").and_then(|n| n.as_str()).map(str::to_owned), email: o.get("email")?.as_str()?.to_owned() })
    });
    let conference_url = v
        .get("hangoutLink")
        .and_then(|h| h.as_str())
        .map(str::to_owned)
        .or_else(|| {
            v.get("conferenceData")
                .and_then(|c| c.get("entryPoints"))
                .and_then(|e| e.as_array())
                .and_then(|arr| arr.iter().find(|p| p.get("entryPointType").and_then(|t| t.as_str()) == Some("video")))
                .and_then(|p| p.get("uri"))
                .and_then(|u| u.as_str())
                .map(str::to_owned)
        });
    Some(Event {
        id: local_id,
        account_id: account.id.clone(),
        calendar_id: calendar.id.clone(),
        provider_id: Some(ProviderId(id.to_owned())),
        ical_uid: v.get("iCalUID").and_then(|s| s.as_str()).map(str::to_owned),
        summary: v.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_owned(),
        description: v.get("description").and_then(|s| s.as_str()).map(str::to_owned),
        location: v.get("location").and_then(|s| s.as_str()).map(str::to_owned),
        start,
        end,
        all_day,
        status,
        organizer,
        attendees,
        html_link: v.get("htmlLink").and_then(|s| s.as_str()).map(str::to_owned),
        conference_url,
        recurring_event_id: v.get("recurringEventId").and_then(|s| s.as_str()).map(str::to_owned),
        updated: v.get("updated").and_then(|s| s.as_str()).and_then(tz::parse_rfc3339).unwrap_or(0),
        event_type: v.get("eventType").and_then(|t| t.as_str()).map(|t| match t {
            "workingLocation" => "working-location".to_owned(),
            "outOfOffice" => "out-of-office".to_owned(),
            "focusTime" => "focus-time".to_owned(),
            other => other.to_owned(),
        }),
        pending: false,
    })
}

/// Event body for insert/patch.
pub fn event_to_json(event: &Event) -> Value {
    let when = |ts: Timestamp| -> Value {
        if event.all_day {
            json!({ "date": tz::local_date(ts).to_string() })
        } else {
            json!({ "dateTime": tz::to_rfc3339(ts) })
        }
    };
    let mut body = json!({
        "summary": event.summary,
        "start": when(event.start),
        "end": when(event.end),
    });
    if let Some(d) = &event.description {
        body["description"] = Value::String(d.clone());
    }
    if let Some(l) = &event.location {
        body["location"] = Value::String(l.clone());
    }
    if !event.attendees.is_empty() {
        body["attendees"] = Value::Array(event.attendees.iter().map(attendee_to_json).collect());
    }
    body
}

fn attendee_to_json(a: &Attendee) -> Value {
    let mut v = json!({ "email": a.email, "responseStatus": match a.response {
        ResponseStatus::NeedsAction => "needsAction",
        ResponseStatus::Accepted => "accepted",
        ResponseStatus::Tentative => "tentative",
        ResponseStatus::Declined => "declined",
    }});
    if let Some(n) = &a.name {
        v["displayName"] = Value::String(n.clone());
    }
    if a.optional {
        v["optional"] = Value::Bool(true);
    }
    v
}

impl CalendarProvider for GoogleCalendarProvider {
    fn has_calendar_access(&self) -> bool {
        self.client.has_scope(CALENDAR_SCOPE_MARKER)
    }

    fn list_calendars(&self, account: &Account) -> BoxFuture<'_, Result<Vec<Calendar>, ProviderError>> {
        let account = account.clone();
        Box::pin(async move {
            let v = self.client.get_url(&format!("{CALENDAR_BASE}/users/me/calendarList?maxResults=250"))?;
            let mut calendars: Vec<Calendar> = v
                .get("items")
                .and_then(|i| i.as_array())
                .map(|arr| arr.iter().enumerate().filter_map(|(i, c)| calendar_from_json(&account.id, c, i as i32)).collect())
                .unwrap_or_default();
            calendars.sort_by(|a, b| b.is_primary.cmp(&a.is_primary).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            Ok(calendars)
        })
    }

    fn fetch_events(&self, account: &Account, calendar: &Calendar, sync_token: Option<&str>) -> BoxFuture<'_, Result<EventChanges, ProviderError>> {
        let account = account.clone();
        let calendar = calendar.clone();
        let sync_token = sync_token.map(str::to_owned);
        Box::pin(async move {
            let mut changes = EventChanges::default();
            let mut page: Option<String> = None;
            let now = overlook_core::time::now();
            let time_min = tz::to_rfc3339(tz::add_days(now, -PAST_DAYS));
            let time_max = tz::to_rfc3339(tz::add_days(now, FUTURE_DAYS));
            loop {
                let mut query: Vec<(&str, &str)> = vec![("singleEvents", "true"), ("maxResults", "250"), ("showDeleted", "true")];
                match (&sync_token, &page) {
                    (Some(t), None) => query.push(("syncToken", t.as_str())),
                    (Some(t), Some(p)) => {
                        query.push(("syncToken", t.as_str()));
                        query.push(("pageToken", p.as_str()));
                    }
                    (None, p) => {
                        query.push(("timeMin", time_min.as_str()));
                        query.push(("timeMax", time_max.as_str()));
                        if let Some(p) = p {
                            query.push(("pageToken", p.as_str()));
                        }
                    }
                }
                let url = Self::events_url(&calendar, &GmailClient::query_string(&query));
                let v = match self.client.get_url(&url) {
                    Ok(v) => v,
                    Err(ApiError::Gone) => return Err(ProviderError::CheckpointExpired),
                    Err(e) => return Err(e.into()),
                };
                for item in v.get("items").and_then(|i| i.as_array()).into_iter().flatten() {
                    if let Some(event) = event_from_json(&account, &calendar, item) {
                        if event.status == EventStatus::Cancelled {
                            changes.deleted.push(event.id);
                        } else {
                            changes.upserts.push(event);
                        }
                    }
                }
                page = v.get("nextPageToken").and_then(|t| t.as_str()).map(str::to_owned);
                if page.is_none() {
                    changes.next_sync_token = v.get("nextSyncToken").and_then(|t| t.as_str()).map(str::to_owned);
                    break;
                }
            }
            info!("calendar {}: {} changes, {} removed", calendar.name, changes.upserts.len(), changes.deleted.len());
            Ok(changes)
        })
    }

    fn respond(&self, _account: &Account, calendar: &Calendar, event: &Event, response: ResponseStatus) -> BoxFuture<'_, Result<(), ProviderError>> {
        let calendar = calendar.clone();
        let mut event = event.clone();
        Box::pin(async move {
            let Some(pid) = event.provider_id.clone() else { return Err(ProviderError::Rejected("event has no Google id".into())) };
            for a in &mut event.attendees {
                if a.is_self {
                    a.response = response;
                }
            }
            let body = json!({ "attendees": event.attendees.iter().map(attendee_to_json).collect::<Vec<_>>() });
            let url = Self::events_url(&calendar, &format!("/{}?sendUpdates=all", url_encode(&pid.0)));
            self.client.patch_url(&url, &body)?;
            Ok(())
        })
    }

    fn create_event(&self, account: &Account, calendar: &Calendar, event: &Event) -> BoxFuture<'_, Result<Event, ProviderError>> {
        let account = account.clone();
        let calendar = calendar.clone();
        let event = event.clone();
        Box::pin(async move {
            let mut body = event_to_json(&event);
            if let Some(pid) = &event.provider_id {
                body["id"] = Value::String(pid.0.clone());
            }
            let url = Self::events_url(&calendar, "?sendUpdates=all");
            let v = match self.client.post_url(&url, &body) {
                Ok(v) => v,
                // Already created by an earlier attempt: fetch it instead of duplicating.
                Err(ApiError::Client(409, _)) => {
                    let pid = event.provider_id.as_ref().map(|p| p.0.clone()).unwrap_or_default();
                    self.client.get_url(&Self::events_url(&calendar, &format!("/{}", url_encode(&pid))))?
                }
                Err(e) => return Err(e.into()),
            };
            event_from_json(&account, &calendar, &v).ok_or_else(|| ProviderError::Rejected("Google returned an unreadable event".into()))
        })
    }

    fn update_event(&self, account: &Account, calendar: &Calendar, event: &Event) -> BoxFuture<'_, Result<Event, ProviderError>> {
        let account = account.clone();
        let calendar = calendar.clone();
        let event = event.clone();
        Box::pin(async move {
            let Some(pid) = event.provider_id.clone() else { return Err(ProviderError::Rejected("event has no Google id".into())) };
            let url = Self::events_url(&calendar, &format!("/{}?sendUpdates=all", url_encode(&pid.0)));
            let v = self.client.patch_url(&url, &event_to_json(&event))?;
            event_from_json(&account, &calendar, &v).ok_or_else(|| ProviderError::Rejected("Google returned an unreadable event".into()))
        })
    }

    fn delete_event(&self, _account: &Account, calendar: &Calendar, provider_id: &ProviderId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let calendar = calendar.clone();
        let pid = provider_id.clone();
        Box::pin(async move {
            let url = Self::events_url(&calendar, &format!("/{}?sendUpdates=all", url_encode(&pid.0)));
            match self.client.delete_url(&url) {
                Ok(_) | Err(ApiError::NotFound) | Err(ApiError::Gone) => Ok(()),
                Err(e) => Err(e.into()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account { id: AccountId::from("g1"), provider: ProviderKind::Gmail, display_name: "Jason".into(), email: "jason@example.com".into(), created_at: 0 }
    }

    #[test]
    fn maps_calendar_and_events() {
        let cal = calendar_from_json(&account().id, &json!({"id":"jason@example.com","summary":"Jason","primary":true,"accessRole":"owner","backgroundColor":"#9fe1e7","selected":true}), 0).unwrap();
        assert!(cal.is_primary && cal.can_edit && cal.visible);
        assert_eq!(cal.id.as_str(), "g1:cal:jason@example.com");
        let timed = event_from_json(&account(), &cal, &json!({
            "id":"abc","iCalUID":"abc@google.com","summary":"Design review","status":"confirmed","htmlLink":"https://calendar.google.com/x",
            "start":{"dateTime":"2026-10-14T09:00:00-07:00"},"end":{"dateTime":"2026-10-14T09:30:00-07:00"},
            "organizer":{"email":"maya@northstar.example","displayName":"Maya Chen"},
            "attendees":[{"email":"maya@northstar.example","organizer":true,"responseStatus":"accepted"},{"email":"jason@example.com","self":true,"responseStatus":"needsAction"}],
            "hangoutLink":"https://meet.google.com/abc"
        })).unwrap();
        assert_eq!(timed.duration_minutes(), 30);
        assert!(timed.is_invitation());
        assert_eq!(timed.my_response(), Some(ResponseStatus::NeedsAction));
        assert_eq!(timed.conference_url.as_deref(), Some("https://meet.google.com/abc"));
        let all_day = event_from_json(&account(), &cal, &json!({"id":"d1","summary":"Offsite","start":{"date":"2026-10-20"},"end":{"date":"2026-10-22"}})).unwrap();
        assert!(all_day.all_day);
        assert_eq!(all_day.end - all_day.start, 2 * 86_400_000);
        let cancelled = event_from_json(&account(), &cal, &json!({"id":"c1","status":"cancelled"})).unwrap();
        assert_eq!(cancelled.status, EventStatus::Cancelled);
        let body = event_to_json(&all_day);
        assert_eq!(body["start"]["date"], "2026-10-20");
        let body = event_to_json(&timed);
        assert!(body["start"]["dateTime"].as_str().unwrap().contains("2026-10-14T16:00:00"));
    }
}
