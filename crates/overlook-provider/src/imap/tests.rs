//! `ImapProvider` against the scripted fake server, plus an ignored round trip against a
//! real server configured through `OVERLOOK_IMAP_TEST_*`.

use super::fake::{serve, FakeFolder, FakeMessage, FakeServer, FakeState, Login};
use super::*;
use crate::ProviderError;
use std::time::Duration;

fn account() -> Account {
    Account { id: AccountId::from("imap:jason@example.com"), provider: ProviderKind::Imap, display_name: "Jason".into(), email: "jason@example.com".into(), created_at: 0 }
}

fn msg(headers: &[&str], body: &str) -> Vec<u8> {
    format!("{}\r\n\r\n{}", headers.join("\r\n"), body).into_bytes()
}

const MULTIPART_BODY: &str = "--outer\r\nContent-Type: multipart/alternative; boundary=\"inner\"\r\n\r\n--inner\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nText part.\r\n--inner\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML part with <img src=\"cid:logo\"></p>\r\n--inner--\r\n--outer\r\nContent-Type: image/png; name=\"logo.png\"\r\nContent-ID: <logo>\r\nContent-Disposition: inline; filename=\"logo.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--outer\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--outer--\r\n";

fn mailbox() -> FakeState {
    let mut state = FakeState::default();
    let mut inbox = FakeFolder::new("INBOX", &["\\HasNoChildren"], 1760400000);
    inbox.add(FakeMessage::new(&["\\Seen"], "10-Oct-2026 09:12:00 +0000", msg(&["From: Ada Lovelace <ada@example.com>", "To: jason@example.com", "Subject: Plain hello", "Date: Sat, 10 Oct 2026 09:12:00 +0000", "Message-ID: <plain-1@example.com>", "Content-Type: text/plain; charset=utf-8"], "Hello from a plain message.\r\n")));
    inbox.add(FakeMessage::new(
        &[],
        "12-Oct-2026 14:30:00 +0000",
        msg(
            &["From: =?UTF-8?Q?Gr=C3=A9goire?= <greg@example.com>", "To: jason@example.com", "Subject: =?UTF-8?B?UmU6IFBsYWluIGhlbGxv?=", "Date: Mon, 12 Oct 2026 14:30:00 +0000", "Message-ID: <multi-2@example.com>", "In-Reply-To: <plain-1@example.com>", "References: <plain-1@example.com>", "MIME-Version: 1.0", "Content-Type: multipart/mixed; boundary=\"outer\""],
            MULTIPART_BODY,
        ),
    ));
    inbox.add(FakeMessage::new(
        &["\\Flagged"],
        "13-Oct-2026 08:00:00 +0000",
        msg(
            &["From: Calendar <cal@example.com>", "To: jason@example.com", "Subject: Invitation: Standup", "Date: Tue, 13 Oct 2026 08:00:00 +0000", "Message-ID: <invite-3@example.com>", "MIME-Version: 1.0", "Content-Type: multipart/alternative; boundary=\"b\""],
            "--b\r\nContent-Type: text/plain\r\n\r\nYou are invited.\r\n--b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:ev1\r\nSUMMARY:Standup\r\nDTSTART:20261014T090000Z\r\nDTEND:20261014T091500Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n",
        ),
    ));
    for i in 4..=5 {
        inbox.add(FakeMessage::new(&[], "14-Oct-2026 10:00:00 +0000", msg(&["From: bot@example.com", "To: jason@example.com", &format!("Subject: Note {i}"), &format!("Message-ID: <note-{i}@example.com>"), &format!("Date: Wed, 14 Oct 2026 10:0{i}:00 +0000")], "body\r\n")));
    }
    let mut sent = FakeFolder::new("Sent", &["\\HasNoChildren", "\\Sent"], 1760400001);
    sent.add(FakeMessage::new(&["\\Seen"], "11-Oct-2026 09:00:00 +0000", msg(&["From: Jason <jason@example.com>", "To: ada@example.com", "Subject: Re: Plain hello", "Message-ID: <sent-1@example.com>", "Date: Sun, 11 Oct 2026 09:00:00 +0000"], "Thanks Ada.\r\n")));
    let mut work = FakeFolder::new("Work/Clients", &["\\HasNoChildren"], 1760400002);
    work.add(FakeMessage::new(&[], "09-Oct-2026 09:00:00 +0000", msg(&["From: client@example.com", "To: jason@example.com", "Subject: Contract", "Message-ID: <work-1@example.com>", "Date: Fri, 09 Oct 2026 09:00:00 +0000"], "Please sign.\r\n")));
    work.add(FakeMessage::new(&["\\Seen"], "09-Oct-2026 10:00:00 +0000", msg(&["From: client@example.com", "To: jason@example.com", "Subject: Contract signed", "Message-ID: <work-2@example.com>", "Date: Fri, 09 Oct 2026 10:00:00 +0000"], "Done.\r\n")));
    state.folders = vec![inbox, sent, FakeFolder::new("Trash", &["\\HasNoChildren", "\\Trash"], 1760400003), FakeFolder::new("Drafts", &["\\HasNoChildren", "\\Drafts"], 1760400004), FakeFolder::new("[Gmail]", &["\\Noselect", "\\HasChildren"], 0), FakeFolder::new("Work", &["\\HasChildren"], 1760400005), work];
    state.subscribed = ["INBOX", "Sent", "Trash", "Work/Clients"].into_iter().map(str::to_owned).collect();
    state
}

fn provider(server: &FakeServer) -> ImapProvider {
    let servers = MailServers {
        incoming: ServerEndpoint::new("127.0.0.1", server.addr.port(), Security::None, "jason@example.com"),
        outgoing: ServerEndpoint::new("127.0.0.1", 1, Security::None, "jason@example.com"),
        pop3: None,
        imap: None,
    };
    ImapProvider::new(servers, "hunter2").with_timeout(Duration::from_secs(5)).with_inbox_limit(20)
}

fn block<T>(f: BoxFuture<'_, T>) -> T {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut f = f;
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn pid(folder: &str, uidvalidity: u32, uid: u32) -> ProviderId {
    MessageRef::provider_id(folder, uidvalidity, uid)
}

#[test]
fn snapshot_maps_folders_and_queues_newest_messages() {
    let server = serve(mailbox());
    let p = provider(&server);
    block(p.probe()).unwrap();
    let caps = p.capabilities();
    assert!(caps.special_use && caps.move_ && caps.uidplus && caps.idle && !caps.condstore);

    let snap = block(p.fetch_snapshot(&account())).unwrap();
    let names: Vec<(&str, LabelKind)> = snap.labels.iter().map(|l| (l.name.as_str(), l.kind)).collect();
    assert_eq!(names, vec![("Inbox", LabelKind::Inbox), ("Sent", LabelKind::Sent), ("Trash", LabelKind::Trash), ("Work", LabelKind::User), ("Work / Clients", LabelKind::User)], "Drafts and the unselectable [Gmail] container are not labels");
    assert_eq!(snap.labels[0].id.as_str(), "imap:jason@example.com:INBOX");
    assert_eq!(snap.labels[4].provider_id.as_ref().unwrap().as_str(), "Work/Clients");
    assert!(snap.messages.is_empty(), "bodies come through the backlog");
    let ids: Vec<&str> = snap.message_ids.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(ids[..5], ["INBOX\u{1f}1760400000\u{1f}5", "INBOX\u{1f}1760400000\u{1f}4", "INBOX\u{1f}1760400000\u{1f}3", "INBOX\u{1f}1760400000\u{1f}2", "INBOX\u{1f}1760400000\u{1f}1"], "Inbox first, newest first");
    assert_eq!(ids.len(), 5 + 1 + 2);

    let cp = Checkpoint::parse(snap.checkpoint.as_deref().unwrap()).unwrap();
    assert_eq!(cp.folders.len(), 5);
    let inbox = &cp.folders["INBOX"];
    assert_eq!((inbox.kind.as_str(), inbox.uidvalidity, inbox.highest_uid, inbox.known.as_str(), inbox.seen.as_str(), inbox.flagged.as_str()), ("inbox", 1760400000, 5, "1:5", "1", "3"));
    assert!(inbox.subscribed && !inbox.fresh);
    assert_eq!(cp.folders["Sent"].special_use.as_deref(), Some("\\Sent"));
    assert_eq!(cp.folders["Trash"].known, "");
    assert_eq!(cp.folders["Work/Clients"].seen, "2");

    let state = server.state.lock().unwrap();
    assert_eq!(state.connections, 1, "one connection serves probe and snapshot");
    assert_eq!(state.commands(" LOGIN "), 1);
    assert_eq!(state.commands("EXAMINE"), 5, "unselectable folders are never examined");
}

#[test]
fn fetch_messages_parses_bodies_flags_and_attachments() {
    let server = serve(mailbox());
    let p = provider(&server);
    let acct = account();
    let ids = vec![pid("INBOX", 1760400000, 2), pid("INBOX", 1760400000, 3), pid("INBOX", 1760400000, 1), pid("Work/Clients", 1760400002, 1), pid("INBOX", 999, 1), ProviderId("gmail-looking-id".into()), pid("INBOX", 1760400000, 77)];
    let messages = block(p.fetch_messages(&acct, &ids, &[])).unwrap();
    assert_eq!(messages.len(), 4, "stale UIDVALIDITY, malformed and vanished ids are skipped: {:?}", messages.iter().map(|m| m.subject.clone()).collect::<Vec<_>>());

    let multi = messages.iter().find(|m| m.provider_id.as_ref().unwrap().as_str() == "INBOX\u{1f}1760400000\u{1f}2").unwrap();
    assert!(multi.id.as_str().starts_with("imap:jason@example.com:"), "local ids are minted under the account: {}", multi.id);
    assert!(multi.conversation_id.0.is_empty(), "threading is the sync engine's job");
    assert_eq!(multi.subject, "Re: Plain hello");
    assert_eq!(multi.from.name.as_deref(), Some("Grégoire"));
    assert_eq!(multi.in_reply_to.as_deref(), Some("<plain-1@example.com>"));
    assert_eq!(multi.body.text.as_deref().map(str::trim), Some("Text part."));
    assert!(multi.body.html.as_deref().unwrap().contains("cid:logo"));
    assert_eq!(multi.attachments.len(), 2);
    assert_eq!((multi.attachments[0].filename.as_str(), multi.attachments[0].content_id.as_deref(), multi.attachments[0].provider_id.as_ref().unwrap().as_str()), ("logo.png", Some("logo"), "2"));
    assert_eq!((multi.attachments[1].filename.as_str(), multi.attachments[1].provider_id.as_ref().unwrap().as_str()), ("report.pdf", "3"));
    assert_eq!(multi.attachments[1].id.as_str(), format!("{}:att-1", multi.id));
    assert!(!multi.is_read && !multi.is_starred);
    assert_eq!(multi.label_ids.iter().map(|l| l.as_str()).collect::<Vec<_>>(), vec!["imap:jason@example.com:INBOX"]);

    let invite = messages.iter().find(|m| m.subject == "Invitation: Standup").unwrap();
    assert!(invite.is_starred && !invite.is_read);
    assert_eq!(invite.invitation.as_ref().map(|i| i.summary.clone()).flatten().as_deref(), Some("Standup"));

    let plain = messages.iter().find(|m| m.subject == "Plain hello").unwrap();
    assert!(plain.is_read);
    assert_eq!(plain.date, overlook_core::tz::parse_rfc3339("2026-10-10T09:12:00Z").unwrap());

    let work = messages.iter().find(|m| m.subject == "Contract").unwrap();
    assert_eq!(work.label_ids.iter().next().unwrap().as_str(), "imap:jason@example.com:Work/Clients");

    let state = server.state.lock().unwrap();
    assert_eq!(state.commands("EXAMINE"), 3, "INBOX once (not again for the stale batch), Work/Clients once, plus the stale UIDVALIDITY probe");
    assert_eq!(state.commands("RFC822.SIZE)"), 2, "sizes are checked per folder before bodies are fetched");
}

#[test]
fn large_messages_take_the_bodystructure_path() {
    let mut state = mailbox();
    let inbox = state.folder_mut("INBOX");
    let mut big = FakeMessage::new(
        &[],
        "14-Oct-2026 12:00:00 +0000",
        msg(&["From: big@example.com", "To: jason@example.com", "Subject: Big one", "Message-ID: <big@example.com>", "Date: Wed, 14 Oct 2026 12:00:00 +0000", "MIME-Version: 1.0", "Content-Type: multipart/mixed; boundary=\"outer\""], MULTIPART_BODY),
    );
    big.size_override = Some(3 * 1024 * 1024);
    big.bodystructure = Some(r#"((("TEXT" "PLAIN" ("CHARSET" "utf-8") NIL NIL "7BIT" 12 1 NIL NIL NIL NIL)("TEXT" "HTML" ("CHARSET" "utf-8") NIL NIL "7BIT" 40 1 NIL NIL NIL NIL) "ALTERNATIVE" ("BOUNDARY" "inner") NIL NIL NIL)("IMAGE" "PNG" ("NAME" "logo.png") "<logo>" NIL "BASE64" 12 NIL ("INLINE" ("FILENAME" "logo.png")) NIL NIL)("APPLICATION" "PDF" ("NAME" "report.pdf") NIL NIL "BASE64" 3000000 NIL ("ATTACHMENT" ("FILENAME" "report.pdf")) NIL NIL) "MIXED" ("BOUNDARY" "outer") NIL NIL NIL)"#.into());
    let uid = inbox.add(big);
    let server = serve(state);
    let p = provider(&server);
    let messages = block(p.fetch_messages(&account(), &[pid("INBOX", 1760400000, uid)], &[])).unwrap();
    assert_eq!(messages.len(), 1);
    let m = &messages[0];
    assert_eq!(m.subject, "Big one");
    assert_eq!(m.rfc_message_id.as_deref(), Some("<big@example.com>"));
    assert_eq!(m.body.text.as_deref().map(str::trim), Some("Text part."));
    assert!(m.body.html.as_deref().unwrap().contains("HTML part"));
    assert_eq!(m.attachments.len(), 2);
    assert_eq!((m.attachments[0].filename.as_str(), m.attachments[0].provider_id.as_ref().unwrap().as_str(), m.attachments[0].content_id.as_deref()), ("logo.png", "2", Some("logo")));
    assert_eq!((m.attachments[1].filename.as_str(), m.attachments[1].size_bytes, m.attachments[1].provider_id.as_ref().unwrap().as_str()), ("report.pdf", 3_000_000, "3"));
    let state = server.state.lock().unwrap();
    assert!(state.log.iter().any(|l| l.contains("BODYSTRUCTURE") && l.contains("BODY.PEEK[HEADER]")), "{:?}", state.log);
    assert!(state.log.iter().any(|l| l.contains("BODY.PEEK[1.1]<0.") && l.contains("BODY.PEEK[1.1.MIME]") && l.contains("BODY.PEEK[1.2]<0.")), "{:?}", state.log);
    assert!(!state.log.iter().any(|l| l.contains("BODY.PEEK[])")), "the whole message is never fetched");
}

#[test]
fn changes_report_new_mail_flags_and_deletions() {
    let server = serve(mailbox());
    let p = provider(&server);
    let acct = account();
    let snap = block(p.fetch_snapshot(&acct)).unwrap();
    let cp = snap.checkpoint.unwrap();

    // Nothing changed: nothing reported, cursor cycle advances.
    let quiet = block(p.fetch_changes(&acct, &cp)).unwrap();
    assert!(quiet.labels.is_none() && quiet.upserts.is_empty() && quiet.deleted_provider_ids.is_empty() && quiet.message_ids.is_empty());
    let cp = quiet.checkpoint.unwrap();
    assert_eq!(Checkpoint::parse(&cp).unwrap().cycle, 1);

    {
        let mut s = server.state.lock().unwrap();
        let inbox = s.folder_mut("INBOX");
        inbox.messages.get_mut(&2).unwrap().flags = vec!["\\Seen".into()];
        inbox.messages.remove(&3);
        inbox.add(FakeMessage::new(&[], "15-Oct-2026 08:00:00 +0000", msg(&["From: new@example.com", "To: jason@example.com", "Subject: Fresh", "Message-ID: <fresh@example.com>", "Date: Thu, 15 Oct 2026 08:00:00 +0000"], "new body\r\n")));
        // A flag change outside the Inbox is only picked up every tenth cycle.
        s.folder_mut("Work/Clients").messages.get_mut(&1).unwrap().flags = vec!["\\Flagged".into()];
    }
    let changes = block(p.fetch_changes(&acct, &cp)).unwrap();
    assert!(changes.labels.is_none(), "the folder list is reread hourly, not every cycle");
    assert_eq!(changes.deleted_provider_ids, vec![pid("INBOX", 1760400000, 3)]);
    assert_eq!(changes.message_ids, vec![pid("INBOX", 1760400000, 6)], "only new mail needs a body");
    let subjects: Vec<(&str, bool, bool)> = changes.upserts.iter().map(|m| (m.subject.as_str(), m.is_read, m.body.is_cached())).collect();
    assert_eq!(subjects, vec![("Fresh", false, false), ("Re: Plain hello", true, false)], "new mail first, then the flag change, both header-only");
    assert_eq!(changes.upserts[0].rfc_message_id.as_deref(), Some("<fresh@example.com>"));
    assert_eq!(changes.upserts[1].provider_id, Some(pid("INBOX", 1760400000, 2)));
    let parsed = Checkpoint::parse(changes.checkpoint.as_deref().unwrap()).unwrap();
    let inbox = &parsed.folders["INBOX"];
    assert_eq!((inbox.known.as_str(), inbox.seen.as_str(), inbox.flagged.as_str(), inbox.highest_uid), ("1:2,4:6", "1:2", "", 6));
    assert_eq!(parsed.folders["Work/Clients"].flagged, "", "not refreshed this cycle");

    // Cycle 10 refreshes flags everywhere.
    let mut cp10 = parsed.clone();
    cp10.cycle = 9;
    let changes = block(p.fetch_changes(&acct, &cp10.to_json())).unwrap();
    assert_eq!(changes.upserts.iter().map(|m| (m.subject.as_str(), m.is_starred)).collect::<Vec<_>>(), vec![("Contract", true)]);
    assert!(changes.message_ids.is_empty());
    assert_eq!(Checkpoint::parse(changes.checkpoint.as_deref().unwrap()).unwrap().folders["Work/Clients"].flagged, "1");
}

#[test]
fn uidvalidity_change_and_folder_list_changes() {
    let server = serve(mailbox());
    let p = provider(&server);
    let acct = account();
    let cp = block(p.fetch_snapshot(&acct)).unwrap().checkpoint.unwrap();

    {
        let mut s = server.state.lock().unwrap();
        s.folder_mut("Work/Clients").uidvalidity = 42;
    }
    let changes = block(p.fetch_changes(&acct, &cp)).unwrap();
    assert_eq!(changes.deleted_provider_ids, vec![pid("Work/Clients", 1760400002, 1), pid("Work/Clients", 1760400002, 2)]);
    assert_eq!(changes.message_ids, vec![pid("Work/Clients", 42, 2), pid("Work/Clients", 42, 1)]);
    assert!(changes.upserts.is_empty());
    let mut parsed = Checkpoint::parse(changes.checkpoint.as_deref().unwrap()).unwrap();
    assert_eq!((parsed.folders["Work/Clients"].uidvalidity, parsed.folders["Work/Clients"].seen.as_str()), (42, "2"));

    // An hour later the folder list is reread: Work/Clients is gone, Archive appeared.
    {
        let mut s = server.state.lock().unwrap();
        s.folders.retain(|f| f.path != "Work/Clients");
        let mut archive = FakeFolder::new("Archive", &["\\Archive"], 7);
        archive.add(FakeMessage::new(&["\\Seen"], "01-Oct-2026 08:00:00 +0000", msg(&["From: old@example.com", "Subject: Old", "Message-ID: <old@example.com>"], "old\r\n")));
        s.folders.push(archive);
    }
    parsed.listed_at = 0;
    let changes = block(p.fetch_changes(&acct, &parsed.to_json())).unwrap();
    let labels = changes.labels.expect("labels are resent after a re-list");
    assert_eq!(labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Inbox", "Sent", "Archive", "Trash", "Work"]);
    assert_eq!(changes.deleted_provider_ids, vec![pid("Work/Clients", 42, 1), pid("Work/Clients", 42, 2)]);
    assert_eq!(changes.message_ids, vec![pid("Archive", 7, 1)]);
    let parsed = Checkpoint::parse(changes.checkpoint.as_deref().unwrap()).unwrap();
    assert!(!parsed.folders.contains_key("Work/Clients"));
    assert_eq!((parsed.folders["Archive"].kind.as_str(), parsed.folders["Archive"].fresh, parsed.folders["Archive"].known.as_str()), ("archive", false, "1"));

    // A folder deleted between listings is noticed at once and triggers a re-list.
    {
        let mut s = server.state.lock().unwrap();
        s.folders.retain(|f| f.path != "Archive");
    }
    let changes = block(p.fetch_changes(&acct, &parsed.to_json())).unwrap();
    assert_eq!(changes.deleted_provider_ids, vec![pid("Archive", 7, 1)]);
    assert!(changes.labels.is_some_and(|l| !l.iter().any(|x| x.name == "Archive")));
    assert!(!Checkpoint::parse(changes.checkpoint.as_deref().unwrap()).unwrap().folders.contains_key("Archive"));
}

#[test]
fn attachments_and_raw_bytes_are_fetched_by_section() {
    let server = serve(mailbox());
    let p = provider(&server);
    let acct = account();
    let messages = block(p.fetch_messages(&acct, &[pid("INBOX", 1760400000, 2)], &[])).unwrap();
    let m = &messages[0];
    let pdf = m.attachments.iter().find(|a| a.filename == "report.pdf").unwrap();
    assert_eq!(block(p.fetch_attachment(&acct, m, pdf)).unwrap(), b"%PDF-1.4\n");
    let png = m.attachments.iter().find(|a| a.filename == "logo.png").unwrap();
    assert_eq!(block(p.fetch_attachment(&acct, m, png)).unwrap(), b"\x89PNG\r\n\x1a\n");
    let raw = block(p.fetch_raw(&acct, m)).unwrap();
    assert!(raw.starts_with(b"From: =?UTF-8?Q?Gr=C3=A9goire?=") && raw.ends_with(b"--outer--\r\n"));

    let mut stale = m.clone();
    stale.provider_id = Some(pid("INBOX", 1, 2));
    assert!(matches!(block(p.fetch_attachment(&acct, &stale, pdf)), Err(ProviderError::Rejected(_))));
    let rebuilt = block(p.fetch_raw(&acct, &stale)).unwrap();
    assert!(rebuilt.windows(16).any(|w| w == b"Re: Plain hello "[..16].as_ref() || w == b"Subject: Re: Pla"), "falls back to the stored message: {}", String::from_utf8_lossy(&rebuilt));
}

#[test]
fn server_failures_map_to_provider_errors() {
    let acct = account();
    let server = serve(FakeState { login: Login::AuthFailed, ..mailbox() });
    assert_eq!(block(provider(&server).probe()), Err(ProviderError::SignInRequired));
    let server = serve(FakeState { password: "other".into(), ..mailbox() });
    assert_eq!(block(provider(&server).probe()), Err(ProviderError::SignInRequired));
    let server = serve(FakeState { login: Login::Unavailable, ..mailbox() });
    assert_eq!(block(provider(&server).probe()), Err(ProviderError::Temporary("Temporary authentication failure".into())));
    let server = serve(FakeState { login: Login::Bye, ..mailbox() });
    assert_eq!(block(provider(&server).probe()), Err(ProviderError::Offline));
    let server = serve(FakeState { drop_next_command: true, ..mailbox() });
    assert_eq!(block(provider(&server).probe()), Err(ProviderError::Offline), "a fresh connection that dies is Offline");

    let unreachable = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let servers = MailServers { incoming: ServerEndpoint::new("127.0.0.1", unreachable.port(), Security::None, "j"), outgoing: ServerEndpoint::new("127.0.0.1", 1, Security::None, "j"), pop3: None, imap: None };
    assert_eq!(block(ImapProvider::new(servers, "pw").with_timeout(Duration::from_secs(2)).probe()), Err(ProviderError::Offline));

    // A folder the server reports as in use is a temporary failure, not a sign-in problem.
    let server = serve(mailbox());
    let p = provider(&server);
    let cp = block(p.fetch_snapshot(&acct)).unwrap().checkpoint.unwrap();
    server.state.lock().unwrap().in_use.insert("Sent".into());
    assert_eq!(block(p.fetch_changes(&acct, &cp)).err(), Some(ProviderError::Temporary("Mailbox is in use by another process".into())));

    // A Gmail history id or anything else that is not our checkpoint asks for a resync.
    assert_eq!(block(p.fetch_changes(&acct, "1234567")).err(), Some(ProviderError::CheckpointExpired));

    // Writes and sends are the next milestone.
    let op = PendingOperation { id: OperationId::from("op"), account_id: acct.id.clone(), seq: 1, kind: OperationKind::SetRead { message_ids: vec![], read: true }, state: OperationState::Pending, attempts: 0, last_error: None, created_at: 0, updated_at: 0 };
    assert!(matches!(block(p.apply(&acct, &op, &[], &[])), Err(ProviderError::Rejected(_))));
}

#[test]
fn a_dead_idle_connection_is_reopened_once() {
    let server = serve(mailbox());
    let p = provider(&server);
    block(p.probe()).unwrap();
    server.state.lock().unwrap().drop_next_command = true;
    block(p.probe()).unwrap();
    let state = server.state.lock().unwrap();
    assert_eq!(state.connections, 2);
    assert_eq!(state.commands(" LOGIN "), 2);
    drop(state);
    p.disconnect();
    assert!(server.state.lock().unwrap().log.iter().any(|l| l.contains("LOGOUT")));
}

/// Round trip against a real server. Run with
/// `OVERLOOK_IMAP_TEST_HOST=imap.example.com OVERLOOK_IMAP_TEST_USER=… OVERLOOK_IMAP_TEST_PASSWORD=…
/// cargo test -p overlook-provider real_imap -- --ignored --nocapture`; optional
/// `OVERLOOK_IMAP_TEST_PORT` and `OVERLOOK_IMAP_TEST_SECURITY=tls|starttls|none`.
#[test]
#[ignore]
fn real_imap_server_round_trip() {
    let var = |n: &str| std::env::var(n).ok().filter(|v| !v.trim().is_empty());
    let (Some(host), Some(user), Some(password)) = (var("OVERLOOK_IMAP_TEST_HOST"), var("OVERLOOK_IMAP_TEST_USER"), var("OVERLOOK_IMAP_TEST_PASSWORD")) else {
        panic!("set OVERLOOK_IMAP_TEST_HOST, OVERLOOK_IMAP_TEST_USER and OVERLOOK_IMAP_TEST_PASSWORD");
    };
    let security = var("OVERLOOK_IMAP_TEST_SECURITY").and_then(|s| Security::from_key(&s)).unwrap_or(Security::Tls);
    let port = var("OVERLOOK_IMAP_TEST_PORT").and_then(|p| p.parse().ok()).unwrap_or(ServerProtocol::Imap.default_port(security));
    let servers = MailServers { incoming: ServerEndpoint::new(host, port, security, user.clone()), outgoing: ServerEndpoint::new("", 0, security, user.clone()), pop3: None, imap: None };
    let p = ImapProvider::new(servers, password).with_inbox_limit(20);
    let acct = Account { id: AccountId(format!("imap:{user}")), provider: ProviderKind::Imap, display_name: user.clone(), email: user, created_at: 0 };
    block(p.probe()).unwrap();
    eprintln!("capabilities: {:?}", p.capabilities());
    let snap = block(p.fetch_snapshot(&acct)).unwrap();
    for l in &snap.labels {
        eprintln!("label {:?} {} ({:?})", l.kind, l.name, l.provider_id);
    }
    eprintln!("{} messages queued", snap.message_ids.len());
    let batch: Vec<ProviderId> = snap.message_ids.iter().take(20).cloned().collect();
    let messages = block(p.fetch_messages(&acct, &batch, &snap.labels)).unwrap();
    assert!(!messages.is_empty() || batch.is_empty());
    for m in &messages {
        eprintln!("{} | {} | {} attachments | text {}B html {}B", m.date, m.subject, m.attachments.len(), m.body.text.as_deref().map(str::len).unwrap_or(0), m.body.html.as_deref().map(str::len).unwrap_or(0));
    }
    if let Some((m, a)) = messages.iter().find_map(|m| m.attachments.first().map(|a| (m, a))) {
        let bytes = block(p.fetch_attachment(&acct, m, a)).unwrap();
        eprintln!("attachment {} ({}): {} bytes fetched, {} declared", a.filename, a.mime_type, bytes.len(), a.size_bytes);
        assert!(!bytes.is_empty());
    }
    let changes = block(p.fetch_changes(&acct, snap.checkpoint.as_deref().unwrap())).unwrap();
    eprintln!("changes: {} upserts, {} deleted, {} bodies queued", changes.upserts.len(), changes.deleted_provider_ids.len(), changes.message_ids.len());
    p.disconnect();
}
