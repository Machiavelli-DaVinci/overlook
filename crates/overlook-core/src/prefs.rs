//! User preferences. One `Prefs` per profile plus `AccountPrefs` per account, persisted as
//! JSON in the settings table. Every field here drives real behaviour; defaults follow
//! Outlook's conventions where Overlook has an equivalent.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StartupModule {
    #[default]
    Mail,
    Calendar,
    LastUsed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NewMailNotifications {
    Off,
    #[default]
    FocusedOnly,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MarkAsRead {
    #[default]
    WhenSelected,
    AfterDelay,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteImages {
    #[default]
    Block,
    Allow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    // General
    pub startup_module: StartupModule,
    pub clock_24h: bool,
    pub week_starts_monday: bool,
    pub new_mail_notifications: NewMailNotifications,
    pub event_reminders: bool,
    pub sync_interval_secs: u32,
    // Mail
    pub show_avatars: bool,
    pub show_previews: bool,
    pub mark_as_read: MarkAsRead,
    pub mark_as_read_delay_secs: u32,
    pub reply_all_by_default: bool,
    pub quote_on_reply: bool,
    pub remote_images: RemoteImages,
    pub attachments_dir: Option<String>,
    /// Newest Inbox messages fetched on first download (Sent/archive/etc. scale with it).
    pub inbox_messages: u32,
    // Calendar
    pub work_start_hour: u8,
    pub work_end_hour: u8,
    pub default_event_minutes: u32,
    pub default_reminder_minutes: u32,
    pub show_declined_events: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            startup_module: StartupModule::Mail,
            clock_24h: false,
            week_starts_monday: true,
            new_mail_notifications: NewMailNotifications::FocusedOnly,
            event_reminders: true,
            sync_interval_secs: 30,
            show_avatars: true,
            show_previews: true,
            mark_as_read: MarkAsRead::WhenSelected,
            mark_as_read_delay_secs: 3,
            reply_all_by_default: false,
            quote_on_reply: true,
            remote_images: RemoteImages::Block,
            attachments_dir: None,
            inbox_messages: 100,
            work_start_hour: 8,
            work_end_hour: 18,
            default_event_minutes: 30,
            default_reminder_minutes: 10,
            show_declined_events: false,
        }
    }
}

/// Per-account preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AccountPrefs {
    /// Plain-text signature. Rich signatures arrive with rich-text composition.
    pub signature: String,
    pub sign_new_messages: bool,
    pub sign_replies: bool,
}

impl AccountPrefs {
    /// Signature block to append to a body, or empty when none applies.
    pub fn signature_block(&self, reply: bool) -> String {
        let wanted = if reply { self.sign_replies } else { self.sign_new_messages };
        if !wanted || self.signature.trim().is_empty() {
            return String::new();
        }
        format!("\n\n{}", self.signature.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_partial_json() {
        let p: Prefs = serde_json::from_str(r#"{"clock_24h":true}"#).unwrap();
        assert!(p.clock_24h);
        assert_eq!(p.sync_interval_secs, 30);
        assert_eq!(p.new_mail_notifications, NewMailNotifications::FocusedOnly);
    }

    #[test]
    fn signature_block_follows_toggles() {
        let a = AccountPrefs { signature: "Jason\nLast Refuge  ".into(), sign_new_messages: true, sign_replies: false };
        assert_eq!(a.signature_block(false), "\n\nJason\nLast Refuge");
        assert_eq!(a.signature_block(true), "");
    }
}
