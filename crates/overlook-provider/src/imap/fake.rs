//! A scripted IMAP server for the provider tests, shaped like Dovecot's responses.
//!
//! It answers the commands the provider issues (`CAPABILITY`, `LOGIN`, `NOOP`, `LIST`,
//! `LSUB`, `EXAMINE`/`SELECT`, `CREATE`, `UID SEARCH` (`ALL` and `HEADER Message-ID`),
//! `UID FETCH` with every item the provider asks for, `UID STORE`, `UID COPY` and `UID MOVE`
//! with `COPYUID`, `UID EXPUNGE`/`EXPUNGE`, `APPEND` with `APPENDUID`, `LOGOUT`) from a
//! mutable mailbox the test can change between calls, and can misbehave on request: refuse
//! the login, say `BYE`, drop the connection before or after a command, answer a command
//! with a scripted `NO`, or report a folder as in use. Sections (`BODY[2.1]`,
//! `BODY[2.1.MIME]`, `BODY[TEXT]`) are served from the raw message through `mail-parser`'s
//! part offsets.
//!
//! Compiled for this crate's tests and, with the `fake-servers` feature, for other crates'
//! tests that drive the real provider through the sync engine.

use mail_parser::{MessageParser, PartType};
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Login {
    Ok,
    AuthFailed,
    Unavailable,
    Bye,
}

#[derive(Clone, Debug)]
pub struct FakeMessage {
    pub flags: Vec<String>,
    pub internaldate: String,
    pub raw: Vec<u8>,
    /// Served for `BODYSTRUCTURE`; a plain text structure is synthesised when absent.
    pub bodystructure: Option<String>,
    /// Reported `RFC822.SIZE`, when it should differ from the bytes (to force the large path).
    pub size_override: Option<usize>,
}

impl FakeMessage {
    pub fn new(flags: &[&str], internaldate: &str, raw: impl Into<Vec<u8>>) -> Self {
        Self { flags: flags.iter().map(|f| f.to_string()).collect(), internaldate: internaldate.into(), raw: raw.into(), bodystructure: None, size_override: None }
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
    }

    /// The value of a header line, for `SEARCH HEADER`.
    pub fn header(&self, name: &str) -> Option<String> {
        header_value(&self.raw, name)
    }
}

#[derive(Clone, Debug)]
pub struct FakeFolder {
    pub path: String,
    pub attributes: Vec<String>,
    pub delimiter: String,
    pub uidvalidity: u32,
    pub messages: BTreeMap<u32, FakeMessage>,
    pub next_uid: u32,
}

impl FakeFolder {
    pub fn new(path: &str, attributes: &[&str], uidvalidity: u32) -> Self {
        Self { path: path.into(), attributes: attributes.iter().map(|a| a.to_string()).collect(), delimiter: "/".into(), uidvalidity, messages: BTreeMap::new(), next_uid: 1 }
    }

    pub fn add(&mut self, m: FakeMessage) -> u32 {
        let uid = self.next_uid;
        self.next_uid += 1;
        self.messages.insert(uid, m);
        uid
    }

    /// UIDs of messages whose named header equals `value`.
    pub fn find_header(&self, name: &str, value: &str) -> Vec<u32> {
        self.messages.iter().filter(|(_, m)| m.header(name).as_deref() == Some(value)).map(|(u, _)| *u).collect()
    }
}

pub struct FakeState {
    pub user: String,
    pub password: String,
    pub capabilities: String,
    pub folders: Vec<FakeFolder>,
    pub subscribed: HashSet<String>,
    pub login: Login,
    /// Close the connection without answering the next command, once.
    pub drop_next_command: bool,
    /// Execute the next command whose line contains this word (upper-cased, e.g. `UID MOVE`,
    /// `APPEND`, `FETCH`), then close the connection without answering it, once. This is
    /// the "server did it, client never heard" case.
    pub drop_after_next: Option<String>,
    /// Answer the next command whose line contains this word with the given tagged reply
    /// (without the tag, e.g. `NO [UNAVAILABLE] Try again later`) instead of executing it.
    pub fail_next: Option<(String, String)>,
    /// Folders whose `EXAMINE`/`SELECT` answers `NO [INUSE]`.
    pub in_use: HashSet<String>,
    /// Every command line received, with passwords blanked.
    pub log: Vec<String>,
    pub connections: usize,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            user: "jason@example.com".into(),
            password: "hunter2".into(),
            capabilities: "IMAP4rev1 UIDPLUS MOVE IDLE SPECIAL-USE LITERAL+ ENABLE".into(),
            folders: Vec::new(),
            subscribed: HashSet::new(),
            login: Login::Ok,
            drop_next_command: false,
            drop_after_next: None,
            fail_next: None,
            in_use: HashSet::new(),
            log: Vec::new(),
            connections: 0,
        }
    }
}

impl FakeState {
    pub fn folder(&self, path: &str) -> &FakeFolder {
        self.folders.iter().find(|f| f.path == path).unwrap_or_else(|| panic!("no fake folder {path}"))
    }

    pub fn folder_mut(&mut self, path: &str) -> &mut FakeFolder {
        self.folders.iter_mut().find(|f| f.path == path).unwrap_or_else(|| panic!("no fake folder {path}"))
    }

    pub fn has_folder(&self, path: &str) -> bool {
        self.folders.iter().any(|f| f.path == path)
    }

    pub fn commands(&self, prefix: &str) -> usize {
        self.log.iter().filter(|l| l.to_ascii_uppercase().contains(&prefix.to_ascii_uppercase())).count()
    }

    fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.split_whitespace().any(|c| c.eq_ignore_ascii_case(cap))
    }
}

pub struct FakeServer {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<FakeState>>,
}

impl FakeServer {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }
}

pub fn serve(state: FakeState) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(state));
    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let state = shared.clone();
            std::thread::spawn(move || {
                let _ = Connection { state, stream: stream.try_clone().unwrap(), reader: BufReader::new(stream), selected: None, muted: false }.run();
            });
        }
    });
    FakeServer { addr, state }
}

struct Connection {
    state: Arc<Mutex<FakeState>>,
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    selected: Option<String>,
    /// Swallow output: the command runs but the client hears nothing (then the connection
    /// closes).
    muted: bool,
}

impl Connection {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if self.muted {
            return Ok(());
        }
        self.stream.write_all(bytes)
    }

    fn say(&mut self, line: &str) -> std::io::Result<()> {
        self.write(line.as_bytes())?;
        self.write(b"\r\n")
    }

    fn run(&mut self) -> std::io::Result<()> {
        let caps = {
            let mut s = self.state.lock().unwrap();
            s.connections += 1;
            s.capabilities.clone()
        };
        self.say(&format!("* OK [CAPABILITY {caps}] Fake IMAP ready."))?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let text = line.trim_end_matches(['\r', '\n']).to_owned();
            let (tag, rest) = text.split_once(' ').unwrap_or((&text, ""));
            let (cmd, args) = rest.split_once(' ').unwrap_or((rest, ""));
            let cmd = cmd.to_ascii_uppercase();
            let upper = text.to_ascii_uppercase();
            {
                let mut s = self.state.lock().unwrap();
                s.log.push(if cmd == "LOGIN" { format!("{tag} LOGIN ****") } else { text.clone() });
                if s.drop_next_command {
                    s.drop_next_command = false;
                    return Ok(());
                }
                if s.drop_after_next.as_ref().is_some_and(|w| upper.contains(w.as_str())) {
                    s.drop_after_next = None;
                    self.muted = true;
                }
                if let Some((word, reply)) = s.fail_next.clone() {
                    if upper.contains(word.as_str()) {
                        s.fail_next = None;
                        drop(s);
                        self.say(&format!("{tag} {reply}"))?;
                        continue;
                    }
                }
            }
            match cmd.as_str() {
                "CAPABILITY" => {
                    self.say(&format!("* CAPABILITY {caps}"))?;
                    self.say(&format!("{tag} OK Capability completed."))?;
                }
                "LOGIN" => {
                    let (behaviour, user, password) = {
                        let s = self.state.lock().unwrap();
                        (s.login, s.user.clone(), s.password.clone())
                    };
                    let mut parts = args.splitn(2, ' ');
                    let (u, p) = (unquote(parts.next().unwrap_or("")), unquote(parts.next().unwrap_or("")));
                    match behaviour {
                        Login::Bye => {
                            self.say("* BYE Server shutting down")?;
                            return Ok(());
                        }
                        Login::Unavailable => self.say(&format!("{tag} NO [UNAVAILABLE] Temporary authentication failure."))?,
                        Login::AuthFailed => self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] Authentication failed."))?,
                        Login::Ok if u == user && p == password => self.say(&format!("{tag} OK [CAPABILITY {caps}] Logged in"))?,
                        Login::Ok => self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] Authentication failed."))?,
                    }
                }
                "NOOP" => self.say(&format!("{tag} OK NOOP completed."))?,
                "LIST" | "LSUB" => {
                    let s = self.state.lock().unwrap();
                    let mut lines = Vec::new();
                    for f in &s.folders {
                        if cmd == "LSUB" && !s.subscribed.contains(&f.path) {
                            continue;
                        }
                        lines.push(format!("* {cmd} ({}) \"{}\" {}", f.attributes.join(" "), f.delimiter, quote(&f.path)));
                    }
                    drop(s);
                    for l in lines {
                        self.say(&l)?;
                    }
                    self.say(&format!("{tag} OK {} completed.", if cmd == "LIST" { "List" } else { "Lsub" }))?;
                }
                "SELECT" | "EXAMINE" => {
                    let path = unquote(args);
                    let s = self.state.lock().unwrap();
                    if s.in_use.contains(&path) {
                        drop(s);
                        self.say(&format!("{tag} NO [INUSE] Mailbox is in use by another process"))?;
                        continue;
                    }
                    let Some(f) = s.folders.iter().find(|f| f.path == path) else {
                        drop(s);
                        self.say(&format!("{tag} NO Mailbox doesn't exist: {path}"))?;
                        continue;
                    };
                    let (exists, uidvalidity, next) = (f.messages.len(), f.uidvalidity, f.next_uid);
                    drop(s);
                    self.selected = Some(path);
                    let writable = cmd == "SELECT";
                    self.say("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)")?;
                    if writable {
                        self.say("* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted.")?;
                    } else {
                        self.say("* OK [PERMANENTFLAGS ()] Read-only mailbox.")?;
                    }
                    self.say(&format!("* {exists} EXISTS"))?;
                    self.say("* 0 RECENT")?;
                    self.say(&format!("* OK [UIDVALIDITY {uidvalidity}] UIDs valid"))?;
                    self.say(&format!("* OK [UIDNEXT {next}] Predicted next UID"))?;
                    if writable {
                        self.say(&format!("{tag} OK [READ-WRITE] Select completed"))?;
                    } else {
                        self.say(&format!("{tag} OK [READ-ONLY] Examine completed"))?;
                    }
                }
                "CREATE" => {
                    let path = unquote(args);
                    let mut s = self.state.lock().unwrap();
                    if s.has_folder(&path) {
                        drop(s);
                        self.say(&format!("{tag} NO [ALREADYEXISTS] Mailbox already exists"))?;
                    } else {
                        let uidvalidity = 1_800_000_000 + s.folders.len() as u32;
                        s.folders.push(FakeFolder::new(&path, &["\\HasNoChildren"], uidvalidity));
                        drop(s);
                        self.say(&format!("{tag} OK Create completed."))?;
                    }
                }
                "UID" => {
                    let (sub, sargs) = args.split_once(' ').unwrap_or((args, ""));
                    match sub.to_ascii_uppercase().as_str() {
                        "SEARCH" => self.search(tag, sargs)?,
                        "FETCH" => {
                            let (set, items) = sargs.split_once(' ').unwrap_or((sargs, ""));
                            self.fetch(tag, set, items.trim().trim_start_matches('(').trim_end_matches(')'))?;
                        }
                        "STORE" => self.store(tag, sargs)?,
                        "COPY" => self.copy_or_move(tag, sargs, false)?,
                        "MOVE" => self.copy_or_move(tag, sargs, true)?,
                        "EXPUNGE" => self.expunge(tag, Some(sargs.trim()))?,
                        _ => self.say(&format!("{tag} BAD Unknown UID command"))?,
                    }
                }
                "EXPUNGE" => self.expunge(tag, None)?,
                "APPEND" => self.append(tag, args)?,
                "LOGOUT" => {
                    self.say("* BYE Logging out")?;
                    self.say(&format!("{tag} OK Logout completed."))?;
                    return Ok(());
                }
                _ => self.say(&format!("{tag} BAD Error in IMAP command {cmd}: Unknown command."))?,
            }
            if self.muted {
                // The command ran; the client never hears the answer.
                return Ok(());
            }
        }
    }

    fn selected_folder<T>(&self, f: impl FnOnce(&FakeFolder) -> T) -> Option<T> {
        let path = self.selected.as_ref()?;
        let s = self.state.lock().unwrap();
        s.folders.iter().find(|x| &x.path == path).map(f)
    }

    fn search(&mut self, tag: &str, criteria: &str) -> std::io::Result<()> {
        let upper = criteria.trim().to_ascii_uppercase();
        let uids: Vec<u32> = if upper == "ALL" {
            self.selected_folder(|f| f.messages.keys().copied().collect()).unwrap_or_default()
        } else if upper.starts_with("HEADER ") {
            let mut parts = criteria.trim().splitn(3, ' ');
            let (_, name, value) = (parts.next(), parts.next().unwrap_or(""), unquote(parts.next().unwrap_or("")));
            self.selected_folder(|f| f.find_header(name, &value)).unwrap_or_default()
        } else {
            Vec::new()
        };
        self.say(&format!("* SEARCH{}", uids.iter().map(|u| format!(" {u}")).collect::<String>()))?;
        self.say(&format!("{tag} OK Search completed."))
    }

    fn store(&mut self, tag: &str, args: &str) -> std::io::Result<()> {
        let Some(path) = self.selected.clone() else {
            return self.say(&format!("{tag} BAD No mailbox selected"));
        };
        let (set, rest) = args.split_once(' ').unwrap_or((args, ""));
        let (item, flags) = rest.split_once(' ').unwrap_or((rest, ""));
        let item = item.to_ascii_uppercase();
        let silent = item.ends_with(".SILENT");
        let flags: Vec<String> = flags.trim().trim_start_matches('(').trim_end_matches(')').split_whitespace().map(str::to_owned).collect();
        let mut lines = Vec::new();
        {
            let mut s = self.state.lock().unwrap();
            let folder = s.folder_mut(&path);
            let wanted = parse_set(set, folder.messages.keys().copied().collect());
            for (seq, (uid, m)) in folder.messages.iter_mut().enumerate() {
                if !wanted.contains(uid) {
                    continue;
                }
                if item.starts_with('+') {
                    for f in &flags {
                        if !m.has_flag(f) {
                            m.flags.push(f.clone());
                        }
                    }
                } else if item.starts_with('-') {
                    m.flags.retain(|f| !flags.iter().any(|x| x.eq_ignore_ascii_case(f)));
                } else {
                    m.flags = flags.clone();
                }
                if !silent {
                    lines.push(format!("* {} FETCH (UID {uid} FLAGS ({}))", seq + 1, m.flags.join(" ")));
                }
            }
        }
        for l in lines {
            self.say(&l)?;
        }
        self.say(&format!("{tag} OK Store completed."))
    }

    fn copy_or_move(&mut self, tag: &str, args: &str, mv: bool) -> std::io::Result<()> {
        let Some(path) = self.selected.clone() else {
            return self.say(&format!("{tag} BAD No mailbox selected"));
        };
        let (set, target) = args.split_once(' ').unwrap_or((args, ""));
        let target = unquote(target);
        let mut s = self.state.lock().unwrap();
        if mv && !s.has_capability("MOVE") {
            drop(s);
            return self.say(&format!("{tag} BAD Error in IMAP command UID MOVE: Unknown command."));
        }
        if !s.has_folder(&target) {
            drop(s);
            return self.say(&format!("{tag} NO [TRYCREATE] Mailbox doesn't exist: {target}"));
        }
        let uidplus = s.has_capability("UIDPLUS");
        let source_uids: Vec<u32> = {
            let f = s.folder(&path);
            let wanted = parse_set(set, f.messages.keys().copied().collect());
            f.messages.keys().copied().filter(|u| wanted.contains(u)).collect()
        };
        let mut copied: Vec<(u32, u32)> = Vec::new();
        let mut expunged_seqs: Vec<usize> = Vec::new();
        for uid in &source_uids {
            let m = s.folder(&path).messages[uid].clone();
            let new_uid = s.folder_mut(&target).add(m);
            copied.push((*uid, new_uid));
        }
        if mv {
            let folder = s.folder_mut(&path);
            let seqs: Vec<usize> = folder.messages.keys().enumerate().filter(|(_, u)| source_uids.contains(u)).map(|(i, _)| i + 1).collect();
            for uid in &source_uids {
                folder.messages.remove(uid);
            }
            expunged_seqs = seqs;
        }
        let dest_validity = s.folder(&target).uidvalidity;
        drop(s);
        let copyuid = (uidplus && !copied.is_empty()).then(|| format!("[COPYUID {dest_validity} {} {}] ", encode_set(copied.iter().map(|c| c.0)), encode_set(copied.iter().map(|c| c.1))));
        if mv {
            if let Some(code) = &copyuid {
                self.say(&format!("* OK {code}Moved UIDs."))?;
            }
            for seq in expunged_seqs.into_iter().rev() {
                self.say(&format!("* {seq} EXPUNGE"))?;
            }
            self.say(&format!("{tag} OK Move completed."))
        } else {
            self.say(&format!("{tag} OK {}Copy completed.", copyuid.unwrap_or_default()))
        }
    }

    fn expunge(&mut self, tag: &str, set: Option<&str>) -> std::io::Result<()> {
        let Some(path) = self.selected.clone() else {
            return self.say(&format!("{tag} BAD No mailbox selected"));
        };
        let seqs: Vec<usize> = {
            let mut s = self.state.lock().unwrap();
            let folder = s.folder_mut(&path);
            let wanted = set.map(|set| parse_set(set, folder.messages.keys().copied().collect()));
            let gone: Vec<u32> = folder.messages.iter().filter(|(u, m)| m.has_flag("\\Deleted") && wanted.as_ref().is_none_or(|w| w.contains(u))).map(|(u, _)| *u).collect();
            let seqs: Vec<usize> = folder.messages.keys().enumerate().filter(|(_, u)| gone.contains(u)).map(|(i, _)| i + 1).collect();
            for uid in gone {
                folder.messages.remove(&uid);
            }
            seqs
        };
        for seq in seqs.into_iter().rev() {
            self.say(&format!("* {seq} EXPUNGE"))?;
        }
        self.say(&format!("{tag} OK Expunge completed."))
    }

    /// `APPEND "<mailbox>" (<flags>) ["<date>"] {<n>}`: answers the continuation, reads the
    /// literal, stores the message and reports `APPENDUID` when UIDPLUS is advertised.
    fn append(&mut self, tag: &str, args: &str) -> std::io::Result<()> {
        let Some(open) = args.rfind('{') else {
            return self.say(&format!("{tag} BAD Missing literal"));
        };
        let size: usize = args[open + 1..].trim_end().trim_end_matches('}').trim_end_matches('+').parse().unwrap_or(0);
        let head = args[..open].trim();
        let (mailbox, rest) = split_quoted(head);
        let flags: Vec<String> = match (rest.find('('), rest.find(')')) {
            (Some(a), Some(b)) if b > a => rest[a + 1..b].split_whitespace().map(str::to_owned).collect(),
            _ => Vec::new(),
        };
        let exists = self.state.lock().unwrap().has_folder(&mailbox);
        if !exists {
            return self.say(&format!("{tag} NO [TRYCREATE] Mailbox doesn't exist: {mailbox}"));
        }
        if !args.trim_end().ends_with("+}") {
            // The client waits for the continuation before sending the literal; a muted
            // connection still sends it so "stored, reply lost" can be scripted.
            let muted = std::mem::replace(&mut self.muted, false);
            self.say("+ OK")?;
            self.muted = muted;
        }
        let mut raw = vec![0u8; size];
        self.reader.read_exact(&mut raw)?;
        let mut trailing = String::new();
        self.reader.read_line(&mut trailing)?;
        let (uidvalidity, uid, uidplus) = {
            let mut s = self.state.lock().unwrap();
            let uidplus = s.has_capability("UIDPLUS");
            let folder = s.folder_mut(&mailbox);
            let mut m = FakeMessage::new(&[], "14-Oct-2026 12:00:00 +0000", raw);
            m.flags = flags;
            let uid = folder.add(m);
            (folder.uidvalidity, uid, uidplus)
        };
        if uidplus {
            self.say(&format!("{tag} OK [APPENDUID {uidvalidity} {uid}] Append completed."))
        } else {
            self.say(&format!("{tag} OK Append completed."))
        }
    }

    fn fetch(&mut self, tag: &str, set: &str, items: &str) -> std::io::Result<()> {
        let Some(folder) = self.selected_folder(|f| f.clone()) else {
            return self.say(&format!("{tag} BAD No mailbox selected"));
        };
        let wanted = parse_set(set, folder.messages.keys().copied().collect());
        let items: Vec<&str> = split_items(items);
        for (seq, (uid, m)) in folder.messages.iter().enumerate() {
            if !wanted.contains(uid) {
                continue;
            }
            let mut out: Vec<u8> = format!("* {} FETCH (UID {uid}", seq + 1).into_bytes();
            for item in &items {
                let upper = item.to_ascii_uppercase();
                if upper == "UID" {
                    continue;
                }
                match upper.as_str() {
                    "FLAGS" => out.extend_from_slice(format!(" FLAGS ({})", m.flags.join(" ")).as_bytes()),
                    "INTERNALDATE" => out.extend_from_slice(format!(" INTERNALDATE \"{}\"", m.internaldate).as_bytes()),
                    "RFC822.SIZE" => out.extend_from_slice(format!(" RFC822.SIZE {}", m.size_override.unwrap_or(m.raw.len())).as_bytes()),
                    "BODYSTRUCTURE" => {
                        let bs = m.bodystructure.clone().unwrap_or_else(|| format!("(\"TEXT\" \"PLAIN\" (\"CHARSET\" \"utf-8\") NIL NIL \"7BIT\" {} 1 NIL NIL NIL NIL)", m.raw.len()));
                        out.extend_from_slice(format!(" BODYSTRUCTURE {bs}").as_bytes());
                    }
                    _ if upper.starts_with("BODY.PEEK[") || upper.starts_with("BODY[") => {
                        let spec = &item[item.find('[').unwrap() + 1..item.rfind(']').unwrap()];
                        let partial = item[item.rfind(']').unwrap() + 1..].trim();
                        let data = section_bytes(&m.raw, spec);
                        let data = match partial.strip_prefix("<0.").and_then(|p| p.strip_suffix('>')).and_then(|n| n.parse::<usize>().ok()) {
                            Some(limit) => data[..data.len().min(limit)].to_vec(),
                            None => data,
                        };
                        let origin = if partial.is_empty() { String::new() } else { "<0>".to_owned() };
                        out.extend_from_slice(format!(" BODY[{spec}]{origin} {{{}}}\r\n", data.len()).as_bytes());
                        out.extend_from_slice(&data);
                    }
                    _ => {}
                }
            }
            out.extend_from_slice(b")\r\n");
            self.write(&out)?;
        }
        self.say(&format!("{tag} OK Fetch completed."))
    }
}

/// Splits a FETCH item list on spaces, keeping `BODY.PEEK[...]<0.N>` together.
fn split_items(items: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in items.char_indices() {
        match c {
            '[' | '<' => depth += 1,
            ']' | '>' => depth -= 1,
            ' ' if depth == 0 => {
                if i > start {
                    out.push(&items[start..i]);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < items.len() {
        out.push(&items[start..]);
    }
    out
}

/// `BODY[<spec>]` bytes: `` (whole), `HEADER`, `TEXT`, `n.m`, `n.m.MIME`.
pub fn section_bytes(raw: &[u8], spec: &str) -> Vec<u8> {
    let Some(parsed) = MessageParser::default().parse(raw) else { return raw.to_vec() };
    let root = &parsed.parts[0];
    let header_end = root.offset_body as usize;
    let upper = spec.to_ascii_uppercase();
    if spec.is_empty() {
        return raw.to_vec();
    }
    if upper == "HEADER" {
        return raw[..header_end.min(raw.len())].to_vec();
    }
    if upper == "TEXT" {
        return raw[header_end.min(raw.len())..].to_vec();
    }
    let (path, mime) = match upper.strip_suffix(".MIME") {
        Some(p) => (p.to_owned(), true),
        None => (upper.clone(), false),
    };
    let mut part_id = 0usize;
    let numbers: Vec<usize> = path.split('.').filter_map(|n| n.parse().ok()).collect();
    for (depth, n) in numbers.iter().enumerate() {
        match &parsed.parts[part_id].body {
            PartType::Multipart(children) => match children.get(n - 1) {
                Some(child) => part_id = *child as usize,
                None => return Vec::new(),
            },
            // A non-multipart part: section 1 is the part itself.
            _ if *n == 1 && depth == numbers.len() - 1 => {}
            _ => return Vec::new(),
        }
    }
    let part = &parsed.parts[part_id];
    let (h, b, e) = (part.offset_header as usize, part.offset_body as usize, part.offset_end as usize);
    if mime {
        raw[h.min(raw.len())..b.min(raw.len())].to_vec()
    } else {
        raw[b.min(raw.len())..e.min(raw.len())].to_vec()
    }
}

/// The value of the first header line named `name` (case-insensitive, no folding support
/// beyond what the tests need).
pub fn header_value(raw: &[u8], name: &str) -> Option<String> {
    let head = match raw.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(end) => &raw[..end],
        None => raw,
    };
    let prefix = format!("{}:", name.to_ascii_lowercase());
    for line in String::from_utf8_lossy(head).lines() {
        if line.to_ascii_lowercase().starts_with(&prefix) {
            return Some(line[prefix.len()..].trim().to_owned());
        }
    }
    None
}

fn parse_set(set: &str, uids: Vec<u32>) -> HashSet<u32> {
    let max = uids.iter().copied().max().unwrap_or(0);
    let mut out = HashSet::new();
    for piece in set.split(',') {
        match piece.split_once(':') {
            Some((a, b)) => {
                let a: u32 = a.parse().unwrap_or(0);
                let b: u32 = if b == "*" { max } else { b.parse().unwrap_or(0) };
                out.extend(uids.iter().copied().filter(|u| *u >= a.min(b) && *u <= a.max(b)));
            }
            None => {
                if let Ok(n) = piece.parse::<u32>() {
                    out.insert(n);
                }
            }
        }
    }
    out
}

/// A sequence set in the compact form servers use in `COPYUID` (`1,3:5`).
fn encode_set(uids: impl IntoIterator<Item = u32>) -> String {
    let mut sorted: Vec<u32> = uids.into_iter().collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        out.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
        i += 1;
    }
    out.join(",")
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        t.to_owned()
    }
}

/// Splits a leading (possibly quoted) word off `s`.
fn split_quoted(s: &str) -> (String, &str) {
    let t = s.trim_start();
    if let Some(rest) = t.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (unquote(&t[..end + 2]), &rest[end + 1..]);
        }
    }
    match t.split_once(' ') {
        Some((a, b)) => (a.to_owned(), b),
        None => (t.to_owned(), ""),
    }
}
