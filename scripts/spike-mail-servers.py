#!/usr/bin/env python3
"""Scripted plaintext IMAP (port 1143) and SMTP (port 2525) servers for the Overlook M0 spike.

Not a real server: it answers the handful of commands the spike issues with Dovecot-shaped
responses. User: spike / password: secret.
"""
import base64, email, email.policy, re, socket, socketserver, threading, time, sys

IMAP_PORT, SMTP_PORT = 1143, 2525
USER, PASSWORD = "spike", "secret"

def msg(headers, body):
    return ("\r\n".join(headers) + "\r\n\r\n" + body).encode()

MESSAGES = {  # uid -> (flags, internaldate, bytes)
    1: ("\\Seen", "10-Oct-2026 09:12:00 +0000", msg([
        "From: Ada Lovelace <ada@example.com>", "To: spike@example.com", "Subject: Plain hello",
        "Date: Sat, 10 Oct 2026 09:12:00 +0000", "Message-ID: <plain-1@example.com>",
        "Content-Type: text/plain; charset=utf-8"], "Hello from a plain message.\r\n")),
    2: ("", "12-Oct-2026 14:30:00 +0000", msg([
        "From: =?UTF-8?Q?Gr=C3=A9goire?= <greg@example.com>", "To: spike@example.com", "Cc: Ada Lovelace <ada@example.com>",
        "Subject: =?UTF-8?B?UmU6IFBsYWluIGhlbGxv?=", "Date: Mon, 12 Oct 2026 14:30:00 +0000",
        "Message-ID: <multi-2@example.com>", "In-Reply-To: <plain-1@example.com>", "References: <plain-1@example.com>",
        "MIME-Version: 1.0", 'Content-Type: multipart/mixed; boundary="outer"'],
        '--outer\r\nContent-Type: multipart/alternative; boundary="inner"\r\n\r\n'
        '--inner\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nText part.\r\n'
        '--inner\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML part with <img src="cid:logo"></p>\r\n'
        '--inner--\r\n'
        '--outer\r\nContent-Type: image/png; name="logo.png"\r\nContent-ID: <logo>\r\nContent-Disposition: inline; filename="logo.png"\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n'
        '--outer\r\nContent-Type: application/pdf; name="report.pdf"\r\nContent-Disposition: attachment; filename="report.pdf"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n'
        '--outer--\r\n')),
    3: ("\\Flagged", "13-Oct-2026 08:00:00 +0000", msg([
        "From: Calendar <cal@example.com>", "To: spike@example.com", "Subject: Invitation: Standup",
        "Date: Tue, 13 Oct 2026 08:00:00 +0000", "Message-ID: <invite-3@example.com>", "MIME-Version: 1.0",
        'Content-Type: multipart/alternative; boundary="b"'],
        '--b\r\nContent-Type: text/plain\r\n\r\nYou are invited.\r\n'
        '--b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:ev1\r\nSUMMARY:Standup\r\nDTSTART:20261014T090000Z\r\nDTEND:20261014T091500Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n'
        '--b--\r\n')),
}
SENT = {}  # uid -> bytes
SENT_NEXT = [1]
LOCK = threading.Lock()

def header(raw, name):
    for line in raw.split(b"\r\n\r\n", 1)[0].split(b"\r\n"):
        if line.lower().startswith(name.lower().encode() + b":"):
            return line.split(b":", 1)[1].strip().decode()
    return None

def quoted(s):
    return "NIL" if s is None else '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'

def addr(h):
    if not h: return "NIL"
    out = []
    for part in h.split(","):
        part = part.strip()
        if "<" in part:
            name, mail = part.split("<", 1); mail = mail.rstrip(">")
            name = name.strip().strip('"') or None
        else:
            name, mail = None, part
        local, _, host = mail.partition("@")
        out.append(f"({quoted(name)} NIL {quoted(local)} {quoted(host)})")
    return "(" + "".join(out) + ")"

def envelope(raw):
    frm = addr(header(raw, "From"))
    return (f"({quoted(header(raw,'Date'))} {quoted(header(raw,'Subject'))} {frm} {frm} {frm} "
            f"{addr(header(raw,'To'))} {addr(header(raw,'Cc'))} NIL {quoted(header(raw,'In-Reply-To'))} {quoted(header(raw,'Message-ID'))})")

def section_bytes(raw, spec):
    """BODY[<spec>] bytes: '' (whole), HEADER, TEXT, n.m, n.m.MIME — served from the raw
    message so the client sees what a real server would send."""
    head, _, body = raw.partition(b"\r\n\r\n")
    if spec == "": return raw
    if spec.upper() == "HEADER": return head + b"\r\n\r\n"
    if spec.upper() == "TEXT": return body
    msg = email.message_from_bytes(raw, policy=email.policy.compat32)
    want_mime = spec.upper().endswith(".MIME")
    numbers = [int(n) for n in (spec[:-5] if want_mime else spec).split(".")]
    part = msg
    for n in numbers:
        if part.is_multipart(): part = part.get_payload()[n - 1]
        elif n != 1: return b""
    part_bytes = part.as_bytes()
    phead, _, pbody = part_bytes.partition(b"\r\n\r\n")
    if b"\r\n\r\n" not in part_bytes:  # compat32 may emit bare LF
        phead, _, pbody = part_bytes.partition(b"\n\n")
        phead = phead.replace(b"\n", b"\r\n"); pbody = pbody.replace(b"\n", b"\r\n")
    return phead + b"\r\n" if want_mime else pbody

def parse_set(s, uids):
    out = set()
    for piece in s.split(","):
        if ":" in piece:
            a, b = piece.split(":"); a = int(a); b = max(uids) if b == "*" else int(b)
            out.update(u for u in uids if min(a,b) <= u <= max(a,b))
        else:
            out.add(int(piece))
    return sorted(u for u in out if u in uids)

class Imap(socketserver.StreamRequestHandler):
    def send(self, s):
        self.wfile.write(s.encode() if isinstance(s, str) else s); self.wfile.write(b"\r\n")
    def handle(self):
        self.send("* OK [CAPABILITY IMAP4rev1 UIDPLUS MOVE IDLE SPECIAL-USE LITERAL+ ENABLE] Spike IMAP ready.")
        box = None
        while True:
            line = self.rfile.readline()
            if not line: return
            text = line.decode(errors="replace").rstrip("\r\n")
            print("IMAP <", text if "LOGIN" not in text.upper() else text.split()[0] + " LOGIN ****", flush=True)
            tag, _, rest = text.partition(" ")
            cmd, _, args = rest.partition(" ")
            cmd = cmd.upper()
            if cmd == "CAPABILITY":
                self.send("* CAPABILITY IMAP4rev1 UIDPLUS MOVE IDLE SPECIAL-USE LITERAL+ ENABLE"); self.send(f"{tag} OK Capability completed.")
            elif cmd == "LOGIN":
                u, p = [a.strip('"') for a in args.split(" ", 1)]
                if (u, p) == (USER, PASSWORD): self.send(f"{tag} OK [CAPABILITY IMAP4rev1 UIDPLUS MOVE IDLE SPECIAL-USE] Logged in")
                else: self.send(f"{tag} NO [AUTHENTICATIONFAILED] Authentication failed.")
            elif cmd == "STARTTLS":
                self.send(f"{tag} NO STARTTLS is not offered by the fake server")
            elif cmd == "LIST":
                self.send('* LIST (\\HasNoChildren) "/" INBOX')
                self.send('* LIST (\\HasNoChildren \\Sent) "/" Sent')
                self.send('* LIST (\\HasNoChildren \\Trash) "/" Trash')
                self.send('* LIST (\\HasNoChildren \\Junk) "/" Junk')
                self.send('* LIST (\\HasChildren) "/" Work')
                self.send('* LIST (\\HasNoChildren) "/" Work/Clients')
                self.send(f"{tag} OK List completed.")
            elif cmd in ("SELECT", "EXAMINE"):
                box = args.strip('"')
                store = MESSAGES if box == "INBOX" else SENT if box == "Sent" else {}
                with LOCK: n = len(store); nxt = (max(store) + 1) if store else 1
                self.send("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)")
                self.send("* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted.")
                self.send(f"* {n} EXISTS"); self.send("* 0 RECENT")
                self.send("* OK [UIDVALIDITY 1760400000] UIDs valid"); self.send(f"* OK [UIDNEXT {nxt}] Predicted next UID")
                self.send(f"{tag} OK [READ-WRITE] {cmd.title()} completed")
            elif cmd == "UID":
                sub, _, sargs = args.partition(" ")
                store = MESSAGES if box == "INBOX" else SENT
                if sub.upper() == "SEARCH":
                    crit = sargs.upper()
                    with LOCK:
                        if crit == "ALL": hits = sorted(store)
                        elif crit.startswith("HEADER MESSAGE-ID"):
                            want = sargs.split(None, 2)[2].strip('"')
                            hits = [u for u, v in store.items() if header(v if isinstance(v, bytes) else v[2], "Message-ID") == want]
                        else: hits = []
                    self.send("* SEARCH" + "".join(f" {u}" for u in hits)); self.send(f"{tag} OK Search completed.")
                elif sub.upper() == "FETCH":
                    sset, _, items = sargs.partition(" ")
                    items = items.strip("()").upper()
                    with LOCK: uids = parse_set(sset, store.keys())
                    for i, u in enumerate(sorted(store)):
                        if u not in uids: continue
                        flags, idate, raw = store[u] if isinstance(store[u], tuple) else ("\\Seen", "14-Oct-2026 00:00:00 +0000", store[u])
                        parts = [f"UID {u}"]
                        if "FLAGS" in items: parts.append(f"FLAGS ({flags})")
                        if "INTERNALDATE" in items: parts.append(f'INTERNALDATE "{idate}"')
                        if "RFC822.SIZE" in items: parts.append(f"RFC822.SIZE {len(raw)}")
                        if "ENVELOPE" in items: parts.append("ENVELOPE " + envelope(raw))
                        head = f"* {i+1} FETCH (" + " ".join(parts)
                        sections = re.findall(r"BODY(?:\.PEEK)?\[([^\]]*)\](<0\.(\d+)>)?", items)
                        if sections:
                            self.wfile.write(head.encode())
                            for spec, partial, limit in sections:
                                data = section_bytes(raw, spec)
                                if partial: data = data[:int(limit)]
                                self.wfile.write(f" BODY[{spec}]{'<0>' if partial else ''} {{{len(data)}}}\r\n".encode()); self.wfile.write(data)
                            self.send(")")
                        else:
                            self.send(head + ")")
                    self.send(f"{tag} OK Fetch completed.")
                else:
                    self.send(f"{tag} BAD Unknown UID command")
            elif cmd == "APPEND":
                # a APPEND "Sent" (\Seen) {N}  or with LITERAL+ {N+}
                size = int(args[args.rindex("{")+1:].rstrip("}+ "))
                if not args.rstrip().endswith("+}"): self.send("+ OK")
                raw = self.rfile.read(size); self.rfile.readline()
                with LOCK:
                    uid = SENT_NEXT[0]; SENT_NEXT[0] += 1; SENT[uid] = raw
                self.send(f"{tag} OK [APPENDUID 1760400000 {uid}] Append completed.")
            elif cmd == "NOOP":
                self.send(f"{tag} OK NOOP completed.")
            elif cmd == "LOGOUT":
                self.send("* BYE Logging out"); self.send(f"{tag} OK Logout completed."); return
            else:
                self.send(f"{tag} BAD Unknown command {cmd}")

class Smtp(socketserver.StreamRequestHandler):
    def send(self, s): self.wfile.write((s + "\r\n").encode())
    def handle(self):
        self.send("220 spike.local ESMTP fake")
        authed = False
        while True:
            line = self.rfile.readline()
            if not line: return
            text = line.decode(errors="replace").rstrip("\r\n")
            print("SMTP <", text if not text.startswith(("AUTH", "dXNlc", "c3Bpa")) and not (len(text) > 8 and " " not in text) else "(auth data)", flush=True)
            cmd = text.split(" ", 1)[0].upper()
            if cmd in ("EHLO", "HELO"):
                self.send("250-spike.local"); self.send("250-AUTH PLAIN LOGIN"); self.send("250-8BITMIME"); self.send("250 SIZE 10485760")
            elif cmd == "AUTH":
                parts = text.split(" ")
                if parts[1].upper() == "PLAIN":
                    blob = parts[2] if len(parts) > 2 else (self.send("334 ") or self.rfile.readline().decode().strip())
                    _, u, p = base64.b64decode(blob).split(b"\0")
                    authed = (u.decode(), p.decode()) == (USER, PASSWORD)
                elif parts[1].upper() == "LOGIN":
                    self.send("334 VXNlcm5hbWU6"); u = base64.b64decode(self.rfile.readline().strip()).decode()
                    self.send("334 UGFzc3dvcmQ6"); p = base64.b64decode(self.rfile.readline().strip()).decode()
                    authed = (u, p) == (USER, PASSWORD)
                self.send("235 2.7.0 Authentication successful" if authed else "535 5.7.8 Authentication credentials invalid")
            elif cmd in ("MAIL", "RCPT"):
                self.send("250 2.1.0 OK" if authed else "530 5.7.0 Authentication required")
            elif cmd == "DATA":
                self.send("354 End data with <CR><LF>.<CR><LF>")
                data = b""
                while not data.endswith(b"\r\n.\r\n"):
                    chunk = self.rfile.readline()
                    if not chunk: return
                    data += chunk
                print(f"SMTP received {len(data)} bytes, subject={header(data, 'Subject')!r}", flush=True)
                self.send("250 2.0.0 OK: queued as spike-1")
            elif cmd == "STARTTLS":
                self.send("454 TLS not available")
            elif cmd == "QUIT":
                self.send("221 2.0.0 Bye"); return
            elif cmd in ("RSET", "NOOP"):
                self.send("250 OK")
            else:
                self.send("502 5.5.2 Command not implemented")

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

if __name__ == "__main__":
    imap = Server(("127.0.0.1", IMAP_PORT), Imap); smtp = Server(("127.0.0.1", SMTP_PORT), Smtp)
    threading.Thread(target=imap.serve_forever, daemon=True).start()
    threading.Thread(target=smtp.serve_forever, daemon=True).start()
    print(f"fake IMAP on 127.0.0.1:{IMAP_PORT}, fake SMTP on 127.0.0.1:{SMTP_PORT}; user {USER}", flush=True)
    try:
        while True: time.sleep(3600)
    except KeyboardInterrupt:
        pass
