//! Durable local storage.
//!
//! One SQLite database per profile holds every account. The interface reads from here
//! immediately, online or offline. Each local mailbox change is written together with its
//! pending operation in a single transaction; callers may only report "Saved on this
//! device" after the call returns `Ok`.
//!
//! Protected data (drafts, Outbox items, outgoing attachments, pending operations) lives in
//! its own tables and is never touched by cache eviction.

mod schema;

use overlook_core::focus::{classify_focus, FocusRule, FocusTab};
use overlook_core::sanitize::html_to_text;
use overlook_core::time::{now, Timestamp};
use overlook_core::*;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Which mailbox the list is showing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MailboxView {
    Kind(LabelKind),
    Label(LabelId),
}

/// Secondary filter tabs above the message list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ListFilter {
    #[default]
    All,
    Unread,
    Attachments,
}

/// A local mailbox change. Each variant maps to one [`OperationKind`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalChange {
    SetRead { message_ids: Vec<MessageId>, read: bool },
    SetStarred { message_ids: Vec<MessageId>, starred: bool },
    AddLabel { message_ids: Vec<MessageId>, label_id: LabelId },
    RemoveLabel { message_ids: Vec<MessageId>, label_id: LabelId },
    Archive { message_ids: Vec<MessageId> },
    Trash { message_ids: Vec<MessageId> },
    Untrash { message_ids: Vec<MessageId> },
}

impl LocalChange {
    fn into_operation(self) -> OperationKind {
        match self {
            LocalChange::SetRead { message_ids, read } => OperationKind::SetRead { message_ids, read },
            LocalChange::SetStarred { message_ids, starred } => OperationKind::SetStarred { message_ids, starred },
            LocalChange::AddLabel { message_ids, label_id } => OperationKind::AddLabel { message_ids, label_id },
            LocalChange::RemoveLabel { message_ids, label_id } => OperationKind::RemoveLabel { message_ids, label_id },
            LocalChange::Archive { message_ids } => OperationKind::Archive { message_ids },
            LocalChange::Trash { message_ids } => OperationKind::Trash { message_ids },
            LocalChange::Untrash { message_ids } => OperationKind::Untrash { message_ids },
        }
    }
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        f(&conn)
    }

    // ----------------------------------------------------------------- accounts

    pub fn upsert_account(&self, account: &Account) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO accounts(id, provider, display_name, email, created_at) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(id) DO UPDATE SET provider=excluded.provider, display_name=excluded.display_name, email=excluded.email",
                params![account.id.0, account.provider.as_key(), account.display_name, account.email, account.created_at],
            )?;
            Ok(())
        })
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, provider, display_name, email, created_at FROM accounts ORDER BY created_at")?;
            let rows = stmt.query_map([], |r| {
                Ok(Account {
                    id: AccountId(r.get(0)?),
                    provider: ProviderKind::from_key(&r.get::<_, String>(1)?).unwrap_or(ProviderKind::Sample),
                    display_name: r.get(2)?,
                    email: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn account_by_email(&self, provider: ProviderKind, email: &str) -> Result<Option<Account>> {
        Ok(self
            .accounts()?
            .into_iter()
            .find(|a| a.provider == provider && a.email.eq_ignore_ascii_case(email)))
    }

    /// Removes an account and everything that belongs to it (cascade).
    pub fn delete_account(&self, account: &AccountId) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute("DELETE FROM accounts WHERE id=?1", [&account.0])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------- labels

    /// Replaces the account's label list with the provider's current list. Labels that no
    /// longer exist are removed (and detached from messages by cascade).
    pub fn replace_labels(&self, account: &AccountId, labels: &[Label]) -> Result<()> {
        self.with_tx(|tx| {
            for label in labels {
                tx.execute(
                    "INSERT INTO labels(id, account_id, kind, name, provider_id, color, sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7)
                     ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, provider_id=excluded.provider_id, color=excluded.color, sort_order=excluded.sort_order",
                    params![label.id.0, label.account_id.0, label.kind.as_key(), label.name, label.provider_id.as_ref().map(|p| &p.0), label.color, label.sort_order],
                )?;
            }
            let keep: Vec<String> = labels.iter().map(|l| l.id.0.clone()).collect();
            let mut stmt = tx.prepare("SELECT id FROM labels WHERE account_id=?1")?;
            let existing: Vec<String> = stmt.query_map([&account.0], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            for id in existing.into_iter().filter(|id| !keep.contains(id)) {
                tx.execute("DELETE FROM labels WHERE id=?1", [&id])?;
            }
            Ok(())
        })
    }

    pub fn upsert_label(&self, label: &Label) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO labels(id, account_id, kind, name, provider_id, color, sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, provider_id=excluded.provider_id, color=excluded.color, sort_order=excluded.sort_order",
                params![label.id.0, label.account_id.0, label.kind.as_key(), label.name, label.provider_id.as_ref().map(|p| &p.0), label.color, label.sort_order],
            )?;
            Ok(())
        })
    }

    pub fn labels(&self, account: &AccountId) -> Result<Vec<Label>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, account_id, kind, name, provider_id, color, sort_order FROM labels WHERE account_id=?1 ORDER BY sort_order, name")?;
            let rows = stmt.query_map([&account.0], row_to_label)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn label_by_kind(&self, account: &AccountId, kind: LabelKind) -> Result<Option<Label>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT id, account_id, kind, name, provider_id, color, sort_order FROM labels WHERE account_id=?1 AND kind=?2 LIMIT 1",
                params![account.0, kind.as_key()],
                row_to_label,
            )
            .optional()?)
        })
    }

    // ----------------------------------------------------------------- messages

    /// Inserts or replaces messages, including their labels and attachments.
    pub fn upsert_messages(&self, messages: &[Message]) -> Result<()> {
        self.with_tx(|tx| {
            for m in messages {
                upsert_message_tx(tx, m)?;
            }
            Ok(())
        })
    }

    /// Merges messages downloaded from the provider into local state.
    ///
    /// Messages referenced by pending local operations keep their local flags and labels
    /// (the pending intent wins until it is replayed). A download without a cached body
    /// keeps the body already on disk.
    pub fn merge_remote_messages(&self, messages: &[Message], protected: &BTreeSet<MessageId>) -> Result<()> {
        self.with_tx(|tx| {
            for incoming in messages {
                let mut existing = load_messages(tx, "WHERE m.id=?1", params![incoming.id.0])?.pop();
                let mut merged = incoming.clone();
                if let Some(local) = existing.take() {
                    if !merged.body.is_cached() {
                        merged.body = local.body;
                    }
                    if merged.attachments.is_empty() {
                        merged.attachments = local.attachments;
                    } else {
                        // Keep downloaded attachment files.
                        for a in &mut merged.attachments {
                            if let Some(l) = local.attachments.iter().find(|l| l.filename == a.filename && l.size_bytes == a.size_bytes) {
                                a.local_path = l.local_path.clone();
                            }
                        }
                    }
                    if merged.focus_override.is_none() {
                        merged.focus_override = local.focus_override;
                    }
                    if protected.contains(&incoming.id) {
                        merged.is_read = local.is_read;
                        merged.is_starred = local.is_starred;
                        merged.label_ids = local.label_ids;
                    }
                }
                upsert_message_tx(tx, &merged)?;
            }
            Ok(())
        })
    }

    /// Removes messages deleted at the provider.
    pub fn delete_messages(&self, ids: &[MessageId]) -> Result<usize> {
        self.with_tx(|tx| {
            let mut n = 0;
            for id in ids {
                n += tx.execute("DELETE FROM messages WHERE id=?1", [&id.0])?;
            }
            Ok(n)
        })
    }

    pub fn message_count(&self, account: &AccountId) -> Result<usize> {
        self.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM messages WHERE account_id=?1", [&account.0], |r| r.get::<_, i64>(0))? as usize))
    }

    pub fn message(&self, id: &MessageId) -> Result<Option<Message>> {
        self.with_conn(|c| {
            let mut found = load_messages(c, "WHERE m.id=?1", params![id.0])?;
            Ok(found.pop())
        })
    }

    pub fn messages_in_conversation(&self, account: &AccountId, conversation: &ConversationId) -> Result<Vec<Message>> {
        self.with_conn(|c| {
            let mut msgs = load_messages(c, "WHERE m.account_id=?1 AND m.conversation_id=?2", params![account.0, conversation.0])?;
            msgs.sort_by_key(|m| m.date);
            Ok(msgs)
        })
    }

    /// All messages of an account (prototype scale). Used to compute conversation lists.
    pub fn messages_for_account(&self, account: &AccountId) -> Result<Vec<Message>> {
        self.with_conn(|c| load_messages(c, "WHERE m.account_id=?1", params![account.0]))
    }

    /// Conversation summaries for a mailbox view, newest first. `focus` splits the Inbox
    /// into Focused and Other; `None` shows everything.
    pub fn conversations(
        &self,
        account: &AccountId,
        owner_email: &str,
        view: &MailboxView,
        filter: ListFilter,
        focus: Option<FocusTab>,
    ) -> Result<Vec<ConversationSummary>> {
        let labels = self.labels(account)?;
        let kind_of = |id: &LabelId| labels.iter().find(|l| &l.id == id).map(|l| l.kind);
        let label_of_kind = |kind: LabelKind| labels.iter().find(|l| l.kind == kind).map(|l| l.id.clone());
        let inbox = label_of_kind(LabelKind::Inbox);
        let trash = label_of_kind(LabelKind::Trash);
        let spam = label_of_kind(LabelKind::Spam);
        let sent = label_of_kind(LabelKind::Sent);
        let in_trash = |m: &Message| trash.as_ref().is_some_and(|t| m.has_label(t));
        let in_spam = |m: &Message| spam.as_ref().is_some_and(|t| m.has_label(t));

        let all = self.messages_for_account(account)?;
        let pending = self.pending_message_ids(account)?;
        let (rules, correspondents) = if focus.is_some() {
            (self.focus_rule_map(account)?, self.sent_recipients(account)?)
        } else {
            (HashMap::new(), HashSet::new())
        };

        // Group every message of the account by conversation first, then decide which
        // conversations belong to the view. This keeps conversation counts consistent
        // across views (a thread in Inbox still shows all its messages).
        let mut by_conv: BTreeMap<ConversationId, Vec<Message>> = BTreeMap::new();
        for m in all {
            by_conv.entry(m.conversation_id.clone()).or_default().push(m);
        }

        let mut out = Vec::new();
        for (_, msgs) in by_conv {
            let matches_view = msgs.iter().any(|m| match view {
                MailboxView::Label(id) => m.has_label(id) && !in_trash(m),
                MailboxView::Kind(LabelKind::Inbox) => inbox.as_ref().is_some_and(|i| m.has_label(i)) && !in_trash(m),
                MailboxView::Kind(LabelKind::Starred) => m.is_starred && !in_trash(m),
                MailboxView::Kind(LabelKind::Sent) => sent.as_ref().is_some_and(|s| m.has_label(s)) && !in_trash(m),
                MailboxView::Kind(LabelKind::Trash) => in_trash(m),
                MailboxView::Kind(LabelKind::Spam) => in_spam(m),
                MailboxView::Kind(LabelKind::Archive) => {
                    !in_trash(m)
                        && !in_spam(m)
                        && !inbox.as_ref().is_some_and(|i| m.has_label(i))
                        && !sent.as_ref().is_some_and(|s| m.has_label(s))
                }
                MailboxView::Kind(LabelKind::Drafts | LabelKind::Outbox) => false,
                MailboxView::Kind(LabelKind::User) => m.label_ids.iter().any(|l| kind_of(l) == Some(LabelKind::User)),
            });
            if !matches_view {
                continue;
            }
            // Inside Trash show only trashed messages; elsewhere hide trashed ones.
            let visible: Vec<Message> = match view {
                MailboxView::Kind(LabelKind::Trash) => msgs.into_iter().filter(|m| in_trash(m)).collect(),
                _ => msgs.into_iter().filter(|m| !in_trash(m)).collect(),
            };
            if let Some(tab) = focus {
                // A conversation follows its newest incoming message; threads with only our
                // own mail count as Focused.
                let decisive = visible.iter().filter(|m| !m.is_outgoing).max_by_key(|m| m.date).or_else(|| visible.iter().max_by_key(|m| m.date));
                let focused = decisive.map(|m| classify_focus(m, &rules, &correspondents)).unwrap_or(true);
                if focused != (tab == FocusTab::Focused) {
                    continue;
                }
            }
            let Some(summary) = ConversationSummary::from_messages(&visible, owner_email, &pending) else { continue };
            let keep = match filter {
                ListFilter::All => true,
                ListFilter::Unread => summary.is_unread(),
                ListFilter::Attachments => summary.has_attachments,
            };
            if keep {
                out.push(summary);
            }
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.latest));
        Ok(out)
    }

    /// Unread conversation count for Inbox, starred count, and so on, for sidebar badges.
    pub fn view_count(&self, account: &AccountId, owner_email: &str, view: &MailboxView, focus: Option<FocusTab>) -> Result<usize> {
        let convs = self.conversations(account, owner_email, view, ListFilter::All, focus)?;
        Ok(match view {
            MailboxView::Kind(LabelKind::Inbox) | MailboxView::Label(_) => convs.iter().filter(|c| c.is_unread()).count(),
            _ => convs.len(),
        })
    }

    // ------------------------------------------------------------ focused inbox

    pub fn focus_rules(&self, account: &AccountId) -> Result<Vec<FocusRule>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT email, focused FROM focus_rules WHERE account_id=?1 ORDER BY email")?;
            let rows = stmt.query_map([&account.0], |r| Ok(FocusRule { email: r.get(0)?, focused: r.get(1)? }))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    fn focus_rule_map(&self, account: &AccountId) -> Result<HashMap<String, bool>> {
        Ok(self.focus_rules(account)?.into_iter().map(|r| (r.email.to_ascii_lowercase(), r.focused)).collect())
    }

    /// Sets (or with `None` clears) the standing rule for a sender.
    pub fn set_focus_rule(&self, account: &AccountId, email: &str, focused: Option<bool>) -> Result<()> {
        let email = email.to_ascii_lowercase();
        self.with_conn(|c| {
            match focused {
                Some(f) => {
                    c.execute(
                        "INSERT INTO focus_rules(account_id, email, focused) VALUES(?1,?2,?3) ON CONFLICT(account_id, email) DO UPDATE SET focused=excluded.focused",
                        params![account.0, email, f],
                    )?;
                }
                None => {
                    c.execute("DELETE FROM focus_rules WHERE account_id=?1 AND email=?2", params![account.0, email])?;
                }
            }
            Ok(())
        })
    }

    /// Per-message override for Focused Inbox (`None` clears it).
    pub fn set_focus_override(&self, ids: &[MessageId], focused: Option<bool>) -> Result<()> {
        self.with_tx(|tx| {
            for id in ids {
                tx.execute("UPDATE messages SET focus_override=?2 WHERE id=?1", params![id.0, focused])?;
            }
            Ok(())
        })
    }

    /// Lowercase addresses the account owner has sent mail to.
    pub fn sent_recipients(&self, account: &AccountId) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        for m in self.messages_for_account(account)? {
            if m.is_outgoing {
                for a in m.to.iter().chain(m.cc.iter()) {
                    out.insert(a.email.to_ascii_lowercase());
                }
            }
        }
        Ok(out)
    }

    // ---------------------------------------------------------------- calendars

    /// Replaces the account's calendars with the provider's list, keeping the user's
    /// visibility choices for calendars that still exist.
    pub fn replace_calendars(&self, account: &AccountId, calendars: &[Calendar]) -> Result<()> {
        self.with_tx(|tx| {
            let mut stmt = tx.prepare("SELECT id, visible FROM calendars WHERE account_id=?1")?;
            let existing: HashMap<String, bool> = stmt
                .query_map([&account.0], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?
                .collect::<std::result::Result<_, _>>()?;
            drop(stmt);
            for c in calendars {
                let visible = existing.get(&c.id.0).copied().unwrap_or(c.visible);
                tx.execute(
                    "INSERT INTO calendars(id, account_id, provider_id, name, color, is_primary, can_edit, visible, sort_order, default_reminders_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                     ON CONFLICT(id) DO UPDATE SET provider_id=excluded.provider_id, name=excluded.name, color=excluded.color, is_primary=excluded.is_primary,
                       can_edit=excluded.can_edit, visible=excluded.visible, sort_order=excluded.sort_order, default_reminders_json=excluded.default_reminders_json",
                    params![c.id.0, c.account_id.0, c.provider_id.as_ref().map(|p| &p.0), c.name, c.color, c.is_primary, c.can_edit, visible, c.sort_order, serde_json::to_string(&c.default_reminders)?],
                )?;
            }
            let keep: Vec<&str> = calendars.iter().map(|c| c.id.0.as_str()).collect();
            for id in existing.keys().filter(|id| !keep.contains(&id.as_str())) {
                tx.execute("DELETE FROM calendars WHERE id=?1", [id])?;
            }
            Ok(())
        })
    }

    pub fn calendars(&self, account: &AccountId) -> Result<Vec<Calendar>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, account_id, provider_id, name, color, is_primary, can_edit, visible, sort_order, default_reminders_json FROM calendars WHERE account_id=?1 ORDER BY is_primary DESC, sort_order, name")?;
            let rows = stmt.query_map([&account.0], |r| {
                Ok(Calendar {
                    id: CalendarId(r.get(0)?),
                    account_id: AccountId(r.get(1)?),
                    provider_id: r.get::<_, Option<String>>(2)?.map(ProviderId),
                    name: r.get(3)?,
                    color: r.get(4)?,
                    is_primary: r.get(5)?,
                    can_edit: r.get(6)?,
                    visible: r.get(7)?,
                    sort_order: r.get(8)?,
                    default_reminders: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn set_calendar_visible(&self, id: &CalendarId, visible: bool) -> Result<()> {
        self.with_conn(|c| {
            c.execute("UPDATE calendars SET visible=?2 WHERE id=?1", params![id.0, visible])?;
            Ok(())
        })
    }

    pub fn upsert_events(&self, events: &[Event]) -> Result<()> {
        self.with_tx(|tx| {
            for e in events {
                upsert_event_tx(tx, e)?;
            }
            Ok(())
        })
    }

    pub fn delete_events(&self, ids: &[EventId]) -> Result<usize> {
        self.with_tx(|tx| {
            let mut n = 0;
            for id in ids {
                n += tx.execute("DELETE FROM events WHERE id=?1", [&id.0])?;
            }
            Ok(n)
        })
    }

    pub fn event_by_ical_uid(&self, account: &AccountId, uid: &str) -> Result<Option<Event>> {
        self.with_conn(|c| {
            let mut found = load_events(c, "WHERE e.account_id=?1 AND e.ical_uid=?2 AND e.status != 'cancelled' ORDER BY e.start", params![account.0, uid])?;
            // Prefer the next upcoming instance for recurring series.
            let now_ms = now();
            let upcoming = found.iter().position(|e| e.end >= now_ms);
            Ok(match upcoming {
                Some(i) => Some(found.swap_remove(i)),
                None => found.pop(),
            })
        })
    }

    /// Events (visible calendars) starting within `[from, to)`, for reminders.
    pub fn events_starting_between(&self, account: &AccountId, from: Timestamp, to: Timestamp) -> Result<Vec<Event>> {
        self.with_conn(|c| {
            load_events(
                c,
                "JOIN calendars cal ON cal.id = e.calendar_id WHERE e.account_id=?1 AND cal.visible=1 AND e.start >= ?2 AND e.start < ?3 AND e.status != 'cancelled' AND e.all_day = 0",
                params![account.0, from, to],
            )
        })
    }

    pub fn event(&self, id: &EventId) -> Result<Option<Event>> {
        self.with_conn(|c| Ok(load_events(c, "WHERE e.id=?1", params![id.0])?.pop()))
    }

    /// Events of visible calendars overlapping `[from, to)`, cancelled ones excluded, with
    /// `pending` set for events referenced by queued operations.
    pub fn events_between(&self, account: &AccountId, from: Timestamp, to: Timestamp) -> Result<Vec<Event>> {
        let pending: HashSet<EventId> = self
            .pending_operations(account)?
            .iter()
            .filter_map(|op| op.kind.event_id().cloned())
            .collect();
        self.with_conn(|c| {
            let mut events = load_events(
                c,
                "JOIN calendars cal ON cal.id = e.calendar_id WHERE e.account_id=?1 AND cal.visible=1 AND e.start < ?3 AND e.end > ?2 AND e.status != 'cancelled'",
                params![account.0, from, to],
            )?;
            for e in &mut events {
                e.pending = pending.contains(&e.id);
            }
            events.sort_by_key(|e| (e.start, e.end));
            Ok(events)
        })
    }

    /// Writes a local event change and its pending operation atomically.
    pub fn apply_event_change(&self, account: &AccountId, event: &Event, kind: OperationKind) -> Result<PendingOperation> {
        let ts = now();
        self.with_tx(|tx| {
            upsert_event_tx(tx, event)?;
            insert_operation_tx(tx, account, kind, ts)
        })
    }

    /// Removes an event locally and queues its deletion.
    pub fn delete_event_locally(&self, account: &AccountId, event: &Event) -> Result<PendingOperation> {
        self.delete_event_locally_scoped(account, event, RecurrenceScope::This)
    }

    /// The local occurrences of `event`'s series that `scope` covers, `event` included.
    fn series_members_tx(tx: &Connection, account: &AccountId, event: &Event, scope: RecurrenceScope) -> Result<Vec<Event>> {
        let Some(master) = event.recurring_event_id.as_deref().filter(|_| scope != RecurrenceScope::This) else { return Ok(vec![event.clone()]) };
        let mut members = load_events(tx, "WHERE e.account_id=?1 AND e.recurring_event_id=?2 ORDER BY e.start", params![account.0, master])?;
        if scope == RecurrenceScope::Following {
            let split = event.split_point();
            members.retain(|m| m.id == event.id || m.split_point() >= split);
        }
        if !members.iter().any(|m| m.id == event.id) {
            members.push(event.clone());
        }
        Ok(members)
    }

    /// Removes an event, or the part of its series `scope` names, and queues one deletion.
    pub fn delete_event_locally_scoped(&self, account: &AccountId, event: &Event, scope: RecurrenceScope) -> Result<PendingOperation> {
        let ts = now();
        self.with_tx(|tx| {
            for member in Self::series_members_tx(tx, account, event, scope)? {
                tx.execute("DELETE FROM events WHERE id=?1", [&member.id.0])?;
            }
            insert_operation_tx(
                tx,
                account,
                OperationKind::DeleteEvent {
                    event_id: event.id.clone(),
                    calendar_id: event.calendar_id.clone(),
                    provider_id: event.provider_id.clone(),
                    scope,
                    recurring_event_id: event.recurring_event_id.clone(),
                    original_start: Some(event.split_point()),
                },
                ts,
            )
        })
    }

    /// Saves an edited occurrence and, for a wider scope, applies the same edit to the other
    /// local occurrences the scope covers: texts and settings as given, times shifted by the
    /// same amount, with the duration of the edited one. One operation carries the scope.
    pub fn apply_series_change(&self, account: &AccountId, event: &Event, scope: RecurrenceScope) -> Result<PendingOperation> {
        let ts = now();
        let before = self.event(&event.id)?;
        let delta = before.as_ref().map(|b| event.start - b.start).unwrap_or(0);
        let duration = event.end - event.start;
        self.with_tx(|tx| {
            let members = before.as_ref().map(|b| Self::series_members_tx(tx, account, b, scope)).transpose()?.unwrap_or_default();
            for member in members {
                if member.id == event.id {
                    continue;
                }
                let mut m = member;
                m.summary = event.summary.clone();
                m.description = event.description.clone();
                m.location = event.location.clone();
                m.color = event.color.clone();
                m.reminders = event.reminders.clone();
                m.attendees = event.attendees.clone();
                m.wants_conference = event.wants_conference;
                m.start += delta;
                m.end = m.start + duration;
                m.all_day = event.all_day;
                m.updated = ts;
                upsert_event_tx(tx, &m)?;
            }
            upsert_event_tx(tx, event)?;
            insert_operation_tx(tx, account, OperationKind::UpdateEvent { event_id: event.id.clone(), scope }, ts)
        })
    }

    /// Provider ids of messages that carry a calendar attachment but no parsed invitation
    /// (stored before invitation parsing existed).
    /// Provider ids of messages whose HTML embeds `cid:` images but whose attachments were
    /// stored before `Content-ID` was recorded.
    pub fn messages_needing_inline_parse(&self, account: &AccountId) -> Result<Vec<ProviderId>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT DISTINCT m.provider_id FROM messages m
                 WHERE m.account_id=?1 AND m.provider_id IS NOT NULL AND m.body_html LIKE '%cid:%'
                   AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND a.content_id IS NOT NULL)",
            )?;
            let rows = stmt.query_map([&account.0], |r| Ok(ProviderId(r.get(0)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn messages_needing_invitation_parse(&self, account: &AccountId) -> Result<Vec<ProviderId>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT DISTINCT m.provider_id FROM messages m JOIN attachments a ON a.message_id = m.id
                 WHERE m.account_id=?1 AND m.invitation_json IS NULL AND m.provider_id IS NOT NULL
                   AND (a.mime_type LIKE 'text/calendar%' OR a.mime_type LIKE 'application/ics%' OR lower(a.filename) LIKE '%.ics')",
            )?;
            let rows = stmt.query_map([&account.0], |r| Ok(ProviderId(r.get(0)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    // -------------------------------------------------------------- attachments

    pub fn set_attachment_local_path(&self, id: &AttachmentId, path: Option<&str>) -> Result<()> {
        self.with_conn(|c| {
            c.execute("UPDATE attachments SET local_path=?2 WHERE id=?1", params![id.0, path])?;
            Ok(())
        })
    }

    // ------------------------------------------------------- local changes + ops

    /// Applies a local change and records its pending operation atomically.
    pub fn apply_change(&self, account: &AccountId, change: LocalChange) -> Result<PendingOperation> {
        let ts = now();
        self.with_tx(|tx| {
            apply_change_tx(tx, account, &change)?;
            let op = insert_operation_tx(tx, account, change.into_operation(), ts)?;
            Ok(op)
        })
    }

    pub fn pending_operations(&self, account: &AccountId) -> Result<Vec<PendingOperation>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, account_id, seq, kind_json, state, attempts, last_error, created_at, updated_at FROM pending_operations
                 WHERE account_id=?1 AND state != 'applied' ORDER BY seq",
            )?;
            let rows = stmt.query_map([&account.0], row_to_operation)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn pending_count(&self, account: &AccountId) -> Result<usize> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM pending_operations WHERE account_id=?1 AND state != 'applied'",
                [&account.0],
                |r| r.get::<_, i64>(0),
            )? as usize)
        })
    }

    /// Message ids referenced by operations that have not been applied yet.
    /// Events touched by operations that have not reached the provider yet. A download must
    /// not overwrite or resurrect them (for example a deleted event the server still lists).
    pub fn pending_event_ids(&self, account: &AccountId) -> Result<BTreeSet<EventId>> {
        let ops = self.pending_operations(account)?;
        Ok(ops.iter().filter_map(|op| op.kind.event_id().cloned()).collect())
    }

    pub fn pending_message_ids(&self, account: &AccountId) -> Result<BTreeSet<MessageId>> {
        let ops = self.pending_operations(account)?;
        let mut ids = BTreeSet::new();
        for op in ops {
            ids.extend(op.kind.message_ids().iter().cloned());
        }
        Ok(ids)
    }

    pub fn set_operation_state(&self, id: &OperationId, state: OperationState, error: Option<&str>) -> Result<()> {
        self.with_conn(|c| {
            let bump = matches!(state, OperationState::InFlight) as i64;
            c.execute(
                "UPDATE pending_operations SET state=?2, last_error=?3, attempts=attempts+?4, updated_at=?5 WHERE id=?1",
                params![id.0, state.as_key(), error, bump, now()],
            )?;
            Ok(())
        })
    }

    /// Removes applied operations older than `before`.
    pub fn prune_applied(&self, before: Timestamp) -> Result<usize> {
        self.with_conn(|c| Ok(c.execute("DELETE FROM pending_operations WHERE state='applied' AND updated_at < ?1", [before])?))
    }

    // ------------------------------------------------------------------- drafts

    pub fn save_draft(&self, draft: &Draft) -> Result<()> {
        self.with_tx(|tx| save_draft_tx(tx, draft))
    }

    pub fn draft(&self, id: &DraftId) -> Result<Option<Draft>> {
        self.with_conn(|c| {
            let mut found = load_drafts(c, "WHERE d.id=?1", params![id.0])?;
            Ok(found.pop())
        })
    }

    /// Drafts of an account. `outbox` selects Outbox states instead of editing drafts.
    pub fn drafts(&self, account: &AccountId, outbox: bool) -> Result<Vec<Draft>> {
        self.with_conn(|c| {
            let mut all = load_drafts(c, "WHERE d.account_id=?1", params![account.0])?;
            all.retain(|d| d.state.is_outbox() == outbox);
            all.sort_by_key(|d| std::cmp::Reverse(d.updated_at));
            Ok(all)
        })
    }

    pub fn delete_draft(&self, id: &DraftId) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute("DELETE FROM drafts WHERE id=?1", [&id.0])?;
            // Any queued send for this draft is cancelled with it.
            tx.execute(
                "DELETE FROM pending_operations WHERE state='pending' AND kind_json LIKE ?1",
                [format!("%\"draft_id\":\"{}\"%", id.0)],
            )?;
            Ok(())
        })
    }

    /// Moves a draft to the Outbox and queues its send in the same transaction.
    pub fn queue_send(&self, draft: &Draft) -> Result<PendingOperation> {
        draft.validate_for_send().map_err(|e| StoreError::Invalid(e.to_string()))?;
        let ts = now();
        let mut queued = draft.clone();
        queued.state = DraftState::Queued;
        queued.updated_at = ts;
        self.with_tx(|tx| {
            save_draft_tx(tx, &queued)?;
            insert_operation_tx(tx, &draft.account_id, OperationKind::SendDraft { draft_id: draft.id.clone() }, ts)
        })
    }

    /// Returns a queued (not yet in-flight) draft to editing and removes its send operation.
    pub fn cancel_send(&self, id: &DraftId) -> Result<Draft> {
        self.with_tx(|tx| {
            let mut found = load_drafts(tx, "WHERE d.id=?1", params![id.0])?;
            let mut draft = found.pop().ok_or_else(|| StoreError::NotFound(id.0.clone()))?;
            if !matches!(draft.state, DraftState::Queued | DraftState::Failed(_)) {
                return Err(StoreError::Invalid("This message is already being sent.".into()));
            }
            draft.state = DraftState::Editing;
            draft.updated_at = now();
            save_draft_tx(tx, &draft)?;
            tx.execute(
                "DELETE FROM pending_operations WHERE state IN ('pending','failed') AND kind_json LIKE ?1",
                [format!("%\"draft_id\":\"{}\"%", id.0)],
            )?;
            Ok(draft)
        })
    }

    pub fn set_draft_state(&self, id: &DraftId, state: &DraftState) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE drafts SET state_json=?2, updated_at=?3 WHERE id=?1",
                params![id.0, serde_json::to_string(state)?, now()],
            )?;
            Ok(())
        })
    }

    /// Records provider acceptance: stores the sent message, removes the draft, and marks
    /// the send operation applied, all atomically.
    pub fn complete_send(&self, draft_id: &DraftId, op_id: &OperationId, sent: &Message) -> Result<()> {
        self.with_tx(|tx| {
            upsert_message_tx(tx, sent)?;
            tx.execute("DELETE FROM drafts WHERE id=?1", [&draft_id.0])?;
            tx.execute(
                "UPDATE pending_operations SET state='applied', last_error=NULL, updated_at=?2 WHERE id=?1",
                params![op_id.0, now()],
            )?;
            Ok(())
        })
    }

    // --------------------------------------------------------------- sync state

    pub fn account_status(&self, account: &AccountId) -> Result<AccountStatus> {
        let (state, last) = self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT state_json, last_successful_sync FROM sync_state WHERE account_id=?1",
                [&account.0],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()?)
        })?
        .map(|(s, l)| (serde_json::from_str::<SyncState>(&s).unwrap_or(SyncState::Offline), l))
        .unwrap_or((SyncState::Offline, None));
        Ok(AccountStatus {
            state,
            last_successful_sync: last,
            pending_operations: self.pending_count(account)?,
            outbox_items: self.drafts(account, true)?.len(),
        })
    }

    pub fn set_sync_state(&self, account: &AccountId, state: &SyncState, successful_at: Option<Timestamp>) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO sync_state(account_id, state_json, last_successful_sync) VALUES(?1,?2,?3)
                 ON CONFLICT(account_id) DO UPDATE SET state_json=excluded.state_json,
                   last_successful_sync=COALESCE(excluded.last_successful_sync, sync_state.last_successful_sync)",
                params![account.0, serde_json::to_string(state)?, successful_at],
            )?;
            Ok(())
        })
    }

    /// Provider sync checkpoint (for Gmail, the history id). Saved transactionally with
    /// the downloaded changes by callers that use `with_tx` through [`Store::upsert_messages`].
    pub fn checkpoint(&self, account: &AccountId) -> Result<Option<String>> {
        self.with_conn(|c| {
            Ok(c.query_row("SELECT checkpoint FROM sync_state WHERE account_id=?1", [&account.0], |r| r.get::<_, Option<String>>(0))
                .optional()?
                .flatten())
        })
    }

    pub fn set_checkpoint(&self, account: &AccountId, checkpoint: Option<&str>) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO sync_state(account_id, state_json, checkpoint) VALUES(?1, '{\"state\":\"offline\"}', ?2)
                 ON CONFLICT(account_id) DO UPDATE SET checkpoint=excluded.checkpoint",
                params![account.0, checkpoint],
            )?;
            Ok(())
        })
    }

    // ----------------------------------------------------------------- settings

    pub fn setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.with_conn(|c| {
            let raw: Option<String> = c.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()?;
            Ok(match raw {
                Some(s) => serde_json::from_str(&s).ok(),
                None => None,
            })
        })
    }

    pub fn set_setting<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO settings(key, value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, serde_json::to_string(value)?],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------- search

    /// Full-text search over downloaded mail. Returns matching message ids, best first.
    pub fn search(&self, account: &AccountId, query: &str) -> Result<Vec<MessageId>> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT m.id FROM messages_fts f JOIN messages m ON m.rowid = f.rowid
                 WHERE messages_fts MATCH ?1 AND m.account_id = ?2 ORDER BY rank LIMIT 500",
            )?;
            let rows = stmt.query_map(params![fts, account.0], |r| Ok(MessageId(r.get(0)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn search_coverage(&self, account: &AccountId) -> Result<SearchCoverage> {
        self.with_conn(|c| {
            let total: i64 = c.query_row("SELECT COUNT(*) FROM messages WHERE account_id=?1", [&account.0], |r| r.get(0))?;
            let cached: i64 = c.query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id=?1 AND (body_text IS NOT NULL OR body_html IS NOT NULL)",
                [&account.0],
                |r| r.get(0),
            )?;
            Ok(SearchCoverage { messages_indexed: total as usize, messages_total: total as usize, bodies_cached: cached as usize })
        })
    }

    /// Replaces the account's contacts with the provider's list. Contacts changed on this device
    /// and not yet pushed are kept.
    pub fn replace_contacts(&self, account: &AccountId, contacts: &[Contact]) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute("DELETE FROM people WHERE account_id=?1 AND pending=0", [&account.0])?;
            for c in contacts {
                if tx.query_row("SELECT COUNT(*) FROM people WHERE account_id=?1 AND id=?2", params![account.0, c.id], |r| r.get::<_, i64>(0))? > 0 {
                    continue; // a pending local edit wins until it is pushed
                }
                upsert_contact_tx(tx, c)?;
            }
            Ok(())
        })
    }

    pub fn contact_count(&self, account: &AccountId) -> Result<usize> {
        self.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM people WHERE account_id=?1", [&account.0], |r| r.get::<_, i64>(0))? as usize))
    }

    pub fn contacts(&self, account: &AccountId) -> Result<Vec<Contact>> {
        let mut all = self.contacts_where(account, "", &[])?;
        all.sort_by_key(|c| c.display_name().to_lowercase());
        Ok(all)
    }

    pub fn contact_by_id(&self, account: &AccountId, id: &str) -> Result<Option<Contact>> {
        Ok(self.contacts_where(account, "AND id=?2", &[id])?.into_iter().next())
    }

    /// The contact with this address, address-book entries first.
    pub fn contact(&self, account: &AccountId, email: &str) -> Result<Option<Contact>> {
        let needle = email.trim().to_ascii_lowercase();
        let mut hits: Vec<Contact> = self.contacts_where(account, "", &[])?.into_iter().filter(|c| c.has_email(&needle)).collect();
        hits.sort_by(|a, b| b.in_address_book.cmp(&a.in_address_book));
        Ok(hits.into_iter().next())
    }

    /// Contacts whose name or address starts with `prefix` (any word of the name), address
    /// book entries first.
    pub fn contacts_matching(&self, account: &AccountId, prefix: &str, limit: usize) -> Result<Vec<Contact>> {
        let needle = prefix.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits: Vec<Contact> = self
            .contacts_where(account, "", &[])?
            .into_iter()
            .filter(|c| {
                c.emails.iter().any(|e| e.to_ascii_lowercase().starts_with(&needle))
                    || c.email.to_ascii_lowercase().starts_with(&needle)
                    || c.display_name().to_lowercase().split_whitespace().any(|w| w.starts_with(&needle))
                    || c.display_name().to_lowercase().starts_with(&needle)
            })
            .collect();
        hits.sort_by(|a, b| b.in_address_book.cmp(&a.in_address_book).then_with(|| a.name.is_none().cmp(&b.name.is_none())).then_with(|| a.display_name().cmp(&b.display_name())));
        hits.truncate(limit);
        Ok(hits)
    }

    /// Saves a contact changed on this device and queues the push.
    pub fn save_contact_locally(&self, account: &AccountId, contact: &Contact) -> Result<PendingOperation> {
        let ts = now();
        let mut c = contact.clone();
        c.pending = true;
        c.updated = ts;
        let kind = if c.id.starts_with("local:") { OperationKind::CreateContact { contact_id: c.id.clone() } } else { OperationKind::UpdateContact { contact_id: c.id.clone() } };
        self.with_tx(|tx| {
            upsert_contact_tx(tx, &c)?;
            insert_operation_tx(tx, account, kind, ts)
        })
    }

    /// Removes a contact locally and queues its deletion.
    pub fn delete_contact_locally(&self, account: &AccountId, contact: &Contact) -> Result<PendingOperation> {
        let ts = now();
        self.with_tx(|tx| {
            tx.execute("DELETE FROM people WHERE account_id=?1 AND id=?2", params![account.0, contact.id])?;
            let resource_name = (!contact.id.starts_with("local:")).then(|| contact.id.clone());
            insert_operation_tx(tx, account, OperationKind::DeleteContact { contact_id: contact.id.clone(), resource_name }, ts)
        })
    }

    /// The provider accepted a local change: the row takes the provider's copy (and id).
    pub fn mark_contact_synced(&self, account: &AccountId, local_id: &str, synced: &Contact) -> Result<()> {
        let mut c = synced.clone();
        c.pending = false;
        self.with_tx(|tx| {
            if local_id != c.id {
                tx.execute("DELETE FROM people WHERE account_id=?1 AND id=?2", params![account.0, local_id])?;
            }
            upsert_contact_tx(tx, &c)
        })
    }

    fn contacts_where(&self, account: &AccountId, clause: &str, args: &[&str]) -> Result<Vec<Contact>> {
        self.with_conn(|c| {
            let sql = format!(
                "SELECT id, account_id, email, emails_json, name, given_name, family_name, phone, organization, title, notes, photo_url, in_address_book, etag, updated, pending
                 FROM people WHERE account_id=?1 {clause}"
            );
            let mut stmt = c.prepare(&sql)?;
            let map = |r: &rusqlite::Row| {
                Ok(Contact {
                    id: r.get(0)?,
                    account_id: AccountId(r.get(1)?),
                    email: r.get(2)?,
                    emails: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                    name: r.get(4)?,
                    given_name: r.get(5)?,
                    family_name: r.get(6)?,
                    phone: r.get(7)?,
                    organization: r.get(8)?,
                    title: r.get(9)?,
                    notes: r.get(10)?,
                    photo_url: r.get(11)?,
                    in_address_book: r.get::<_, i64>(12)? != 0,
                    etag: r.get(13)?,
                    updated: r.get(14)?,
                    pending: r.get::<_, i64>(15)? != 0,
                })
            };
            let rows: Vec<Contact> = match args {
                [] => stmt.query_map([&account.0], map)?.collect::<std::result::Result<Vec<_>, _>>()?,
                [a] => stmt.query_map(params![account.0, a], map)?.collect::<std::result::Result<Vec<_>, _>>()?,
                _ => Vec::new(),
            };
            Ok(rows)
        })
    }

    /// Addresses the user has corresponded with, most frequent first, for recipient
    /// autocomplete.
    pub fn known_recipients(&self, account: &AccountId, prefix: &str, limit: usize) -> Result<Vec<Address>> {
        let msgs = self.messages_for_account(account)?;
        let needle = prefix.trim().to_ascii_lowercase();
        let mut counts: BTreeMap<String, (Address, usize)> = BTreeMap::new();
        for m in msgs {
            for a in std::iter::once(&m.from).chain(m.to.iter()).chain(m.cc.iter()) {
                let key = a.email.to_ascii_lowercase();
                let entry = counts.entry(key).or_insert_with(|| (a.clone(), 0));
                entry.1 += 1;
                if entry.0.name.is_none() && a.name.is_some() {
                    entry.0 = a.clone();
                }
            }
        }
        let mut list: Vec<(Address, usize)> = counts
            .into_values()
            .filter(|(a, _)| {
                needle.is_empty()
                    || a.email.to_ascii_lowercase().contains(&needle)
                    || a.name.as_deref().is_some_and(|n| n.to_ascii_lowercase().contains(&needle))
            })
            .collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.email.cmp(&b.0.email)));
        Ok(list.into_iter().take(limit).map(|(a, _)| a).collect())
    }
}

// ---------------------------------------------------------------------- helpers

fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn row_to_label(r: &Row<'_>) -> rusqlite::Result<Label> {
    Ok(Label {
        id: LabelId(r.get(0)?),
        account_id: AccountId(r.get(1)?),
        kind: LabelKind::from_key(&r.get::<_, String>(2)?).unwrap_or(LabelKind::User),
        name: r.get(3)?,
        provider_id: r.get::<_, Option<String>>(4)?.map(ProviderId),
        color: r.get(5)?,
        sort_order: r.get(6)?,
    })
}

fn row_to_operation(r: &Row<'_>) -> rusqlite::Result<PendingOperation> {
    let kind_json: String = r.get(3)?;
    let kind: OperationKind = serde_json::from_str(&kind_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e)))?;
    Ok(PendingOperation {
        id: OperationId(r.get(0)?),
        account_id: AccountId(r.get(1)?),
        seq: r.get(2)?,
        kind,
        state: OperationState::from_key(&r.get::<_, String>(4)?).unwrap_or(OperationState::Pending),
        attempts: r.get::<_, i64>(5)? as u32,
        last_error: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

fn upsert_message_tx(tx: &Connection, m: &Message) -> Result<()> {
    tx.execute(
        "INSERT INTO messages(id, account_id, conversation_id, provider_id, from_json, to_json, cc_json, bcc_json, reply_to_json,
            subject, snippet, body_text, body_html, search_text, date, is_read, is_starred, is_outgoing, rfc_message_id, in_reply_to,
            category, is_important, focus_override, invitation_json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)
         ON CONFLICT(id) DO UPDATE SET conversation_id=excluded.conversation_id, provider_id=excluded.provider_id,
            from_json=excluded.from_json, to_json=excluded.to_json, cc_json=excluded.cc_json, bcc_json=excluded.bcc_json,
            reply_to_json=excluded.reply_to_json, subject=excluded.subject, snippet=excluded.snippet,
            body_text=excluded.body_text, body_html=excluded.body_html, search_text=excluded.search_text, date=excluded.date,
            is_read=excluded.is_read, is_starred=excluded.is_starred, is_outgoing=excluded.is_outgoing,
            rfc_message_id=excluded.rfc_message_id, in_reply_to=excluded.in_reply_to,
            category=excluded.category, is_important=excluded.is_important, focus_override=excluded.focus_override, invitation_json=excluded.invitation_json",
        params![
            m.id.0,
            m.account_id.0,
            m.conversation_id.0,
            m.provider_id.as_ref().map(|p| &p.0),
            serde_json::to_string(&m.from)?,
            serde_json::to_string(&m.to)?,
            serde_json::to_string(&m.cc)?,
            serde_json::to_string(&m.bcc)?,
            m.reply_to.as_ref().map(serde_json::to_string).transpose()?,
            m.subject,
            m.snippet,
            m.body.text,
            m.body.html,
            search_text(m),
            m.date,
            m.is_read,
            m.is_starred,
            m.is_outgoing,
            m.rfc_message_id,
            m.in_reply_to,
            m.category,
            m.is_important,
            m.focus_override,
            m.invitation.as_ref().map(serde_json::to_string).transpose()?,
        ],
    )?;
    tx.execute("DELETE FROM message_labels WHERE message_id=?1", [&m.id.0])?;
    for label in &m.label_ids {
        tx.execute("INSERT INTO message_labels(message_id, label_id) VALUES(?1,?2)", params![m.id.0, label.0])?;
    }
    tx.execute("DELETE FROM attachments WHERE message_id=?1", [&m.id.0])?;
    for a in &m.attachments {
        tx.execute(
            "INSERT INTO attachments(id, message_id, filename, mime_type, size_bytes, local_path, provider_id, content_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![a.id.0, m.id.0, a.filename, a.mime_type, a.size_bytes as i64, a.local_path, a.provider_id.as_ref().map(|p| &p.0), a.content_id],
        )?;
    }
    Ok(())
}

fn upsert_contact_tx(tx: &Connection, c: &Contact) -> Result<()> {
    tx.execute(
        "INSERT INTO people(id, account_id, email, emails_json, name, given_name, family_name, phone, organization, title, notes, photo_url, in_address_book, etag, updated, pending)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
         ON CONFLICT(account_id, id) DO UPDATE SET email=excluded.email, emails_json=excluded.emails_json, name=excluded.name, given_name=excluded.given_name,
            family_name=excluded.family_name, phone=excluded.phone, organization=excluded.organization, title=excluded.title, notes=excluded.notes,
            photo_url=excluded.photo_url, in_address_book=excluded.in_address_book, etag=excluded.etag, updated=excluded.updated, pending=excluded.pending",
        params![
            c.id,
            c.account_id.0,
            c.email.to_ascii_lowercase(),
            serde_json::to_string(&c.emails)?,
            c.name,
            c.given_name,
            c.family_name,
            c.phone,
            c.organization,
            c.title,
            c.notes,
            c.photo_url,
            c.in_address_book as i64,
            c.etag,
            c.updated,
            c.pending as i64,
        ],
    )?;
    Ok(())
}

fn upsert_event_tx(tx: &Connection, e: &Event) -> Result<()> {
    tx.execute(
        "INSERT INTO events(id, account_id, calendar_id, provider_id, ical_uid, summary, description, location, start, end, all_day, status,
            organizer_json, attendees_json, html_link, conference_url, recurring_event_id, updated, event_type, color, reminders_json, original_start, wants_conference)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
         ON CONFLICT(id) DO UPDATE SET calendar_id=excluded.calendar_id, provider_id=excluded.provider_id, ical_uid=excluded.ical_uid,
            summary=excluded.summary, description=excluded.description, location=excluded.location, start=excluded.start, end=excluded.end,
            all_day=excluded.all_day, status=excluded.status, organizer_json=excluded.organizer_json, attendees_json=excluded.attendees_json,
            html_link=excluded.html_link, conference_url=excluded.conference_url, recurring_event_id=excluded.recurring_event_id, updated=excluded.updated,
            event_type=excluded.event_type, color=excluded.color, reminders_json=excluded.reminders_json,
            original_start=excluded.original_start, wants_conference=excluded.wants_conference",
        params![
            e.id.0,
            e.account_id.0,
            e.calendar_id.0,
            e.provider_id.as_ref().map(|p| &p.0),
            e.ical_uid,
            e.summary,
            e.description,
            e.location,
            e.start,
            e.end,
            e.all_day,
            serde_json::to_string(&e.status)?.trim_matches('"').to_owned(),
            e.organizer.as_ref().map(serde_json::to_string).transpose()?,
            serde_json::to_string(&e.attendees)?,
            e.html_link,
            e.conference_url,
            e.recurring_event_id,
            e.updated,
            e.event_type,
            e.color,
            e.reminders.as_ref().map(serde_json::to_string).transpose()?,
            e.original_start,
            e.wants_conference,
        ],
    )?;
    Ok(())
}

fn load_events(c: &Connection, clause: &str, args: impl rusqlite::Params) -> Result<Vec<Event>> {
    let sql = format!(
        "SELECT e.id, e.account_id, e.calendar_id, e.provider_id, e.ical_uid, e.summary, e.description, e.location, e.start, e.end, e.all_day, e.status,
                e.organizer_json, e.attendees_json, e.html_link, e.conference_url, e.recurring_event_id, e.updated, e.event_type, e.color, e.reminders_json,
                e.original_start, e.wants_conference FROM events e {clause}"
    );
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, i64>(8)?,
            r.get::<_, i64>(9)?,
            r.get::<_, bool>(10)?,
            r.get::<_, String>(11)?,
            r.get::<_, Option<String>>(12)?,
            r.get::<_, String>(13)?,
            r.get::<_, Option<String>>(14)?,
            r.get::<_, Option<String>>(15)?,
            r.get::<_, Option<String>>(16)?,
            r.get::<_, i64>(17)?,
            r.get::<_, Option<String>>(18)?,
            r.get::<_, Option<String>>(19)?,
            r.get::<_, Option<String>>(20)?,
            (r.get::<_, Option<i64>>(21)?, r.get::<_, i64>(22)? != 0),
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, account, calendar, provider, ical, summary, description, location, start, end, all_day, status, organizer, attendees, link, conf, recurring, updated, event_type, color, reminders, (original_start, wants_conference)) = row?;
        out.push(Event {
            id: EventId(id),
            account_id: AccountId(account),
            calendar_id: CalendarId(calendar),
            provider_id: provider.map(ProviderId),
            ical_uid: ical,
            summary,
            description,
            location,
            start,
            end,
            all_day,
            status: serde_json::from_str(&format!("\"{status}\"")).unwrap_or_default(),
            organizer: organizer.as_deref().map(serde_json::from_str).transpose()?,
            attendees: serde_json::from_str(&attendees)?,
            html_link: link,
            conference_url: conf,
            recurring_event_id: recurring,
            updated,
            event_type,
            color,
            reminders: reminders.as_deref().map(serde_json::from_str).transpose()?,
            original_start,
            wants_conference,
            pending: false,
        });
    }
    Ok(out)
}

fn search_text(m: &Message) -> String {
    let body = match (&m.body.text, &m.body.html) {
        (Some(t), _) => t.clone(),
        (None, Some(h)) => html_to_text(h),
        (None, None) => m.snippet.clone(),
    };
    let people: Vec<String> = std::iter::once(&m.from)
        .chain(m.to.iter())
        .chain(m.cc.iter())
        .map(|a| format!("{} {}", a.display_name(), a.email))
        .collect();
    format!("{}\n{}", people.join(" "), body)
}

fn load_messages(c: &Connection, where_clause: &str, args: impl rusqlite::Params) -> Result<Vec<Message>> {
    let sql = format!(
        "SELECT m.id, m.account_id, m.conversation_id, m.provider_id, m.from_json, m.to_json, m.cc_json, m.bcc_json, m.reply_to_json,
                m.subject, m.snippet, m.body_text, m.body_html, m.date, m.is_read, m.is_starred, m.is_outgoing, m.rfc_message_id, m.in_reply_to,
                m.category, m.is_important, m.focus_override, m.invitation_json
         FROM messages m {where_clause} ORDER BY m.date DESC"
    );
    let mut stmt = c.prepare(&sql)?;
    let mut messages: Vec<Message> = stmt
        .query_map(args, |r| {
            let parse = |i: usize| -> rusqlite::Result<String> { r.get(i) };
            Ok(RawMessage {
                id: parse(0)?,
                account_id: parse(1)?,
                conversation_id: parse(2)?,
                provider_id: r.get(3)?,
                from_json: parse(4)?,
                to_json: parse(5)?,
                cc_json: parse(6)?,
                bcc_json: parse(7)?,
                reply_to_json: r.get(8)?,
                subject: parse(9)?,
                snippet: parse(10)?,
                body_text: r.get(11)?,
                body_html: r.get(12)?,
                date: r.get(13)?,
                is_read: r.get(14)?,
                is_starred: r.get(15)?,
                is_outgoing: r.get(16)?,
                rfc_message_id: r.get(17)?,
                in_reply_to: r.get(18)?,
                category: r.get(19)?,
                is_important: r.get(20)?,
                focus_override: r.get(21)?,
                invitation_json: r.get(22)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(RawMessage::into_message)
        .collect::<Result<Vec<_>>>()?;

    if messages.is_empty() {
        return Ok(messages);
    }
    // Labels and attachments in two passes to avoid N+1 queries.
    let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
    for (i, m) in messages.iter().enumerate() {
        by_id.insert(m.id.0.clone(), i);
    }
    let mut stmt = c.prepare("SELECT message_id, label_id FROM message_labels")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (mid, lid) = row?;
        if let Some(&i) = by_id.get(&mid) {
            messages[i].label_ids.insert(LabelId(lid));
        }
    }
    let mut stmt = c.prepare("SELECT id, message_id, filename, mime_type, size_bytes, local_path, provider_id, content_id FROM attachments ORDER BY rowid")?;
    for row in stmt.query_map([], |r| {
        Ok(Attachment {
            id: AttachmentId(r.get(0)?),
            message_id: MessageId(r.get(1)?),
            filename: r.get(2)?,
            mime_type: r.get(3)?,
            size_bytes: r.get::<_, i64>(4)? as u64,
            local_path: r.get(5)?,
            provider_id: r.get::<_, Option<String>>(6)?.map(ProviderId),
            content_id: r.get(7)?,
        })
    })? {
        let a = row?;
        if let Some(&i) = by_id.get(&a.message_id.0) {
            messages[i].attachments.push(a);
        }
    }
    Ok(messages)
}

struct RawMessage {
    id: String,
    account_id: String,
    conversation_id: String,
    provider_id: Option<String>,
    from_json: String,
    to_json: String,
    cc_json: String,
    bcc_json: String,
    reply_to_json: Option<String>,
    subject: String,
    snippet: String,
    body_text: Option<String>,
    body_html: Option<String>,
    date: i64,
    is_read: bool,
    is_starred: bool,
    is_outgoing: bool,
    rfc_message_id: Option<String>,
    in_reply_to: Option<String>,
    category: Option<String>,
    is_important: bool,
    focus_override: Option<bool>,
    invitation_json: Option<String>,
}

impl RawMessage {
    fn into_message(self) -> Result<Message> {
        Ok(Message {
            id: MessageId(self.id),
            account_id: AccountId(self.account_id),
            conversation_id: ConversationId(self.conversation_id),
            provider_id: self.provider_id.map(ProviderId),
            from: serde_json::from_str(&self.from_json)?,
            to: serde_json::from_str(&self.to_json)?,
            cc: serde_json::from_str(&self.cc_json)?,
            bcc: serde_json::from_str(&self.bcc_json)?,
            reply_to: self.reply_to_json.as_deref().map(serde_json::from_str).transpose()?,
            subject: self.subject,
            snippet: self.snippet,
            body: Body { text: self.body_text, html: self.body_html },
            date: self.date,
            is_read: self.is_read,
            is_starred: self.is_starred,
            is_outgoing: self.is_outgoing,
            label_ids: BTreeSet::new(),
            attachments: Vec::new(),
            rfc_message_id: self.rfc_message_id,
            in_reply_to: self.in_reply_to,
            category: self.category,
            is_important: self.is_important,
            focus_override: self.focus_override,
            invitation: self.invitation_json.as_deref().and_then(|j| serde_json::from_str(j).ok()),
        })
    }
}

fn apply_change_tx(tx: &Connection, account: &AccountId, change: &LocalChange) -> Result<()> {
    let label_id_of = |kind: LabelKind| -> Result<LabelId> {
        tx.query_row(
            "SELECT id FROM labels WHERE account_id=?1 AND kind=?2 LIMIT 1",
            params![account.0, kind.as_key()],
            |r| Ok(LabelId(r.get(0)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::Invalid(format!("account has no {} label", kind.as_key())))
    };
    let add_label = |ids: &[MessageId], label: &LabelId| -> Result<()> {
        for id in ids {
            tx.execute("INSERT OR IGNORE INTO message_labels(message_id, label_id) VALUES(?1,?2)", params![id.0, label.0])?;
        }
        Ok(())
    };
    let remove_label = |ids: &[MessageId], label: &LabelId| -> Result<()> {
        for id in ids {
            tx.execute("DELETE FROM message_labels WHERE message_id=?1 AND label_id=?2", params![id.0, label.0])?;
        }
        Ok(())
    };
    match change {
        LocalChange::SetRead { message_ids, read } => {
            for id in message_ids {
                tx.execute("UPDATE messages SET is_read=?2 WHERE id=?1", params![id.0, read])?;
            }
        }
        LocalChange::SetStarred { message_ids, starred } => {
            for id in message_ids {
                tx.execute("UPDATE messages SET is_starred=?2 WHERE id=?1", params![id.0, starred])?;
            }
        }
        LocalChange::AddLabel { message_ids, label_id } => add_label(message_ids, label_id)?,
        LocalChange::RemoveLabel { message_ids, label_id } => remove_label(message_ids, label_id)?,
        LocalChange::Archive { message_ids } => remove_label(message_ids, &label_id_of(LabelKind::Inbox)?)?,
        LocalChange::Trash { message_ids } => {
            let trash = label_id_of(LabelKind::Trash)?;
            let inbox = label_id_of(LabelKind::Inbox)?;
            remove_label(message_ids, &inbox)?;
            add_label(message_ids, &trash)?;
        }
        LocalChange::Untrash { message_ids } => {
            let trash = label_id_of(LabelKind::Trash)?;
            let inbox = label_id_of(LabelKind::Inbox)?;
            remove_label(message_ids, &trash)?;
            add_label(message_ids, &inbox)?;
        }
    }
    Ok(())
}

fn insert_operation_tx(tx: &Connection, account: &AccountId, kind: OperationKind, ts: Timestamp) -> Result<PendingOperation> {
    let seq: i64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM pending_operations WHERE account_id=?1",
        [&account.0],
        |r| r.get(0),
    )?;
    let op = PendingOperation {
        id: OperationId::generate(),
        account_id: account.clone(),
        seq,
        kind,
        state: OperationState::Pending,
        attempts: 0,
        last_error: None,
        created_at: ts,
        updated_at: ts,
    };
    tx.execute(
        "INSERT INTO pending_operations(id, account_id, seq, kind_json, state, attempts, last_error, created_at, updated_at)
         VALUES(?1,?2,?3,?4,?5,0,NULL,?6,?6)",
        params![op.id.0, op.account_id.0, op.seq, serde_json::to_string(&op.kind)?, op.state.as_key(), ts],
    )?;
    Ok(op)
}

fn save_draft_tx(tx: &Connection, d: &Draft) -> Result<()> {
    tx.execute(
        "INSERT INTO drafts(id, account_id, kind, conversation_id, in_reply_to, to_json, cc_json, bcc_json, subject, body_text, body_html,
            state_json, created_at, updated_at, provider_id, include_quote)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
         ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, conversation_id=excluded.conversation_id, in_reply_to=excluded.in_reply_to,
            to_json=excluded.to_json, cc_json=excluded.cc_json, bcc_json=excluded.bcc_json, subject=excluded.subject,
            body_text=excluded.body_text, body_html=excluded.body_html, state_json=excluded.state_json,
            updated_at=excluded.updated_at, provider_id=excluded.provider_id, include_quote=excluded.include_quote",
        params![
            d.id.0,
            d.account_id.0,
            d.kind.as_key(),
            d.conversation_id.as_ref().map(|c| &c.0),
            d.in_reply_to.as_ref().map(|m| &m.0),
            serde_json::to_string(&d.to)?,
            serde_json::to_string(&d.cc)?,
            serde_json::to_string(&d.bcc)?,
            d.subject,
            d.body_text,
            d.body_html,
            serde_json::to_string(&d.state)?,
            d.created_at,
            d.updated_at,
            d.provider_id.as_ref().map(|p| &p.0),
            d.include_quote as i64,
        ],
    )?;
    tx.execute("DELETE FROM draft_attachments WHERE draft_id=?1", [&d.id.0])?;
    for a in &d.attachments {
        tx.execute(
            "INSERT INTO draft_attachments(id, draft_id, filename, mime_type, size_bytes, local_path) VALUES(?1,?2,?3,?4,?5,?6)",
            params![a.id.0, d.id.0, a.filename, a.mime_type, a.size_bytes as i64, a.local_path],
        )?;
    }
    Ok(())
}

fn load_drafts(c: &Connection, where_clause: &str, args: impl rusqlite::Params) -> Result<Vec<Draft>> {
    let sql = format!(
        "SELECT d.id, d.account_id, d.kind, d.conversation_id, d.in_reply_to, d.to_json, d.cc_json, d.bcc_json, d.subject, d.body_text,
                d.body_html, d.state_json, d.created_at, d.updated_at, d.provider_id, d.include_quote FROM drafts d {where_clause}"
    );
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
            r.get::<_, Option<String>>(10)?,
            r.get::<_, String>(11)?,
            r.get::<_, i64>(12)?,
            r.get::<_, i64>(13)?,
            r.get::<_, Option<String>>(14)?,
            r.get::<_, i64>(15)? != 0,
        ))
    })?;
    let mut drafts = Vec::new();
    for row in rows {
        let (id, account_id, kind, conv, reply, to, cc, bcc, subject, body_text, body_html, state, created, updated, provider, include_quote) = row?;
        drafts.push(Draft {
            id: DraftId(id),
            account_id: AccountId(account_id),
            kind: DraftKind::from_key(&kind).unwrap_or(DraftKind::New),
            conversation_id: conv.map(ConversationId),
            in_reply_to: reply.map(MessageId),
            to: serde_json::from_str(&to)?,
            cc: serde_json::from_str(&cc)?,
            bcc: serde_json::from_str(&bcc)?,
            subject,
            body_text,
            body_html,
            include_quote,
            attachments: Vec::new(),
            state: serde_json::from_str(&state)?,
            created_at: created,
            updated_at: updated,
            provider_id: provider.map(ProviderId),
        });
    }
    let mut stmt = c.prepare("SELECT id, draft_id, filename, mime_type, size_bytes, local_path FROM draft_attachments ORDER BY rowid")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            DraftAttachment {
                id: AttachmentId(r.get(0)?),
                filename: r.get(2)?,
                mime_type: r.get(3)?,
                size_bytes: r.get::<_, i64>(4)? as u64,
                local_path: r.get(5)?,
            },
        ))
    })? {
        let (_, draft_id, att) = row?;
        if let Some(d) = drafts.iter_mut().find(|d| d.id.0 == draft_id) {
            d.attachments.push(att);
        }
    }
    Ok(drafts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            id: AccountId::from("acct-1"),
            provider: ProviderKind::Sample,
            display_name: "Jason".into(),
            email: "jason@example.com".into(),
            created_at: 1,
        }
    }

    fn label(kind: LabelKind, name: &str) -> Label {
        Label {
            id: LabelId::from(format!("label-{}", kind.as_key()).as_str()),
            account_id: AccountId::from("acct-1"),
            kind,
            name: name.into(),
            provider_id: None,
            color: None,
            sort_order: 0,
        }
    }

    fn message(id: &str, subject: &str, body: &str, labels: &[&str]) -> Message {
        Message {
            id: MessageId::from(id),
            account_id: AccountId::from("acct-1"),
            conversation_id: ConversationId::from(format!("conv-{id}").as_str()),
            provider_id: None,
            from: Address::named("Maya Chen", "maya@northstar.example"),
            to: vec![Address::named("Jason", "jason@example.com")],
            cc: vec![],
            bcc: vec![],
            reply_to: None,
            subject: subject.into(),
            snippet: body.chars().take(40).collect(),
            body: Body { text: Some(body.into()), html: None },
            date: 1_000_000,
            is_read: false,
            is_starred: false,
            is_outgoing: false,
            label_ids: labels.iter().map(|l| LabelId::from(*l)).collect(),
            attachments: vec![],
            rfc_message_id: None,
            in_reply_to: None,
            category: None,
            is_important: false,
            focus_override: None,
            invitation: None,
        }
    }

    fn seeded(store: &Store) {
        store.upsert_account(&account()).unwrap();
        for (kind, name) in [
            (LabelKind::Inbox, "Inbox"),
            (LabelKind::Sent, "Sent"),
            (LabelKind::Trash, "Trash"),
            (LabelKind::User, "Projects"),
        ] {
            store.upsert_label(&label(kind, name)).unwrap();
        }
        store
            .upsert_messages(&[
                message("m1", "Launch creative — final review", "The updated launch materials are ready.", &["label-inbox", "label-user"]),
                message("m2", "Saturday plans?", "Coffee first, then a trail with a view?", &["label-inbox"]),
            ])
            .unwrap();
    }

    #[test]
    fn change_and_operation_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlook.sqlite");
        {
            let store = Store::open(&path).unwrap();
            seeded(&store);
            let op = store
                .apply_change(&AccountId::from("acct-1"), LocalChange::SetStarred { message_ids: vec![MessageId::from("m1")], starred: true })
                .unwrap();
            assert_eq!(op.seq, 1);
            store
                .apply_change(&AccountId::from("acct-1"), LocalChange::Archive { message_ids: vec![MessageId::from("m2")] })
                .unwrap();
        }
        // "Crash" and reopen.
        let store = Store::open(&path).unwrap();
        let acct = AccountId::from("acct-1");
        let ops = store.pending_operations(&acct).unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].seq, 1);
        assert!(matches!(ops[0].kind, OperationKind::SetStarred { starred: true, .. }));
        assert!(matches!(ops[1].kind, OperationKind::Archive { .. }));
        assert!(store.message(&MessageId::from("m1")).unwrap().unwrap().is_starred);
        let inbox = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), ListFilter::All, None).unwrap();
        assert_eq!(inbox.len(), 1, "archived message left the inbox");
        assert!(inbox[0].pending, "pending indicator is derived from the queue");
        let archive = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Archive), ListFilter::All, None).unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(store.account_status(&acct).unwrap().pending_operations, 2);
    }

    #[test]
    fn failed_change_leaves_no_operation() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        let acct = AccountId::from("acct-1");
        let err = store.apply_change(&acct, LocalChange::AddLabel { message_ids: vec![MessageId::from("m1")], label_id: LabelId::from("missing") });
        assert!(err.is_err(), "foreign key violation must fail the transaction");
        assert_eq!(store.pending_count(&acct).unwrap(), 0);
        assert!(!store.message(&MessageId::from("m1")).unwrap().unwrap().has_label(&LabelId::from("missing")));
    }

    #[test]
    fn draft_queue_and_cancel_are_atomic_and_durable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlook.sqlite");
        let acct = AccountId::from("acct-1");
        let draft_id;
        {
            let store = Store::open(&path).unwrap();
            seeded(&store);
            let mut draft = Draft::new(acct.clone(), DraftKind::New, 10);
            draft.to = vec![Address::named("Maya Chen", "maya@northstar.example")];
            draft.subject = "Agenda".into();
            draft.body_text = "A few things to cover.".into();
            store.save_draft(&draft).unwrap();
            draft_id = draft.id.clone();
            let op = store.queue_send(&draft).unwrap();
            assert!(matches!(op.kind, OperationKind::SendDraft { .. }));
        }
        let store = Store::open(&path).unwrap();
        let outbox = store.drafts(&acct, true).unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].state, DraftState::Queued);
        assert_eq!(store.drafts(&acct, false).unwrap().len(), 0);
        assert_eq!(store.account_status(&acct).unwrap().outbox_items, 1);

        let restored = store.cancel_send(&draft_id).unwrap();
        assert_eq!(restored.state, DraftState::Editing);
        assert_eq!(store.pending_count(&acct).unwrap(), 0);
        assert_eq!(store.drafts(&acct, false).unwrap().len(), 1);

        // Empty recipients are rejected before anything is written.
        let mut bad = restored.clone();
        bad.to.clear();
        assert!(store.queue_send(&bad).is_err());
        assert_eq!(store.pending_count(&acct).unwrap(), 0);
    }

    #[test]
    fn complete_send_stores_message_and_clears_draft() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        let acct = AccountId::from("acct-1");
        let mut draft = Draft::new(acct.clone(), DraftKind::New, 10);
        draft.to = vec![Address::named("Maya Chen", "maya@northstar.example")];
        draft.subject = "Agenda".into();
        store.save_draft(&draft).unwrap();
        let op = store.queue_send(&draft).unwrap();
        let mut sent = message("sent-1", "Agenda", "body", &["label-sent"]);
        sent.from = Address::named("Jason", "jason@example.com");
        sent.to = vec![Address::named("Maya Chen", "maya@northstar.example")];
        sent.is_outgoing = true;
        sent.is_read = true;
        store.complete_send(&draft.id, &op.id, &sent).unwrap();
        assert!(store.draft(&draft.id).unwrap().is_none());
        assert_eq!(store.pending_count(&acct).unwrap(), 0);
        let sent_view = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Sent), ListFilter::All, None).unwrap();
        assert_eq!(sent_view.len(), 1);
        assert_eq!(sent_view[0].participant_line(), "Maya Chen", "sent conversations name the recipients");
    }

    #[test]
    fn search_and_coverage() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        let acct = AccountId::from("acct-1");
        let hits = store.search(&acct, "launch mater").unwrap();
        assert_eq!(hits, vec![MessageId::from("m1")]);
        let hits = store.search(&acct, "maya").unwrap();
        assert_eq!(hits.len(), 2, "sender names are indexed");
        assert!(store.search(&acct, "   ").unwrap().is_empty());
        let mut headers_only = message("m3", "Receipt", "", &["label-inbox"]);
        headers_only.body = Body::default();
        store.upsert_messages(&[headers_only]).unwrap();
        let coverage = store.search_coverage(&acct).unwrap();
        assert_eq!(coverage.messages_total, 3);
        assert_eq!(coverage.bodies_cached, 2);
        assert!(coverage.sentence().contains("1 have headers only"));
    }

    #[test]
    fn settings_round_trip_and_recipients() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        store.set_setting("appearance", &AppearancePreference::Dark).unwrap();
        assert_eq!(store.setting::<AppearancePreference>("appearance").unwrap(), Some(AppearancePreference::Dark));
        assert_eq!(store.setting::<AppearancePreference>("nope").unwrap(), None);
        let recipients = store.known_recipients(&AccountId::from("acct-1"), "ma", 5).unwrap();
        assert_eq!(recipients[0].email, "maya@northstar.example");
    }

    #[test]
    fn remote_merge_respects_pending_intent_and_keeps_bodies() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        let acct = AccountId::from("acct-1");
        // Local change pending: unread message starred by the user.
        store.apply_change(&acct, LocalChange::SetStarred { message_ids: vec![MessageId::from("m1")], starred: true }).unwrap();
        let protected = store.pending_message_ids(&acct).unwrap();
        // Remote says m1 is read, not starred, and only sends headers; m2 was deleted remotely.
        let mut remote = message("m1", "Launch creative — final review", "", &["label-inbox"]);
        remote.body = Body::default();
        remote.is_read = true;
        remote.is_starred = false;
        store.merge_remote_messages(&[remote], &protected).unwrap();
        let m1 = store.message(&MessageId::from("m1")).unwrap().unwrap();
        assert!(m1.is_starred, "pending local intent wins");
        assert!(!m1.is_read);
        assert!(m1.body.is_cached(), "existing body kept when remote sends headers only");
        assert_eq!(store.delete_messages(&[MessageId::from("m2")]).unwrap(), 1);
        assert!(store.message(&MessageId::from("m2")).unwrap().is_none());

        // Unprotected merge takes remote flags.
        let mut remote = message("m1", "Launch creative — final review", "new body", &["label-inbox"]);
        remote.is_read = true;
        remote.is_starred = false;
        store.merge_remote_messages(&[remote], &BTreeSet::new()).unwrap();
        let m1 = store.message(&MessageId::from("m1")).unwrap().unwrap();
        assert!(m1.is_read && !m1.is_starred);
        assert_eq!(m1.body.text.as_deref(), Some("new body"));

        store.replace_labels(&acct, &[label(LabelKind::Inbox, "Inbox")]).unwrap();
        assert_eq!(store.labels(&acct).unwrap().len(), 1);
        assert!(store.account_by_email(ProviderKind::Sample, "JASON@example.com").unwrap().is_some());
        store.delete_account(&acct).unwrap();
        assert!(store.accounts().unwrap().is_empty());
        assert_eq!(store.message_count(&acct).unwrap(), 0);
    }

    #[test]
    fn focused_inbox_splits_and_respects_rules() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        let acct = AccountId::from("acct-1");
        let mut promo = message("m3", "Big sale", "Everything must go", &["label-inbox"]);
        promo.from = Address::named("Shop", "deals@shop.example");
        promo.category = Some("promotions".into());
        store.upsert_messages(&[promo]).unwrap();
        let focused = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), ListFilter::All, Some(FocusTab::Focused)).unwrap();
        let other = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), ListFilter::All, Some(FocusTab::Other)).unwrap();
        assert_eq!(focused.len(), 2);
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].subject, "Big sale");
        store.set_focus_rule(&acct, "DEALS@shop.example", Some(true)).unwrap();
        let other = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), ListFilter::All, Some(FocusTab::Other)).unwrap();
        assert!(other.is_empty(), "rule moves the sender to Focused");
        store.set_focus_rule(&acct, "deals@shop.example", None).unwrap();
        store.set_focus_override(&[MessageId::from("m1")], Some(false)).unwrap();
        let other = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), ListFilter::All, Some(FocusTab::Other)).unwrap();
        assert_eq!(other.len(), 2);
        assert_eq!(store.view_count(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), Some(FocusTab::Focused)).unwrap(), 1);
    }

    #[test]
    fn series_edits_and_deletes_follow_the_scope() {
        let store = Store::open_in_memory().unwrap();
        let acct = AccountId::from("acct");
        store.upsert_account(&Account { id: acct.clone(), provider: ProviderKind::Sample, display_name: "Me".into(), email: "me@example.com".into(), created_at: 0 }).unwrap();
        let cal = Calendar { id: CalendarId::from("acct:cal"), account_id: acct.clone(), provider_id: None, name: "Cal".into(), color: None, is_primary: true, can_edit: true, visible: true, sort_order: 0, default_reminders: vec![] };
        store.replace_calendars(&acct, &[cal.clone()]).unwrap();
        let day = 86_400_000;
        let instance = |n: i64| Event {
            id: EventId(format!("acct:cal:m_{n}")), account_id: acct.clone(), calendar_id: cal.id.clone(), provider_id: Some(ProviderId(format!("m_{n}"))), ical_uid: None,
            summary: "Standup".into(), description: None, location: None, start: n * day + 9 * 3_600_000, end: n * day + 9 * 3_600_000 + 1_800_000, all_day: false,
            status: EventStatus::Confirmed, organizer: None, attendees: vec![], html_link: None, conference_url: None, recurring_event_id: Some("m".into()), updated: 0,
            event_type: None, color: None, reminders: None, original_start: Some(n * day + 9 * 3_600_000), wants_conference: false, pending: false,
        };
        store.upsert_events(&[instance(1), instance(2), instance(3), instance(4)]).unwrap();
        // Rename and shift the third one by an hour, for it and everything after.
        let mut edited = instance(3);
        edited.summary = "Sync".into();
        edited.start += 3_600_000;
        edited.end += 3_600_000;
        let op = store.apply_series_change(&acct, &edited, RecurrenceScope::Following).unwrap();
        assert!(matches!(op.kind, OperationKind::UpdateEvent { scope: RecurrenceScope::Following, .. }));
        let all = store.events_between(&acct, 0, 10 * day).unwrap();
        let by = |n: i64| all.iter().find(|e| e.provider_id.as_ref().unwrap().0 == format!("m_{n}")).unwrap().clone();
        assert_eq!(by(2).summary, "Standup");
        assert_eq!(by(4).summary, "Sync");
        assert_eq!(by(4).start, instance(4).start + 3_600_000);
        // Delete all: every occurrence goes, one operation carries the series id.
        let op = store.delete_event_locally_scoped(&acct, &by(2), RecurrenceScope::All).unwrap();
        assert!(matches!(op.kind, OperationKind::DeleteEvent { scope: RecurrenceScope::All, recurring_event_id: Some(_), .. }));
        assert!(store.events_between(&acct, 0, 10 * day).unwrap().is_empty());
    }

    #[test]
    fn calendar_round_trip_and_pending_flags() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        let acct = AccountId::from("acct-1");
        let cal = Calendar {
            id: CalendarId::from("acct-1:cal:primary"),
            account_id: acct.clone(),
            provider_id: Some(ProviderId::from("primary")),
            name: "Jason".into(),
            color: Some("#9fe1e7".into()),
            is_primary: true,
            can_edit: true,
            visible: true,
            sort_order: 0,
            default_reminders: vec![10],
        };
        store.replace_calendars(&acct, &[cal.clone()]).unwrap();
        store.set_calendar_visible(&cal.id, false).unwrap();
        let mut hidden = cal.clone();
        hidden.name = "Jason Alexander".into();
        store.replace_calendars(&acct, &[hidden]).unwrap();
        assert!(!store.calendars(&acct).unwrap()[0].visible, "visibility survives provider refresh");
        store.set_calendar_visible(&cal.id, true).unwrap();
        let event = Event {
            id: EventId::from("acct-1:cal:primary:e1"),
            account_id: acct.clone(),
            calendar_id: cal.id.clone(),
            provider_id: Some(ProviderId::from("e1")),
            ical_uid: Some("e1@google.com".into()),
            summary: "Design review".into(),
            description: None,
            location: Some("Room 4".into()),
            start: 1_000_000,
            end: 1_000_000 + 3_600_000,
            all_day: false,
            status: EventStatus::Confirmed,
            organizer: Some(Address::named("Maya Chen", "maya@northstar.example")),
            attendees: vec![Attendee { email: "jason@example.com".into(), name: None, response: ResponseStatus::NeedsAction, is_self: true, is_organizer: false, optional: false }],
            html_link: None,
            conference_url: None,
            recurring_event_id: None,
            updated: 0,
            event_type: None,
            color: None,
            reminders: None,
            original_start: None,
            wants_conference: false,
            pending: false,
        };
        store.upsert_events(&[event.clone()]).unwrap();
        assert_eq!(store.event_by_ical_uid(&acct, "e1@google.com").unwrap().unwrap().id, event.id);
        assert_eq!(store.events_starting_between(&acct, 0, 2_000_000).unwrap().len(), 1);
        assert_eq!(store.calendars(&acct).unwrap()[0].default_reminders, vec![10]);
        let found = store.events_between(&acct, 0, 2_000_000).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].is_invitation());
        assert!(store.events_between(&acct, 5_000_000, 6_000_000).unwrap().is_empty());
        let mut accepted = event.clone();
        accepted.attendees[0].response = ResponseStatus::Accepted;
        store.apply_event_change(&acct, &accepted, OperationKind::RespondToEvent { event_id: event.id.clone(), response: ResponseStatus::Accepted }).unwrap();
        let found = store.events_between(&acct, 0, 2_000_000).unwrap();
        assert_eq!(found[0].my_response(), Some(ResponseStatus::Accepted));
        assert!(found[0].pending);
        store.delete_event_locally(&acct, &event).unwrap();
        assert!(store.pending_event_ids(&acct).unwrap().contains(&event.id), "a locally deleted event is protected until replayed");
        assert!(store.event(&event.id).unwrap().is_none());
        assert_eq!(store.pending_count(&acct).unwrap(), 2);
    }

    #[test]
    fn trash_and_untrash_round_trip() {
        let store = Store::open_in_memory().unwrap();
        seeded(&store);
        let acct = AccountId::from("acct-1");
        store.apply_change(&acct, LocalChange::Trash { message_ids: vec![MessageId::from("m1")] }).unwrap();
        let trash = store.conversations(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Trash), ListFilter::All, None).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(store.view_count(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), None).unwrap(), 1);
        store.apply_change(&acct, LocalChange::Untrash { message_ids: vec![MessageId::from("m1")] }).unwrap();
        assert_eq!(store.view_count(&acct, "jason@example.com", &MailboxView::Kind(LabelKind::Inbox), None).unwrap(), 2);
    }
}
