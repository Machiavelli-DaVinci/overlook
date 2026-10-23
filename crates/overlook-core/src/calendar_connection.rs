//! Configuration for an external calendar service. Secrets belong in the OS keyring.
//!
//! Google and sample accounts still use their existing implicit connections. These
//! records introduce CalDAV without changing mail provider identity or credentials.

use crate::{AccountId, CalendarConnectionId};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CalendarCredentialSource {
    /// Explicit user choice to reuse the account's incoming-mail credential.
    IncomingMail,
    /// A dedicated secret owned by this calendar connection.
    Separate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalDavSettings {
    pub server_url: String,
    pub username: String,
    pub credential_source: CalendarCredentialSource,
}

impl CalDavSettings {
    /// Leaving the password field empty may keep only an explicitly separate calendar
    /// credential for the same endpoint and login. It must not silently copy mail access.
    pub fn can_keep_separate_password(&self, previous: &Self) -> bool {
        self.credential_source == CalendarCredentialSource::Separate
            && previous.credential_source == CalendarCredentialSource::Separate
            && self.server_url == previous.server_url
            && self.username == previous.username
    }
    /// Validates without network access. Errors never echo untrusted URLs or credentials.
    pub fn validate(&self) -> Result<Url, &'static str> {
        if self.server_url.len() > 4096 || self.server_url.chars().any(char::is_control) {
            return Err("Enter a valid calendar server URL.");
        }
        let url = Url::parse(&self.server_url).map_err(|_| "Enter a valid calendar server URL.")?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err("The calendar server must use HTTPS.");
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "Use a calendar server URL without credentials, query parameters, or a fragment.",
            );
        }
        if self.username.trim().is_empty()
            || self.username.len() > 1024
            || self.username.contains(':')
            || self.username.chars().any(char::is_control)
        {
            return Err("Enter a valid calendar username.");
        }
        Ok(url)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarConnection {
    pub id: CalendarConnectionId,
    pub account_id: AccountId,
    pub settings: CalDavSettings,
    pub enabled: bool,
    /// Incremented on each configuration change so background work can reject stale results.
    pub generation: u32,
}

impl CalendarConnection {
    /// Use alongside the owning account's keyring attributes. Never aliases a mail secret.
    pub fn secret_kind(&self) -> String {
        format!("password:calendar:{}", self.id)
    }
}

/// A fully downloaded, read-only CalDAV view. Original resources are retained verbatim;
/// the server-expanded events are display projections, never a payload for remote writes.
#[derive(Clone, Debug)]
pub struct CalDavSnapshot {
    pub from: i64,
    pub to: i64,
    pub timezone: String,
    pub calendars: Vec<CalDavCalendarSnapshot>,
}

#[derive(Clone, Debug)]
pub struct CalDavCalendarSnapshot {
    pub calendar: crate::Calendar,
    pub resources: Vec<CalDavResource>,
}

#[derive(Clone, Debug)]
pub struct CalDavResource {
    pub href: String,
    pub etag: String,
    pub icalendar: String,
    pub events: Vec<crate::Event>,
}

/// A collection token is usable only with the successfully cached projection window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalDavCheckpoint {
    pub calendar_id: crate::CalendarId,
    pub token: String,
    pub from: i64,
    pub to: i64,
    pub timezone: String,
}

/// Changed collections contain complete bounded snapshots. Unchanged collections
/// retain their stored resources; their previous checkpoint is checked at commit.
#[derive(Clone, Debug)]
pub struct CalDavDownload {
    pub snapshot: CalDavSnapshot,
    pub checkpoints: Vec<CalDavCheckpoint>,
    pub reused: Vec<CalDavCheckpoint>,
}

impl From<CalDavSnapshot> for CalDavDownload {
    fn from(snapshot: CalDavSnapshot) -> Self {
        Self {
            snapshot,
            checkpoints: Vec::new(),
            reused: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(url: &str) -> CalDavSettings {
        CalDavSettings {
            server_url: url.into(),
            username: "me@example.com".into(),
            credential_source: CalendarCredentialSource::Separate,
        }
    }

    #[test]
    fn rejects_unsafe_endpoints_without_echoing_secrets() {
        for url in [
            "http://example.com/",
            "https://me:secret@example.com/",
            "https://example.com/?token=secret",
            "https://example.com/#secret",
            "file:///secret",
            "https://example.com/\nsecret",
        ] {
            let err = settings(url).validate().unwrap_err();
            assert!(!err.contains("secret"));
        }
        assert!(settings("https://calendar.internal:8443/dav/")
            .validate()
            .is_ok());
    }

    #[test]
    fn rejects_basic_auth_delimiters_in_username() {
        let mut config = settings("https://caldav.fastmail.com/");
        for name in ["", "  ", "me:other", "me\r\ninjected"] {
            config.username = name.into();
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn keeping_a_password_cannot_silently_copy_mail_or_change_its_destination() {
        let old = settings("https://caldav.fastmail.com/");
        assert!(old.can_keep_separate_password(&old));
        let mut changed = old.clone();
        changed.server_url = "https://another.example/".into();
        assert!(!changed.can_keep_separate_password(&old));
        changed = old.clone();
        changed.credential_source = CalendarCredentialSource::IncomingMail;
        assert!(!old.can_keep_separate_password(&changed));
        changed = old.clone();
        changed.username = "other@example.test".into();
        assert!(!changed.can_keep_separate_password(&old));
    }
}
