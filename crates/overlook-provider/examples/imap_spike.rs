//! M0 spike for the IMAP/POP3 plan: connect to an IMAP server, list folders, fetch and parse
//! recent messages, optionally send one message over SMTP and append it to Sent.
//!
//! Throwaway code: it exercises the `imap`, `mail-parser`, `lettre` (SMTP) and `rustls`
//! combination the plan proposes, against a real server. Nothing here is reused by the app.
//!
//! Configuration is by environment variable (never on the command line, so the password
//! stays out of shell history and `ps`):
//!
//! ```text
//! SPIKE_IMAP_HOST=imap.example.com   SPIKE_IMAP_PORT=993   SPIKE_IMAP_SECURITY=tls|starttls|plain
//! SPIKE_SMTP_HOST=smtp.example.com   SPIKE_SMTP_PORT=587   SPIKE_SMTP_SECURITY=tls|starttls|plain
//! SPIKE_USER=me@example.com          SPIKE_PASSWORD=…      SPIKE_FROM=me@example.com (defaults to SPIKE_USER)
//! SPIKE_FETCH=10                     SPIKE_SEND_TO=someone@example.com (omit to skip sending)
//! ```
//!
//! Run with `cargo run -p overlook-provider --example imap_spike`. Without a real server,
//! `python3 scripts/spike-mail-servers.py` serves a scripted plaintext IMAP (127.0.0.1:1143) and
//! SMTP (127.0.0.1:2525) pair with user `spike` / password `secret`; use `SPIKE_*_SECURITY=plain`.

use imap_proto::types::NameAttribute;
use mail_parser::{MessageParser, MimeHeaders};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(30);

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn required(name: &str) -> String {
    env(name).unwrap_or_else(|| {
        eprintln!("{name} is not set; see the comment at the top of examples/imap_spike.rs");
        std::process::exit(2)
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Security {
    Tls,
    StartTls,
    Plain,
}

impl Security {
    fn parse(value: Option<String>, default: Security) -> Security {
        match value.as_deref().map(|v| v.to_ascii_lowercase()).as_deref() {
            Some("tls") | Some("ssl") => Security::Tls,
            Some("starttls") => Security::StartTls,
            Some("plain") | Some("none") => Security::Plain,
            Some(other) => {
                eprintln!("unknown security mode {other:?}; use tls, starttls or plain");
                std::process::exit(2)
            }
            None => default,
        }
    }
}

/// A TCP stream that may be wrapped in TLS. The `imap` crate is generic over `Read + Write`,
/// so one `Client<Stream>` type serves all three security modes.
enum Stream {
    Plain(TcpStream),
    Tls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
}

fn tcp_connect(host: &str, port: u16) -> Result<TcpStream, String> {
    let addr = (host, port).to_socket_addrs().map_err(|e| format!("resolving {host}: {e}"))?.next().ok_or_else(|| format!("{host} resolved to nothing"))?;
    let tcp = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| format!("connecting to {host}:{port}: {e}"))?;
    tcp.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    Ok(tcp)
}

fn wrap_tls(host: &str, tcp: TcpStream) -> Result<Stream, String> {
    let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|e| format!("{host} is not a valid TLS server name: {e}"))?;
    let conn = rustls::ClientConnection::new(tls_config(), name).map_err(|e| format!("TLS setup: {e}"))?;
    Ok(Stream::Tls(rustls::StreamOwned::new(conn, tcp)))
}

/// Connects and authenticates. STARTTLS is negotiated by hand: greeting, `STARTTLS`, then
/// the plain socket is taken back out of the client and wrapped.
fn imap_session(host: &str, port: u16, security: Security, user: &str, password: &str) -> Result<imap::Session<Stream>, String> {
    let tcp = tcp_connect(host, port)?;
    let mut client = match security {
        Security::Tls => {
            let mut c = imap::Client::new(wrap_tls(host, tcp)?);
            c.read_greeting().map_err(|e| format!("greeting: {e}"))?;
            c
        }
        Security::Plain => {
            let mut c = imap::Client::new(Stream::Plain(tcp));
            c.read_greeting().map_err(|e| format!("greeting: {e}"))?;
            c
        }
        Security::StartTls => {
            // The imap crate only exposes STARTTLS through its own `ClientBuilder` (which brings
            // an older rustls). Pre-login commands are private on `Client`, so the upgrade is
            // negotiated on the raw socket, then the TLS stream is handed to the client.
            let tcp = starttls_by_hand(tcp)?;
            let mut c = imap::Client::new(wrap_tls(host, tcp)?);
            // No new greeting after STARTTLS; mark it read so login does not wait for one.
            c.greeting_read = true;
            c
        }
    };
    client.debug = env("SPIKE_DEBUG").is_some();
    client.login(user, password).map_err(|(e, _)| format!("login failed: {e}"))
}

/// Reads the greeting, sends `STARTTLS`, and returns the socket once the server says OK.
fn starttls_by_hand(tcp: TcpStream) -> Result<TcpStream, String> {
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(tcp.try_clone().map_err(|e| e.to_string())?);
    let mut writer = tcp;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("reading the greeting: {e}"))?;
    if !line.starts_with("* OK") && !line.starts_with("* PREAUTH") {
        return Err(format!("unexpected greeting: {}", line.trim_end()));
    }
    writer.write_all(b"s1 STARTTLS\r\n").map_err(|e| format!("sending STARTTLS: {e}"))?;
    writer.flush().map_err(|e| e.to_string())?;
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| format!("reading the STARTTLS reply: {e}"))? == 0 {
            return Err("server closed the connection during STARTTLS".into());
        }
        if line.starts_with("s1 OK") {
            return Ok(writer);
        }
        if line.starts_with("s1 ") {
            return Err(format!("STARTTLS refused: {}", line.trim_end()));
        }
    }
}

fn attribute_name(a: &NameAttribute<'_>) -> String {
    match a {
        NameAttribute::NoInferiors => "\\Noinferiors".into(),
        NameAttribute::NoSelect => "\\Noselect".into(),
        NameAttribute::Marked => "\\Marked".into(),
        NameAttribute::Unmarked => "\\Unmarked".into(),
        NameAttribute::All => "\\All".into(),
        NameAttribute::Archive => "\\Archive".into(),
        NameAttribute::Drafts => "\\Drafts".into(),
        NameAttribute::Flagged => "\\Flagged".into(),
        NameAttribute::Junk => "\\Junk".into(),
        NameAttribute::Sent => "\\Sent".into(),
        NameAttribute::Trash => "\\Trash".into(),
        NameAttribute::Extension(s) => s.to_string(),
        other => format!("{other:?}"),
    }
}

fn addresses(a: Option<&mail_parser::Address<'_>>) -> String {
    a.map(|a| a.iter().map(|x| format!("{} <{}>", x.name().unwrap_or(""), x.address().unwrap_or(""))).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

struct Parsed {
    subject: String,
    from: String,
    message_id: Option<String>,
    references: usize,
    text_len: usize,
    html_len: usize,
    attachments: Vec<(String, String, usize)>,
    has_calendar: bool,
}

fn parse(raw: &[u8]) -> Option<Parsed> {
    let m = MessageParser::default().parse(raw)?;
    let attachments = m
        .attachments()
        .map(|p| (p.attachment_name().unwrap_or("(unnamed)").to_owned(), p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or(""))).unwrap_or_else(|| "?".into()), p.contents().len()))
        .collect();
    let has_calendar = m.parts.iter().any(|p| p.is_content_type("text", "calendar"));
    Some(Parsed {
        subject: m.subject().unwrap_or("(no subject)").to_owned(),
        from: addresses(m.from()),
        message_id: m.message_id().map(str::to_owned),
        references: m.references().as_text_list().map(|l| l.len()).unwrap_or(0) + m.in_reply_to().as_text_list().map(|l| l.len()).unwrap_or(0),
        text_len: m.body_text(0).map(|t| t.len()).unwrap_or(0),
        html_len: m.body_html(0).map(|t| t.len()).unwrap_or(0),
        attachments,
        has_calendar,
    })
}

fn main() {
    if let Err(e) = run() {
        eprintln!("spike failed: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let host = required("SPIKE_IMAP_HOST");
    let security = Security::parse(env("SPIKE_IMAP_SECURITY"), Security::Tls);
    let port: u16 = env("SPIKE_IMAP_PORT").map(|p| p.parse().expect("SPIKE_IMAP_PORT must be a number")).unwrap_or(match security {
        Security::Tls => 993,
        _ => 143,
    });
    let user = required("SPIKE_USER");
    let password = required("SPIKE_PASSWORD");
    let fetch_count: usize = env("SPIKE_FETCH").map(|n| n.parse().expect("SPIKE_FETCH must be a number")).unwrap_or(10);

    // ---------------------------------------------------------------- connect
    let t = Instant::now();
    let mut session = imap_session(&host, port, security, &user, &password)?;
    println!("connected and logged in to {host}:{port} ({security:?}) in {:?}", t.elapsed());

    let caps = session.capabilities().map_err(|e| format!("CAPABILITY: {e}"))?;
    let interesting = ["IDLE", "MOVE", "UIDPLUS", "CONDSTORE", "QRESYNC", "SPECIAL-USE", "LIST-STATUS", "ENABLE", "COMPRESS=DEFLATE", "LITERAL+", "ID"];
    let have: Vec<&str> = interesting.iter().copied().filter(|c| caps.has_str(c)).collect();
    let missing: Vec<&str> = interesting.iter().copied().filter(|c| !caps.has_str(c)).collect();
    println!("capabilities of interest: have {have:?}; missing {missing:?}");

    // ------------------------------------------------------------------ folders
    let t = Instant::now();
    let names = session.list(Some(""), Some("*")).map_err(|e| format!("LIST: {e}"))?;
    println!("{} folders listed in {:?}:", names.len(), t.elapsed());
    let mut sent_folder: Option<String> = None;
    for n in names.iter() {
        let attrs: Vec<String> = n.attributes().iter().map(attribute_name).collect();
        println!("  {:<40} delimiter={:?} {}", n.name(), n.delimiter(), attrs.join(" "));
        if n.attributes().iter().any(|a| matches!(a, NameAttribute::Sent)) {
            sent_folder = Some(n.name().to_owned());
        }
    }
    if sent_folder.is_none() {
        sent_folder = names.iter().map(|n| n.name().to_owned()).find(|n| {
            let last = n.rsplit(['/', '.']).next().unwrap_or(n).to_ascii_lowercase();
            matches!(last.as_str(), "sent" | "sent items" | "sent messages" | "sent mail")
        });
    }
    println!("sent folder: {}", sent_folder.as_deref().unwrap_or("(none found; would create 'Sent')"));

    // -------------------------------------------------------------------- inbox
    let t = Instant::now();
    let mailbox = session.select("INBOX").map_err(|e| format!("SELECT INBOX: {e}"))?;
    println!("INBOX: exists={} uidvalidity={:?} uidnext={:?} in {:?}", mailbox.exists, mailbox.uid_validity, mailbox.uid_next, t.elapsed());

    let t = Instant::now();
    let mut uids: Vec<u32> = session.uid_search("ALL").map_err(|e| format!("UID SEARCH ALL: {e}"))?.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    println!("UID SEARCH ALL returned {} uids in {:?}", uids.len(), t.elapsed());
    let newest: Vec<String> = uids.iter().take(fetch_count).map(|u| u.to_string()).collect();
    if newest.is_empty() {
        println!("INBOX is empty; nothing to fetch");
    } else {
        // Headers first (what the sync engine stores before bodies arrive).
        let t = Instant::now();
        let set = newest.join(",");
        let heads = session.uid_fetch(&set, "(UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE)").map_err(|e| format!("UID FETCH envelope: {e}"))?;
        println!("fetched {} envelopes in {:?}", heads.len(), t.elapsed());
        for f in heads.iter() {
            let subject = f.envelope().and_then(|e| e.subject.as_ref()).map(|s| String::from_utf8_lossy(s).into_owned()).unwrap_or_default();
            println!("  uid={:?} size={:?} flags={:?} date={:?} subject={subject:?}", f.uid, f.size, f.flags(), f.internal_date().map(|d| d.to_rfc3339()));
        }

        // Then full bodies, parsed with mail-parser.
        let t = Instant::now();
        let bodies = session.uid_fetch(&set, "(UID BODY.PEEK[])").map_err(|e| format!("UID FETCH body: {e}"))?;
        let total: usize = bodies.iter().filter_map(|f| f.body()).map(|b| b.len()).sum();
        println!("fetched {} bodies ({} bytes) in {:?}", bodies.len(), total, t.elapsed());
        let t = Instant::now();
        let mut parsed = 0;
        for f in bodies.iter() {
            let Some(raw) = f.body() else {
                println!("  uid={:?}: no body returned", f.uid);
                continue;
            };
            match parse(raw) {
                Some(p) => {
                    parsed += 1;
                    println!("  uid={:?} {:?} from {} id={:?} refs={} text={}B html={}B calendar={} attachments={:?}", f.uid, p.subject, p.from, p.message_id, p.references, p.text_len, p.html_len, p.has_calendar, p.attachments);
                }
                None => println!("  uid={:?}: mail-parser returned None for {} bytes", f.uid, raw.len()),
            }
        }
        println!("parsed {parsed}/{} messages in {:?}", bodies.len(), t.elapsed());
    }

    // --------------------------------------------------------------------- send
    if let Some(to) = env("SPIKE_SEND_TO") {
        let smtp_host = required("SPIKE_SMTP_HOST");
        let smtp_security = Security::parse(env("SPIKE_SMTP_SECURITY"), Security::StartTls);
        let smtp_port: u16 = env("SPIKE_SMTP_PORT").map(|p| p.parse().expect("SPIKE_SMTP_PORT must be a number")).unwrap_or(match smtp_security {
            Security::Tls => 465,
            Security::StartTls => 587,
            Security::Plain => 25,
        });
        let from = env("SPIKE_FROM").unwrap_or_else(|| user.clone());
        let message_id = format!("<spike-{}@overlook.lastrefuge.ai>", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
        let email = lettre::Message::builder()
            .from(from.parse().map_err(|e| format!("SPIKE_FROM: {e}"))?)
            .to(to.parse().map_err(|e| format!("SPIKE_SEND_TO: {e}"))?)
            .subject("Overlook IMAP/SMTP spike")
            .message_id(Some(message_id.clone()))
            .body(String::from("This message was sent by the Overlook M0 spike over SMTP, then appended to Sent over IMAP.\n"))
            .map_err(|e| format!("building the message: {e}"))?;
        let raw = email.formatted();

        use lettre::transport::smtp::authentication::Credentials;
        use lettre::transport::smtp::client::{Tls, TlsParameters};
        use lettre::{SmtpTransport, Transport};
        let tls_params = TlsParameters::new(smtp_host.clone()).map_err(|e| format!("SMTP TLS parameters: {e}"))?;
        let builder = SmtpTransport::builder_dangerous(&smtp_host).port(smtp_port).timeout(Some(TIMEOUT)).credentials(Credentials::new(user.clone(), password.clone()));
        let transport = match smtp_security {
            Security::Tls => builder.tls(Tls::Wrapper(tls_params)),
            Security::StartTls => builder.tls(Tls::Required(tls_params)),
            Security::Plain => builder.tls(Tls::None),
        }
        .build();
        let t = Instant::now();
        let response = transport.send(&email).map_err(|e| format!("SMTP send to {smtp_host}:{smtp_port} ({smtp_security:?}): {e}"))?;
        println!("SMTP accepted in {:?}: {} {}", t.elapsed(), response.code(), response.first_line().unwrap_or(""));

        match &sent_folder {
            Some(folder) => {
                let t = Instant::now();
                let appended = session.append(folder, &raw).flag(imap::types::Flag::Seen).finish().map_err(|e| format!("APPEND to {folder}: {e}"))?;
                println!("APPEND to {folder:?} in {:?}: uidvalidity={:?} uids={:?}", t.elapsed(), appended.uid_validity, appended.uids);
                // The reconcile path: find the message by Message-ID in Sent.
                session.select(folder).map_err(|e| format!("SELECT {folder}: {e}"))?;
                let found = session.uid_search(format!("HEADER Message-ID {}", message_id)).map_err(|e| format!("UID SEARCH HEADER: {e}"))?;
                println!("UID SEARCH HEADER Message-ID in {folder:?}: {:?}", found);
            }
            None => println!("no Sent folder; skipped APPEND"),
        }
    } else {
        println!("SPIKE_SEND_TO not set; skipped SMTP");
    }

    session.logout().map_err(|e| format!("LOGOUT: {e}"))?;
    println!("done");
    Ok(())
}
