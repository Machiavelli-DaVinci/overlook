//! `MailProvider` for IMAP accounts (plan milestone M2: the read path).
//!
//! - `connection`: TLS / STARTTLS / plaintext-to-localhost sessions and the error mapping.
//! - `folders`: `LIST` results as [`Label`] rows.
//! - `structure`: `BODYSTRUCTURE`-driven partial download of large messages.
//! - `uidset`: sequence sets for the cursors.
//!
//! One connection per account serves every call, under a mutex; a dropped connection is
//! reopened once before the call reports [`ProviderError::Offline`]. The checkpoint the sync
//! engine stores is the JSON of all folder cursors ([`Checkpoint`]): per folder its
//! `UIDVALIDITY`, the highest UID seen, and the UIDs Overlook holds with their `\Seen` and
//! `\Flagged` state, so `fetch_changes` can find new mail, flag changes and deletions with
//! a `UID SEARCH ALL` and a `UID FETCH (FLAGS)` per folder, without server extensions.
//!
//! Message identity, from the plan: local ids are `<account>:<uuid>` minted on first sight
//! (the sync engine adopts the stored id for a provider id already on disk); `provider_id`
//! is `<folder>\u{1f}<uidvalidity>\u{1f}<uid>`.
//!
//! Mailbox writes (`apply`), sending and send reconciliation are milestone M3 and return
//! `Rejected` here.

pub mod connection;
pub mod folders;
pub mod structure;
pub mod uidset;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;

use crate::mime::{headers_from_rfc822, message_from_rfc822, MessageFlags};
use crate::{AppliedChanges, BoxFuture, MailProvider, MailboxSnapshot, ProviderError, RemoteChanges, SendContext, SendOutcome, SentReceipt};
use connection::{map_error, Session};
use folders::FolderInfo;
use imap::types::Flag;
use imap_proto::types::{MessageSection, SectionPath};
use overlook_core::time::now;
use overlook_core::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, warn};

/// Socket connect/read/write timeout. A hung server must not stall the sync thread.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the folder list is re-read.
const LIST_INTERVAL_MS: i64 = 60 * 60 * 1000;

/// Every nth change cycle refreshes flags in folders other than the Inbox.
const FLAG_REFRESH_EVERY: u64 = 10;

/// UIDs per `UID FETCH` command.
const FETCH_CHUNK: usize = 400;

const SEP: char = '\u{1f}';

// ------------------------------------------------------------------ checkpoint

/// The sync cursor for one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderCursor {
    /// `LabelKind` key of the label this folder maps to.
    pub kind: String,
    pub uidvalidity: u32,
    /// Highest UID ever seen in this folder; new mail has a higher UID.
    pub highest_uid: u32,
    /// UIDs Overlook holds (sequence set).
    pub known: String,
    /// Subset of `known` that carried `\Seen` when last checked.
    pub seen: String,
    /// Subset of `known` that carried `\Flagged` when last checked.
    pub flagged: String,
    pub delimiter: Option<String>,
    pub special_use: Option<String>,
    pub subscribed: bool,
    /// Listed but not yet selected (added between folder listings).
    pub fresh: bool,
}

impl FolderCursor {
    fn label_kind(&self) -> LabelKind {
        LabelKind::from_key(&self.kind).unwrap_or(LabelKind::User)
    }

    fn known_set(&self) -> BTreeSet<u32> {
        uidset::decode(&self.known)
    }

    fn set_flags(&mut self, known: &BTreeSet<u32>, seen: &BTreeSet<u32>, flagged: &BTreeSet<u32>) {
        self.known = uidset::encode(known);
        self.seen = uidset::encode(&seen.intersection(known).copied().collect());
        self.flagged = uidset::encode(&flagged.intersection(known).copied().collect());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Checkpoint {
    pub version: u32,
    /// When the folder list was last read (ms since the epoch).
    pub listed_at: i64,
    /// Change cycles run so far; drives the flag refresh cadence.
    pub cycle: u64,
    pub folders: BTreeMap<String, FolderCursor>,
}

impl Checkpoint {
    pub const VERSION: u32 = 1;

    pub fn parse(text: &str) -> Result<Self, ProviderError> {
        let cp: Checkpoint = serde_json::from_str(text).map_err(|_| ProviderError::CheckpointExpired)?;
        if cp.version != Self::VERSION {
            return Err(ProviderError::CheckpointExpired);
        }
        Ok(cp)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// The folder rows the app mirrors into `imap_folders`, from a checkpoint.
pub fn folder_states(checkpoint: &str) -> Vec<ImapFolder> {
    let Ok(cp) = Checkpoint::parse(checkpoint) else { return Vec::new() };
    cp.folders
        .into_iter()
        .map(|(path, c)| ImapFolder { path, uidvalidity: (c.uidvalidity != 0).then_some(c.uidvalidity), highest_uid: c.highest_uid, highest_modseq: None, delimiter: c.delimiter, special_use: c.special_use, subscribed: c.subscribed })
        .collect()
}

// -------------------------------------------------------------- message refs

/// What a message's provider id names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageRef {
    pub folder: String,
    pub uidvalidity: u32,
    pub uid: u32,
}

impl MessageRef {
    pub fn parse(id: &ProviderId) -> Option<Self> {
        let mut parts = id.0.rsplitn(3, SEP);
        let uid = parts.next()?.parse().ok()?;
        let uidvalidity = parts.next()?.parse().ok()?;
        let folder = parts.next()?.to_owned();
        Some(Self { folder, uidvalidity, uid })
    }

    pub fn provider_id(folder: &str, uidvalidity: u32, uid: u32) -> ProviderId {
        ProviderId(format!("{folder}{SEP}{uidvalidity}{SEP}{uid}"))
    }
}

// ------------------------------------------------------------------ provider

#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub special_use: bool,
    pub move_: bool,
    pub uidplus: bool,
    pub condstore: bool,
    pub qresync: bool,
    pub idle: bool,
}

struct Conn {
    session: Option<Session>,
    /// The folder currently examined, with its UIDVALIDITY.
    selected: Option<(String, u32)>,
    caps: Capabilities,
}

impl Conn {
    fn session(&mut self) -> &mut Session {
        self.session.as_mut().expect("with_session connects before use")
    }

    /// `EXAMINE`s the folder (read-only) and returns its state. Always issued, since the
    /// callers want a fresh `UIDVALIDITY`.
    fn examine(&mut self, path: &str) -> Result<imap::types::Mailbox, ProviderError> {
        self.selected = None;
        let mailbox = self.session().examine(path).map_err(|e| map_error(&e))?;
        self.selected = Some((path.to_owned(), mailbox.uid_validity.unwrap_or(0)));
        Ok(mailbox)
    }

    /// Makes `path` the examined folder, skipping the round trip when it already is.
    /// Returns `false` when its `UIDVALIDITY` is not `uidvalidity` any more.
    fn examine_for(&mut self, path: &str, uidvalidity: u32) -> Result<bool, ProviderError> {
        if self.selected.as_ref().is_some_and(|(p, v)| p == path && *v == uidvalidity) {
            return Ok(true);
        }
        match self.examine(path) {
            Ok(_) => Ok(self.selected.as_ref().is_some_and(|(_, v)| *v == uidvalidity)),
            // The folder is gone: its messages are stale; the next change cycle cleans up.
            Err(ProviderError::Rejected(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn uid_search_all(&mut self) -> Result<BTreeSet<u32>, ProviderError> {
        Ok(self.session().uid_search("ALL").map_err(|e| map_error(&e))?.into_iter().collect())
    }

    /// `(UID FLAGS)` for a set, as `(seen, flagged)`, plus every UID the server answered for.
    fn flags_for(&mut self, uids: &BTreeSet<u32>) -> Result<(BTreeSet<u32>, BTreeSet<u32>, BTreeSet<u32>), ProviderError> {
        let (mut present, mut seen, mut flagged) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for chunk in uidset::chunks(uids, FETCH_CHUNK) {
            let fetches = self.session().uid_fetch(&chunk, "(UID FLAGS)").map_err(|e| map_error(&e))?;
            for f in fetches.iter() {
                let Some(uid) = f.uid else { continue };
                present.insert(uid);
                if f.flags().contains(&Flag::Seen) {
                    seen.insert(uid);
                }
                if f.flags().contains(&Flag::Flagged) {
                    flagged.insert(uid);
                }
            }
        }
        Ok((present, seen, flagged))
    }

    fn list_folders(&mut self) -> Result<Vec<FolderInfo>, ProviderError> {
        let names = self.session().list(Some(""), Some("*")).map_err(|e| map_error(&e))?;
        let mut folders: Vec<FolderInfo> = names.iter().map(FolderInfo::from_name).collect();
        // Subscriptions are informational; a server without LSUB support is not an error.
        if let Ok(subscribed) = self.session().lsub(Some(""), Some("*")) {
            let set: HashSet<String> = subscribed.iter().map(|n| n.name().to_owned()).collect();
            for f in &mut folders {
                f.subscribed = set.contains(&f.path);
            }
        }
        Ok(folders)
    }
}

pub struct ImapProvider {
    servers: MailServers,
    password: String,
    inbox_limit: usize,
    timeout: Duration,
    conn: Mutex<Conn>,
}

impl std::fmt::Debug for ImapProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The password is deliberately absent.
        f.debug_struct("ImapProvider").field("incoming", &self.servers.incoming).field("inbox_limit", &self.inbox_limit).finish()
    }
}

impl ImapProvider {
    pub fn new(servers: MailServers, password: impl Into<String>) -> Self {
        Self { servers, password: password.into(), inbox_limit: 100, timeout: DEFAULT_TIMEOUT, conn: Mutex::new(Conn { session: None, selected: None, caps: Capabilities::default() }) }
    }

    /// Newest Inbox messages to list on first download; other folders scale with it, with
    /// the same split the Gmail provider uses.
    pub fn with_inbox_limit(mut self, inbox_limit: usize) -> Self {
        self.inbox_limit = inbox_limit.max(20);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn servers(&self) -> &MailServers {
        &self.servers
    }

    /// Server extensions seen at login, once connected.
    pub fn capabilities(&self) -> Capabilities {
        self.conn.lock().map(|c| c.caps).unwrap_or_default()
    }

    fn folder_limit(&self, kind: LabelKind) -> usize {
        let scale = self.inbox_limit as f32 / 100.;
        let base: usize = match kind {
            LabelKind::Inbox => return self.inbox_limit,
            LabelKind::Sent | LabelKind::Archive => 40,
            LabelKind::Trash | LabelKind::User => 20,
            LabelKind::Spam => 10,
            _ => 20,
        };
        ((base as f32) * scale).round().max(10.) as usize
    }

    fn ensure_connected(&self, conn: &mut Conn) -> Result<(), ProviderError> {
        if conn.session.is_some() {
            return Ok(());
        }
        let mut session = connection::connect(&self.servers.incoming, &self.password, self.timeout)?;
        let caps = session.capabilities().map_err(|e| map_error(&e))?;
        conn.caps = Capabilities {
            special_use: caps.has_str("SPECIAL-USE"),
            move_: caps.has_str("MOVE"),
            uidplus: caps.has_str("UIDPLUS"),
            condstore: caps.has_str("CONDSTORE"),
            qresync: caps.has_str("QRESYNC"),
            idle: caps.has_str("IDLE"),
        };
        conn.session = Some(session);
        conn.selected = None;
        Ok(())
    }

    /// Runs `f` on the account's connection, opening it when needed. A connection that was
    /// reused and turns out dead is reopened once; the closure must therefore be safe to run
    /// twice (build results locally, return them).
    fn with_session<T>(&self, mut f: impl FnMut(&mut Conn) -> Result<T, ProviderError>) -> Result<T, ProviderError> {
        let mut guard = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let reused = guard.session.is_some();
        self.ensure_connected(&mut guard)?;
        let drop_session = |g: &mut Conn| {
            g.session = None;
            g.selected = None;
        };
        match f(&mut guard) {
            Err(ProviderError::Offline) if reused => {
                drop_session(&mut guard);
                self.ensure_connected(&mut guard)?;
                let result = f(&mut guard);
                if matches!(result, Err(ProviderError::Offline)) {
                    drop_session(&mut guard);
                }
                result
            }
            Err(ProviderError::Offline) => {
                drop_session(&mut guard);
                Err(ProviderError::Offline)
            }
            other => other,
        }
    }

    /// Closes the connection politely. The next call reconnects.
    pub fn disconnect(&self) {
        if let Ok(mut guard) = self.conn.lock() {
            if let Some(mut s) = guard.session.take() {
                let _ = s.logout();
            }
            guard.selected = None;
        }
    }

    fn local_id(account: &Account) -> MessageId {
        MessageId(format!("{}:{}", account.id.0, MessageId::generate().0))
    }

    fn flags_of(fetch: &imap::types::Fetch<'_>, label: &LabelId, provider_id: ProviderId) -> MessageFlags {
        MessageFlags {
            provider_id: Some(provider_id),
            label_ids: [label.clone()].into_iter().collect(),
            is_read: fetch.flags().contains(&Flag::Seen),
            is_starred: fetch.flags().contains(&Flag::Flagged),
            received_at: fetch.internal_date().map(|d| d.timestamp_millis()),
        }
    }

    /// Header-only rows for `uids` in the examined folder (`BODY.PEEK[HEADER]`), newest
    /// first. Also reports each UID's flags.
    fn fetch_headers(conn: &mut Conn, account: &Account, path: &str, uidvalidity: u32, uids: &BTreeSet<u32>) -> Result<(Vec<Message>, BTreeSet<u32>, BTreeSet<u32>), ProviderError> {
        let label = folders::label_id(&account.id, path);
        let mut rows = Vec::new();
        let (mut seen, mut flagged) = (BTreeSet::new(), BTreeSet::new());
        for chunk in uidset::chunks(uids, FETCH_CHUNK / 4) {
            let fetches = conn.session().uid_fetch(&chunk, "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])").map_err(|e| map_error(&e))?;
            for f in fetches.iter() {
                let Some(uid) = f.uid else { continue };
                if f.flags().contains(&Flag::Seen) {
                    seen.insert(uid);
                }
                if f.flags().contains(&Flag::Flagged) {
                    flagged.insert(uid);
                }
                let pid = MessageRef::provider_id(path, uidvalidity, uid);
                let flags = Self::flags_of(f, &label, pid);
                rows.push(headers_from_rfc822(account, f.header().unwrap_or_default(), Self::local_id(account), &flags));
            }
        }
        rows.sort_by(|a, b| b.date.cmp(&a.date));
        Ok((rows, seen, flagged))
    }

    /// Full messages for `uids` in the examined folder. Large messages take the
    /// `BODYSTRUCTURE` path. UIDs the server no longer has are skipped.
    fn fetch_bodies(conn: &mut Conn, account: &Account, path: &str, uidvalidity: u32, uids: &BTreeSet<u32>) -> Result<Vec<Message>, ProviderError> {
        let label = folders::label_id(&account.id, path);
        let mut out = Vec::new();
        let mut sizes: HashMap<u32, u32> = HashMap::new();
        for chunk in uidset::chunks(uids, FETCH_CHUNK) {
            let fetches = conn.session().uid_fetch(&chunk, "(UID RFC822.SIZE)").map_err(|e| map_error(&e))?;
            for f in fetches.iter() {
                if let Some(uid) = f.uid {
                    sizes.insert(uid, f.size.unwrap_or(0));
                }
            }
        }
        let small: BTreeSet<u32> = sizes.iter().filter(|(_, s)| **s < structure::LARGE_MESSAGE_BYTES).map(|(u, _)| *u).collect();
        let large: Vec<u32> = sizes.iter().filter(|(_, s)| **s >= structure::LARGE_MESSAGE_BYTES).map(|(u, _)| *u).collect();
        for chunk in uidset::chunks(&small, 20) {
            let fetches = conn.session().uid_fetch(&chunk, "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])").map_err(|e| map_error(&e))?;
            for f in fetches.iter() {
                let (Some(uid), Some(body)) = (f.uid, f.body()) else { continue };
                let flags = Self::flags_of(f, &label, MessageRef::provider_id(path, uidvalidity, uid));
                out.push(message_from_rfc822(account, body, Self::local_id(account), &flags));
            }
        }
        for uid in large {
            if let Some(m) = Self::fetch_large(conn, account, path, uidvalidity, uid, &label)? {
                out.push(m);
            }
        }
        Ok(out)
    }

    fn fetch_large(conn: &mut Conn, account: &Account, path: &str, uidvalidity: u32, uid: u32, label: &LabelId) -> Result<Option<Message>, ProviderError> {
        let fetches = conn.session().uid_fetch(uid.to_string(), "(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE BODY.PEEK[HEADER])").map_err(|e| map_error(&e))?;
        let Some(f) = fetches.iter().find(|f| f.uid == Some(uid)) else { return Ok(None) };
        let header = f.header().unwrap_or_default().to_vec();
        let flags = Self::flags_of(f, label, MessageRef::provider_id(path, uidvalidity, uid));
        let Some(bs) = f.bodystructure() else {
            warn!("imap: {path} uid {uid} is large and has no BODYSTRUCTURE; storing headers only");
            return Ok(Some(headers_from_rfc822(account, &header, Self::local_id(account), &flags)));
        };
        let plan = structure::plan(bs);
        let sections = plan.text_sections();
        let mut parts: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        if !sections.is_empty() {
            let query = if plan.single_part {
                format!("(UID BODY.PEEK[TEXT]<0.{}>)", structure::TEXT_PART_LIMIT)
            } else {
                let items: Vec<String> = sections.iter().map(|s| format!("BODY.PEEK[{s}]<0.{}> BODY.PEEK[{s}.MIME]", structure::TEXT_PART_LIMIT)).collect();
                format!("(UID {})", items.join(" "))
            };
            let fetched = conn.session().uid_fetch(uid.to_string(), query).map_err(|e| map_error(&e))?;
            if let Some(f) = fetched.iter().find(|f| f.uid == Some(uid)) {
                if plan.single_part {
                    parts.push((Vec::new(), f.text().unwrap_or_default().to_vec()));
                } else {
                    for s in &sections {
                        let section_path: Vec<u32> = s.split('.').filter_map(|n| n.parse().ok()).collect();
                        let body = f.section(&SectionPath::Part(section_path.clone(), None)).unwrap_or_default().to_vec();
                        let mime = f.section(&SectionPath::Part(section_path, Some(MessageSection::Mime))).unwrap_or_default().to_vec();
                        parts.push((mime, body));
                    }
                }
            }
        }
        let bytes = structure::synthetic_message(&header, &parts, plan.single_part);
        let local_id = Self::local_id(account);
        let mut message = message_from_rfc822(account, &bytes, local_id.clone(), &flags);
        message.attachments = plan
            .attachments
            .iter()
            .enumerate()
            .map(|(n, a)| Attachment {
                id: AttachmentId(format!("{}:att-{n}", local_id.0)),
                message_id: local_id.clone(),
                filename: a.filename.clone().unwrap_or_else(|| format!("attachment-{}", n + 1)),
                mime_type: a.mime.clone(),
                size_bytes: a.size,
                local_path: None,
                provider_id: Some(ProviderId(a.section.clone())),
                content_id: a.content_id.clone(),
            })
            .collect();
        Ok(Some(message))
    }

    /// Lists folders, builds labels and resets `cp.folders` to match (keeping cursors of
    /// folders that are still there). Returns the labels and the provider ids of messages in
    /// folders that disappeared.
    fn relist(&self, conn: &mut Conn, account: &Account, cp: &mut Checkpoint) -> Result<(Vec<Label>, Vec<ProviderId>), ProviderError> {
        let listed = conn.list_folders()?;
        let classified = folders::classify(&listed, self.servers.imap.as_ref());
        let labels = folders::labels_for(&account.id, &classified);
        let mut deleted = Vec::new();
        let mut next: BTreeMap<String, FolderCursor> = BTreeMap::new();
        for (info, kind) in &classified {
            let mut cursor = cp.folders.remove(&info.path).unwrap_or_else(|| FolderCursor { fresh: true, ..Default::default() });
            cursor.kind = kind.as_key().to_owned();
            cursor.delimiter = info.delimiter.clone();
            cursor.special_use = info.special_use();
            cursor.subscribed = info.subscribed;
            next.insert(info.path.clone(), cursor);
        }
        for (path, gone) in std::mem::take(&mut cp.folders) {
            deleted.extend(gone.known_set().into_iter().map(|uid| MessageRef::provider_id(&path, gone.uidvalidity, uid)));
        }
        cp.folders = next;
        cp.listed_at = now();
        Ok((labels, deleted))
    }

    /// Selects the newest N of a folder as its cursor and returns their provider ids,
    /// newest first.
    fn reset_folder(&self, conn: &mut Conn, path: &str, cursor: &mut FolderCursor, uidvalidity: u32, server: &BTreeSet<u32>) -> Result<Vec<ProviderId>, ProviderError> {
        let limit = self.folder_limit(cursor.label_kind());
        let keep: BTreeSet<u32> = server.iter().rev().take(limit).copied().collect();
        let (_, seen, flagged) = if keep.is_empty() { Default::default() } else { conn.flags_for(&keep)? };
        cursor.uidvalidity = uidvalidity;
        cursor.highest_uid = server.iter().next_back().copied().unwrap_or(0);
        cursor.set_flags(&keep, &seen, &flagged);
        cursor.fresh = false;
        Ok(keep.iter().rev().map(|uid| MessageRef::provider_id(path, uidvalidity, *uid)).collect())
    }

    fn snapshot(&self, account: &Account) -> Result<MailboxSnapshot, ProviderError> {
        self.with_session(|conn| {
            let mut cp = Checkpoint { version: Checkpoint::VERSION, ..Default::default() };
            let (labels, _) = self.relist(conn, account, &mut cp)?;
            let mut ids: Vec<ProviderId> = Vec::new();
            // Inbox first so the first bodies downloaded are the ones shown first.
            let mut order: Vec<String> = cp.folders.keys().cloned().collect();
            order.sort_by_key(|p| labels.iter().position(|l| l.provider_id.as_ref().map(|x| x.0.as_str()) == Some(p.as_str())).unwrap_or(usize::MAX));
            for path in order {
                let Some(cursor) = cp.folders.get_mut(&path) else { continue };
                let mailbox = match conn.examine(&path) {
                    Ok(m) => m,
                    Err(ProviderError::Rejected(reason)) => {
                        warn!("imap: skipping folder {path}: {reason}");
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let uidvalidity = mailbox.uid_validity.unwrap_or(0);
                let server = if mailbox.exists == 0 { BTreeSet::new() } else { conn.uid_search_all()? };
                ids.extend(self.reset_folder(conn, &path, cursor, uidvalidity, &server)?);
            }
            info!("imap: {} folders, {} recent messages queued for download", cp.folders.len(), ids.len());
            Ok(MailboxSnapshot { labels, messages: Vec::new(), message_ids: ids, checkpoint: Some(cp.to_json()) })
        })
    }

    fn changes(&self, account: &Account, checkpoint: &str) -> Result<RemoteChanges, ProviderError> {
        let base = Checkpoint::parse(checkpoint)?;
        self.with_session(|conn| {
            let mut cp = base.clone();
            cp.cycle += 1;
            let mut changes = RemoteChanges::default();
            if cp.folders.is_empty() || now() - cp.listed_at > LIST_INTERVAL_MS {
                let (labels, deleted) = self.relist(conn, account, &mut cp)?;
                changes.labels = Some(labels);
                changes.deleted_provider_ids.extend(deleted);
            }
            let mut vanished: Vec<String> = Vec::new();
            let mut relist_needed = false;
            let paths: Vec<String> = cp.folders.keys().cloned().collect();
            for path in paths {
                let cursor = cp.folders.get_mut(&path).expect("present");
                let mailbox = match conn.examine(&path) {
                    Ok(m) => m,
                    Err(ProviderError::Rejected(reason)) => {
                        info!("imap: folder {path} is gone ({reason}); removing its messages");
                        changes.deleted_provider_ids.extend(cursor.known_set().into_iter().map(|uid| MessageRef::provider_id(&path, cursor.uidvalidity, uid)));
                        vanished.push(path.clone());
                        relist_needed = true;
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let uidvalidity = mailbox.uid_validity.unwrap_or(0);
                let server = if mailbox.exists == 0 { BTreeSet::new() } else { conn.uid_search_all()? };
                if cursor.fresh || cursor.uidvalidity != uidvalidity {
                    if !cursor.fresh {
                        info!("imap: UIDVALIDITY of {path} changed; re-listing the folder");
                        changes.deleted_provider_ids.extend(cursor.known_set().into_iter().map(|uid| MessageRef::provider_id(&path, cursor.uidvalidity, uid)));
                    }
                    changes.message_ids.extend(self.reset_folder(conn, &path, cursor, uidvalidity, &server)?);
                    continue;
                }
                let known = cursor.known_set();
                let (mut seen, mut flagged) = (uidset::decode(&cursor.seen), uidset::decode(&cursor.flagged));
                // Deletions: known locally, gone from the server.
                let gone: Vec<u32> = known.difference(&server).copied().collect();
                changes.deleted_provider_ids.extend(gone.iter().map(|uid| MessageRef::provider_id(&path, uidvalidity, *uid)));
                let mut known: BTreeSet<u32> = known.intersection(&server).copied().collect();
                // New mail: above the highest UID seen, newest first, bounded per cycle.
                let limit = self.folder_limit(cursor.label_kind()).max(200);
                let new: BTreeSet<u32> = server.iter().rev().filter(|u| **u > cursor.highest_uid).take(limit).copied().collect();
                if !new.is_empty() {
                    let (rows, new_seen, new_flagged) = Self::fetch_headers(conn, account, &path, uidvalidity, &new)?;
                    changes.message_ids.extend(rows.iter().filter_map(|m| m.provider_id.clone()));
                    changes.upserts.extend(rows);
                    known.extend(new.iter().copied());
                    seen.extend(new_seen);
                    flagged.extend(new_flagged);
                }
                // Flag changes: the Inbox every cycle, other folders now and then.
                let refresh = cursor.label_kind() == LabelKind::Inbox || cp.cycle % FLAG_REFRESH_EVERY == 0;
                let old: BTreeSet<u32> = known.difference(&new).copied().collect();
                if refresh && !old.is_empty() {
                    let (_, now_seen, now_flagged) = conn.flags_for(&old)?;
                    let changed: BTreeSet<u32> = old.iter().filter(|u| seen.contains(u) != now_seen.contains(u) || flagged.contains(u) != now_flagged.contains(u)).copied().collect();
                    if !changed.is_empty() {
                        let (rows, _, _) = Self::fetch_headers(conn, account, &path, uidvalidity, &changed)?;
                        changes.upserts.extend(rows);
                    }
                    seen = seen.difference(&old).copied().chain(now_seen).collect();
                    flagged = flagged.difference(&old).copied().chain(now_flagged).collect();
                }
                cursor.highest_uid = cursor.highest_uid.max(server.iter().next_back().copied().unwrap_or(0));
                cursor.set_flags(&known, &seen, &flagged);
            }
            for path in vanished {
                cp.folders.remove(&path);
            }
            if relist_needed {
                let (labels, deleted) = self.relist(conn, account, &mut cp)?;
                changes.labels = Some(labels);
                changes.deleted_provider_ids.extend(deleted);
            }
            changes.checkpoint = Some(cp.to_json());
            Ok(changes)
        })
    }

    fn messages(&self, account: &Account, ids: &[ProviderId]) -> Result<Vec<Message>, ProviderError> {
        // Group by folder and UIDVALIDITY, keeping the first-seen order of folders.
        let mut groups: Vec<((String, u32), BTreeSet<u32>)> = Vec::new();
        for id in ids {
            let Some(r) = MessageRef::parse(id) else {
                warn!("imap: ignoring malformed provider id {:?}", id.0);
                continue;
            };
            let key = (r.folder, r.uidvalidity);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, set)) => {
                    set.insert(r.uid);
                }
                None => groups.push((key, [r.uid].into_iter().collect())),
            }
        }
        self.with_session(|conn| {
            let mut out = Vec::new();
            for ((path, uidvalidity), uids) in &groups {
                if !conn.examine_for(path, *uidvalidity)? {
                    info!("imap: {path} changed under us; {} queued messages skipped until the next sync", uids.len());
                    continue;
                }
                out.extend(Self::fetch_bodies(conn, account, path, *uidvalidity, uids)?);
            }
            Ok(out)
        })
    }

    fn attachment(&self, message: &Message, attachment: &Attachment) -> Result<Vec<u8>, ProviderError> {
        let r = message.provider_id.as_ref().and_then(MessageRef::parse).ok_or_else(|| ProviderError::Rejected("this message has no server copy".into()))?;
        let section = attachment.provider_id.as_ref().map(|p| p.0.clone()).ok_or_else(|| ProviderError::Rejected("this attachment has no section number".into()))?;
        let section_path: Vec<u32> = section.split('.').map(|n| n.parse::<u32>().map_err(|_| ProviderError::Rejected(format!("bad attachment section {section:?}")))).collect::<Result<_, _>>()?;
        self.with_session(|conn| {
            if !conn.examine_for(&r.folder, r.uidvalidity)? {
                return Err(ProviderError::Rejected("the message is no longer in that folder; sync and try again".into()));
            }
            let fetches = conn.session().uid_fetch(r.uid.to_string(), format!("(UID BODY.PEEK[{section}] BODY.PEEK[{section}.MIME])")).map_err(|e| map_error(&e))?;
            let Some(f) = fetches.iter().find(|f| f.uid == Some(r.uid)) else {
                return Err(ProviderError::Rejected("the message is no longer on the server".into()));
            };
            let Some(body) = f.section(&SectionPath::Part(section_path.clone(), None)) else {
                return Err(ProviderError::Rejected("the server returned no data for this attachment".into()));
            };
            let mime = f.section(&SectionPath::Part(section_path.clone(), Some(MessageSection::Mime))).unwrap_or_default();
            Ok(decode_part(mime, body))
        })
    }

    fn raw(&self, message: &Message) -> Result<Vec<u8>, ProviderError> {
        let r = message.provider_id.as_ref().and_then(MessageRef::parse).ok_or_else(|| ProviderError::Rejected("this message has no server copy".into()))?;
        self.with_session(|conn| {
            if !conn.examine_for(&r.folder, r.uidvalidity)? {
                return Err(ProviderError::Rejected("the message is no longer in that folder".into()));
            }
            let fetches = conn.session().uid_fetch(r.uid.to_string(), "(UID BODY.PEEK[])").map_err(|e| map_error(&e))?;
            fetches.iter().find(|f| f.uid == Some(r.uid)).and_then(|f| f.body().map(<[u8]>::to_vec)).ok_or_else(|| ProviderError::Rejected("the message is no longer on the server".into()))
        })
    }
}

/// Decodes a fetched body part using its MIME headers (transfer encoding, charset) through
/// the shared parser. Falls back to the raw bytes when the headers are unusable.
pub fn decode_part(mime_headers: &[u8], body: &[u8]) -> Vec<u8> {
    if mime_headers.trim_ascii().is_empty() {
        return body.to_vec();
    }
    let mut raw = mime_headers.trim_ascii_end().to_vec();
    raw.extend_from_slice(b"\r\n\r\n");
    raw.extend_from_slice(body);
    let Some(parsed) = mail_parser::MessageParser::default().parse(&raw) else { return body.to_vec() };
    match parsed.parts.first().map(|p| &p.body) {
        Some(mail_parser::PartType::Binary(b)) | Some(mail_parser::PartType::InlineBinary(b)) => b.to_vec(),
        Some(mail_parser::PartType::Text(t)) | Some(mail_parser::PartType::Html(t)) => t.as_bytes().to_vec(),
        Some(mail_parser::PartType::Message(m)) => m.raw_message.to_vec(),
        _ => body.to_vec(),
    }
}

const WRITE_PATH_PENDING: &str = "Changes to IMAP mailboxes are not sent to the server yet; this arrives in the next release.";

impl MailProvider for ImapProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Imap
    }

    fn probe(&self) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async move { self.with_session(|conn| conn.session().noop().map_err(|e| map_error(&e))) })
    }

    fn fetch_snapshot(&self, account: &Account) -> BoxFuture<'_, Result<MailboxSnapshot, ProviderError>> {
        let account = account.clone();
        Box::pin(async move { self.snapshot(&account) })
    }

    fn fetch_changes(&self, account: &Account, checkpoint: &str) -> BoxFuture<'_, Result<RemoteChanges, ProviderError>> {
        let account = account.clone();
        let checkpoint = checkpoint.to_owned();
        Box::pin(async move { self.changes(&account, &checkpoint) })
    }

    fn fetch_messages(&self, account: &Account, ids: &[ProviderId], _labels: &[Label]) -> BoxFuture<'_, Result<Vec<Message>, ProviderError>> {
        let account = account.clone();
        let ids = ids.to_vec();
        Box::pin(async move { self.messages(&account, &ids) })
    }

    fn apply(&self, _account: &Account, op: &PendingOperation, _messages: &[Message], _labels: &[Label]) -> BoxFuture<'_, Result<AppliedChanges, ProviderError>> {
        let calendar = op.kind.is_calendar();
        Box::pin(async move {
            if calendar {
                return Err(ProviderError::Rejected("this account has no calendar".into()));
            }
            Err(ProviderError::Rejected(WRITE_PATH_PENDING.into()))
        })
    }

    fn fetch_raw(&self, _account: &Account, message: &Message) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let message = message.clone();
        Box::pin(async move {
            match self.raw(&message) {
                Ok(bytes) => Ok(bytes),
                Err(ProviderError::Rejected(_)) => Ok(crate::message_to_raw(&message)),
                Err(e) => Err(e),
            }
        })
    }

    fn fetch_attachment(&self, _account: &Account, message: &Message, attachment: &Attachment) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let message = message.clone();
        let attachment = attachment.clone();
        Box::pin(async move { self.attachment(&message, &attachment) })
    }

    fn send(&self, _account: &Account, _draft: &Draft, _context: SendContext) -> BoxFuture<'_, Result<SentReceipt, ProviderError>> {
        Box::pin(async { Err(ProviderError::Rejected("Sending from IMAP accounts arrives in the next release.".into())) })
    }

    fn reconcile_send(&self, _account: &Account, _draft: &Draft) -> BoxFuture<'_, Result<SendOutcome, ProviderError>> {
        Box::pin(async { Ok(SendOutcome::NotDelivered) })
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn provider_ids_round_trip() {
        let id = MessageRef::provider_id("Work/Clients", 1760400000, 42);
        assert_eq!(id.0, "Work/Clients\u{1f}1760400000\u{1f}42");
        assert_eq!(MessageRef::parse(&id), Some(MessageRef { folder: "Work/Clients".into(), uidvalidity: 1760400000, uid: 42 }));
        assert_eq!(MessageRef::parse(&ProviderId("gmail-id".into())), None);
    }

    #[test]
    fn checkpoint_round_trips_and_rejects_other_versions() {
        let mut cp = Checkpoint { version: Checkpoint::VERSION, listed_at: 5, cycle: 2, folders: BTreeMap::new() };
        cp.folders.insert("INBOX".into(), FolderCursor { kind: "inbox".into(), uidvalidity: 7, highest_uid: 9, known: "3:9".into(), seen: "3:5".into(), flagged: "9".into(), delimiter: Some("/".into()), special_use: None, subscribed: true, fresh: false });
        let json = cp.to_json();
        assert_eq!(Checkpoint::parse(&json).unwrap(), cp);
        assert_eq!(Checkpoint::parse("12345"), Err(ProviderError::CheckpointExpired), "a Gmail history id is not ours");
        assert_eq!(Checkpoint::parse(r#"{"version":99}"#), Err(ProviderError::CheckpointExpired));
        let rows = folder_states(&json);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].path.as_str(), rows[0].uidvalidity, rows[0].highest_uid, rows[0].delimiter.as_deref()), ("INBOX", Some(7), 9, Some("/")));
    }

    #[test]
    fn folder_limits_scale_with_the_inbox_preference() {
        let servers = MailServers { incoming: ServerEndpoint::new("localhost", 143, Security::None, "j"), outgoing: ServerEndpoint::new("localhost", 25, Security::None, "j"), pop3: None, imap: None };
        let p = ImapProvider::new(servers.clone(), "pw");
        assert_eq!((p.folder_limit(LabelKind::Inbox), p.folder_limit(LabelKind::Sent), p.folder_limit(LabelKind::Trash), p.folder_limit(LabelKind::Spam), p.folder_limit(LabelKind::User)), (100, 40, 20, 10, 20));
        let p = ImapProvider::new(servers, "pw").with_inbox_limit(300);
        assert_eq!((p.folder_limit(LabelKind::Inbox), p.folder_limit(LabelKind::Sent), p.folder_limit(LabelKind::Spam)), (300, 120, 30));
        assert!(!format!("{p:?}").contains("pw"), "the password never appears in Debug output");
    }

    #[test]
    fn decodes_parts_by_their_mime_headers() {
        assert_eq!(decode_part(b"Content-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n\r\n", b"JVBERi0xLjQK"), b"%PDF-1.4\n");
        assert_eq!(decode_part(b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n", b"Caf=C3=A9"), "Café".as_bytes());
        assert_eq!(decode_part(b"", b"raw"), b"raw");
    }
}
