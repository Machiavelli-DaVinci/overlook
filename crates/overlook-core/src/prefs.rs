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
    /// Seconds a sent message waits in the Outbox so it can be undone; 0 sends at once.
    pub undo_send_secs: u32,
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
    // Updates
    pub check_for_updates: bool,
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
            undo_send_secs: 5,
            quote_on_reply: true,
            remote_images: RemoteImages::Block,
            attachments_dir: None,
            inbox_messages: 100,
            work_start_hour: 8,
            work_end_hour: 18,
            default_event_minutes: 30,
            default_reminder_minutes: 10,
            show_declined_events: false,
            check_for_updates: true,
        }
    }
}

/// One named signature. The body is Markdown-flavoured text like the composer's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Signature {
    pub id: String,
    pub name: String,
    pub body: String,
}

/// Per-account preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AccountPrefs {
    /// The single signature of older versions; migrated into `signatures` by `normalized`.
    pub signature: String,
    pub signatures: Vec<Signature>,
    /// Signature ids used by default; `None` means no signature.
    pub default_new: Option<String>,
    pub default_reply: Option<String>,
    pub sign_new_messages: bool,
    pub sign_replies: bool,
}

impl AccountPrefs {
    /// Brings older stored preferences up to date: the single signature becomes the first
    /// named one, and the two "include" switches become the defaults.
    pub fn normalized(mut self) -> Self {
        if self.signatures.is_empty() && !self.signature.trim().is_empty() {
            self.signatures.push(Signature { id: "default".into(), name: "Default".into(), body: std::mem::take(&mut self.signature) });
            self.default_new = self.sign_new_messages.then(|| "default".to_string());
            self.default_reply = self.sign_replies.then(|| "default".to_string());
        }
        self.sign_new_messages = true;
        self.sign_replies = true;
        let known = |id: &Option<String>| id.as_ref().is_some_and(|i| self.signatures.iter().any(|s| &s.id == i));
        if !known(&self.default_new) {
            self.default_new = None;
        }
        if !known(&self.default_reply) {
            self.default_reply = None;
        }
        self
    }

    pub fn signature_by_id(&self, id: &str) -> Option<&Signature> {
        self.signatures.iter().find(|s| s.id == id)
    }

    pub fn default_signature(&self, reply: bool) -> Option<&Signature> {
        let id = if reply { self.default_reply.as_deref() } else { self.default_new.as_deref() };
        id.and_then(|i| self.signature_by_id(i)).filter(|s| !s.body.trim().is_empty())
    }

    /// Signature block to append to a body, or empty when none applies.
    pub fn signature_block(&self, reply: bool) -> String {
        match self.default_signature(reply) {
            Some(s) => format!("\n\n{}", s.body.trim_end()),
            None => String::new(),
        }
    }

    pub fn new_signature_id() -> String {
        format!("sig-{}", uuid::Uuid::new_v4().simple())
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
        let a = AccountPrefs { signature: "Jason\nLast Refuge  ".into(), sign_new_messages: true, sign_replies: false, ..Default::default() }.normalized();
        assert_eq!(a.signatures.len(), 1);
        assert_eq!(a.signature_block(false), "\n\nJason\nLast Refuge");
        assert_eq!(a.signature_block(true), "");
        let mut b = a.clone();
        b.signatures.push(Signature { id: "short".into(), name: "Short".into(), body: "J.".into() });
        b.default_reply = Some("short".into());
        assert_eq!(b.signature_block(true), "\n\nJ.");
    }
}
