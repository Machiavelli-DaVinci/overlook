//! A scripted IMAP server for the provider tests, shaped like Dovecot's responses.
//!
//! It answers the commands the provider issues (`CAPABILITY`, `LOGIN`, `NOOP`, `LIST`,
//! `LSUB`, `EXAMINE`/`SELECT`, `UID SEARCH ALL`, `UID FETCH` with every item the provider
//! asks for, `LOGOUT`) from a mutable mailbox the test can change between calls, and can
//! misbehave on request: refuse the login, say `BYE`, drop the connection, or report a
//! folder as in use. Sections (`BODY[2.1]`, `BODY[2.1.MIME]`, `BODY[TEXT]`) are served from
//! the raw message through `mail-parser`'s part offsets.

use mail_parser::{MessageParser, PartType};
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
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
    /// Folders whose `EXAMINE` answers `NO [INUSE]`.
    pub in_use: HashSet<String>,
    /// Every command line received, with passwords blanked.
    pub log: Vec<String>,
    pub connections: usize,
}

impl Default for FakeState {
    fn default() -> Self {
        Self { user: "jason@example.com".into(), password: "hunter2".into(), capabilities: "IMAP4rev1 UIDPLUS MOVE IDLE SPECIAL-USE LITERAL+ ENABLE".into(), folders: Vec::new(), subscribed: HashSet::new(), login: Login::Ok, drop_next_command: false, in_use: HashSet::new(), log: Vec::new(), connections: 0 }
    }
}

impl FakeState {
    pub fn folder_mut(&mut self, path: &str) -> &mut FakeFolder {
        self.folders.iter_mut().find(|f| f.path == path).unwrap_or_else(|| panic!("no fake folder {path}"))
    }

    pub fn commands(&self, prefix: &str) -> usize {
        self.log.iter().filter(|l| l.to_ascii_uppercase().contains(&prefix.to_ascii_uppercase())).count()
    }
}

pub struct FakeServer {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<FakeState>>,
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
                let _ = Connection { state, stream: stream.try_clone().unwrap(), reader: BufReader::new(stream), selected: None }.run();
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
}

impl Connection {
    fn say(&mut self, line: &str) -> std::io::Result<()> {
        self.stream.write_all(line.as_bytes())?;
        self.stream.write_all(b"\r\n")
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
            {
                let mut s = self.state.lock().unwrap();
                s.log.push(if cmd == "LOGIN" { format!("{tag} LOGIN ****") } else { text.clone() });
                if s.drop_next_command {
                    s.drop_next_command = false;
                    return Ok(());
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
                    for f in &s.folders {
                        if cmd == "LSUB" && !s.subscribed.contains(&f.path) {
                            continue;
                        }
                        self.stream.write_all(format!("* {cmd} ({}) \"{}\" {}\r\n", f.attributes.join(" "), f.delimiter, quote(&f.path)).as_bytes())?;
                    }
                    drop(s);
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
                    self.say("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)")?;
                    self.say("* OK [PERMANENTFLAGS ()] Read-only mailbox.")?;
                    self.say(&format!("* {exists} EXISTS"))?;
                    self.say("* 0 RECENT")?;
                    self.say(&format!("* OK [UIDVALIDITY {uidvalidity}] UIDs valid"))?;
                    self.say(&format!("* OK [UIDNEXT {next}] Predicted next UID"))?;
                    self.say(&format!("{tag} OK [READ-ONLY] Examine completed"))?;
                }
                "UID" => {
                    let (sub, sargs) = args.split_once(' ').unwrap_or((args, ""));
                    match sub.to_ascii_uppercase().as_str() {
                        "SEARCH" => {
                            let uids: Vec<u32> = self.selected_folder(|f| f.messages.keys().copied().collect()).unwrap_or_default();
                            self.say(&format!("* SEARCH{}", uids.iter().map(|u| format!(" {u}")).collect::<String>()))?;
                            self.say(&format!("{tag} OK Search completed."))?;
                        }
                        "FETCH" => {
                            let (set, items) = sargs.split_once(' ').unwrap_or((sargs, ""));
                            self.fetch(tag, set, items.trim().trim_start_matches('(').trim_end_matches(')'))?;
                        }
                        _ => self.say(&format!("{tag} BAD Unknown UID command"))?,
                    }
                }
                "LOGOUT" => {
                    self.say("* BYE Logging out")?;
                    self.say(&format!("{tag} OK Logout completed."))?;
                    return Ok(());
                }
                _ => self.say(&format!("{tag} BAD Error in IMAP command {cmd}: Unknown command."))?,
            }
        }
    }

    fn selected_folder<T>(&self, f: impl FnOnce(&FakeFolder) -> T) -> Option<T> {
        let path = self.selected.as_ref()?;
        let s = self.state.lock().unwrap();
        s.folders.iter().find(|x| &x.path == path).map(f)
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
            self.stream.write_all(&out)?;
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
