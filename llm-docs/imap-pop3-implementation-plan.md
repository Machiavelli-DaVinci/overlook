# IMAP and POP3 Implementation Plan

Written 14 October 2026 against Overlook 0.4.2. This is the concrete sequence for phase 2 (IMAP + SMTP) and phase 3 (POP3) of the [product plan](native-linux-mail-client-plan.md), fitted to the code that exists today: the `MailProvider` trait, the per-account sync engine, the SQLite store, the keyring, and the GPUI settings dialog. Update the status log at the end as milestones land.

## What already exists and is reused unchanged

- `ProviderKind::Imap` and `ProviderKind::Pop3` exist in `overlook-core` and are persisted by the store. `providers_for` in `crates/overlook-app/src/providers.rs` currently returns "This account type is not supported yet." for both.
- `Folder` and `FolderId` exist in the model but are unused. This plan does **not** use `Folder` for the UI; IMAP folders become `Label` rows (see "Folders as labels"). `Folder` can be deleted or repurposed for the folder-state table.
- The sync engine (`crates/overlook-app/src/sync.rs`) is provider-agnostic: probe → `fetch_snapshot` or `fetch_changes` → persisted download backlog drained through `fetch_messages` in batches of 20 → replay pending operations in sequence → record state. Sends go through `send` / `reconcile_send` with the Uncertain state. Nothing in it names Gmail except two status strings.
- Local intent protection: `merge_remote_messages` keeps local read/star/label state for messages with pending operations. Works for IMAP as is.
- `lettre` is already a dependency (builder only) and `build_raw` in `crates/overlook-provider/src/gmail/mime.rs` already produces a complete RFC 822 message from a `Draft` (threading headers, attachments, HTML alternative). SMTP sending needs lettre's `smtp-transport` and `rustls-tls` features; `rustls` is already in the tree through `ureq`.
- `credentials.rs` stores one JSON secret per account under keyring attributes `(application, account, kind="oauth-tokens")`. Adding `kind="password"` entries is a small generalisation.
- The settings dialog has an Accounts page with an account list, per-account status, Sign in again / Sync now, and Remove.

## What is missing

| Gap | Why it matters |
|---|---|
| No RFC 822 / MIME parser | Gmail returns pre-parsed JSON; IMAP and POP3 return raw bytes |
| No threading | Gmail supplies `threadId`; IMAP and POP3 only give `Message-ID`, `In-Reply-To`, `References` |
| No server settings on an account | `Account` holds only id, provider, name, email |
| No password storage | Only OAuth tokens are modelled |
| No SMTP transport | Gmail sends through its REST API |
| Messages are modelled as a replaceable cache | For POP3 the local copy is the only copy |
| Status wording and settings copy assume Gmail or Sample | `RATE_LIMIT_WORDING`, "Connected through the Gmail…", `is_gmail` branches |

## Design decisions

### Dependencies

| Need | Choice | Notes |
|---|---|---|
| MIME parsing | `mail-parser` | Zero-copy, handles RFC 2047/2231, nested multiparts, broken input. Used by Stalwart. |
| IMAP client | `imap` 3.0.0-alpha with `rustls-tls` (blocking) | Matches the existing blocking-I/O-inside-a-future model (`pollster_block` never sees `Pending`). Milestone 0 confirms it compiles with the pinned toolchain and handles the servers in the test matrix; the fallback is `imap-next` (sans-io) driven over a blocking socket. Do not pull in `async-imap`: it would force a real async runtime into the sync path. |
| POP3 client | Hand-written, in `overlook-provider` | RFC 1939 plus `UIDL`, `TOP`, `STLS` is ~300 lines. The crates available are unmaintained. |
| SMTP | `lettre` with `smtp-transport`, `rustls-tls`, `rustls-native-certs` | Already a dependency. Disable `native-tls` and `pool`. |
| TLS | `rustls` with system roots | Same as `ureq`. No OpenSSL. |
| Autoconfiguration | `ureq` GET to Thunderbird's ISPDB | `https://autoconfig.thunderbird.net/v1.1/<domain>`; then RFC 6186 SRV records (optional, later); then a short built-in preset table. |

### Account server settings

Server settings are not secrets and belong in the store, not the keyring. Keep `Account` unchanged (it is constructed in dozens of tests) and add a sibling type:

```rust
// overlook-core
pub enum Security { Tls, StartTls, None }           // None only on localhost, for tests
pub enum AuthMethod { Password, OAuth2 }             // OAuth2 reserved, not implemented in this plan
pub struct ServerEndpoint { host, port: u16, security, username, auth: AuthMethod }
pub struct MailServers {
    incoming: ServerEndpoint,                        // IMAP or POP3 depending on account.provider
    outgoing: ServerEndpoint,                        // SMTP
    pop3: Option<Pop3Policy>,                        // leave on server, delete after N days, delete when trashed
    imap: Option<ImapPolicy>,                        // archive folder, sent folder override, trash folder override, expunge on trash
}
```

Store: schema V11 adds `account_servers(account_id PRIMARY KEY, servers_json)` with `Store::servers(account)` and `Store::set_servers(account, &MailServers)`. Passwords go to the keyring as `kind="password:incoming"` and `kind="password:outgoing"` (the outgoing one is omitted when "same as incoming" is ticked).

Account id format: `imap:<email>` and `pop3:<email>`, mirroring `gmail:<email>`.

### Credentials

Generalise `credentials.rs` to `save_secret(account, kind, bytes)`, `load_secret`, `delete_secret`, keeping the typed `save_tokens`/`load_tokens` wrappers. `remove_account` deletes every kind for the account. A failed IMAP/SMTP `LOGIN`/`AUTH` maps to `ProviderError::SignInRequired`; the UI's "Sign in again" button becomes "Update password" for these accounts and opens the server-settings dialog with the password field focused.

### Folders as labels

Overlook's sidebar, views, counts and operations are label-based. IMAP folders map onto `Label` rows so no UI changes are needed for navigation:

| IMAP folder | `LabelKind` | How identified |
|---|---|---|
| `INBOX` | `Inbox` | always |
| `\Sent` or names Sent, Sent Items, Sent Messages | `Sent` | SPECIAL-USE first (RFC 6154), then name heuristics, then user override |
| `\Trash` or Trash, Deleted Items, Deleted Messages | `Trash` | same |
| `\Junk` or Junk, Spam | `Spam` | same |
| `\Archive` or Archive, All Mail (Gmail over IMAP) | `Archive` | same; Archive is also where the Archive operation moves mail. Created on demand when no candidate exists. |
| `\Drafts` | not mapped | `LabelKind::Drafts` is a virtual local view. Server drafts are a follow-up. |
| everything else | `User` | `name` is the path with the delimiter replaced by " / "; `provider_id` is the raw path |

Folder sync state lives in a new table `imap_folders(account_id, path, uidvalidity, highest_uid, highest_modseq, delimiter, special_use, subscribed, PRIMARY KEY(account_id, path))`. The `Label.provider_id` is the folder path; the folder row carries the sync cursors.

For IMAP a message lives in exactly one folder, so `label_ids` holds one folder label. Flags map directly: `\Seen` ↔ `is_read`, `\Flagged` ↔ `is_starred`. Gmail-style multi-label is not simulated.

### Message identity and moves

Gmail local ids are `<account>:<gmail id>`. IMAP UIDs are per folder and change on move, so a message's local id must not be derived from its UID. Decision:

- Local `MessageId` is `<account>:<uuid>` minted on first sight.
- `provider_id` is `<folder path>\u{1f}<uidvalidity>\u{1f}<uid>` and is rewritten when the message moves. The existing unique index on `(account_id, provider_id)` keeps this honest.
- Moves initiated locally (Archive, Trash, Untrash, AddLabel as "move to folder") use `UID MOVE` when the server advertises `MOVE`, otherwise `COPY` + `\Deleted` + `UID EXPUNGE`. The `COPYUID` response gives the new UID so `provider_id` is updated at once. This needs `apply` to be able to report provider-id changes: extend its return type to `Result<AppliedChanges, ProviderError>` where `AppliedChanges { provider_ids: Vec<(MessageId, ProviderId, LabelId)> }` and `Default` is "nothing changed". Gmail and Sample return the default.
- Moves made elsewhere (another client) show up as a UID vanishing from folder A and a new UID in folder B. `fetch_changes` carries `rfc_message_id` on new headers; the sync engine asks the store for an existing message with the same `rfc_message_id` whose provider folder no longer contains it, and reuses that local id instead of inserting a duplicate. New store method: `message_by_rfc_id(account, rfc_id)`. Add an index on `messages(account_id, rfc_message_id)`.

### Threading

Add `references_json` to `messages` (V11) and a `references: Vec<String>` field on `Message` (serde default). Add `overlook_core::threading::assign_conversations(incoming: &mut [Message], lookup: impl Fn(&str) -> Option<ConversationId>)`: for each message, the first of `In-Reply-To` and `References` (newest first) that resolves to a known conversation wins; otherwise the message starts a conversation whose id is `<account>:<its own Message-ID or uuid>`. The sync engine runs this after `fetch_snapshot`/`fetch_changes`/`fetch_messages` for providers that return `conversation_id` empty. Subject-based fallback is deliberately excluded (it merges unrelated mail); revisit if real mailboxes show too many broken threads.

### IMAP synchronisation

`fetch_snapshot` (first sync and after `CheckpointExpired`):

1. `CAPABILITY`, `LIST "" "*"` (with `SPECIAL-USE` when supported), `LSUB` to flag subscriptions. Build labels; write `imap_folders` cursors.
2. For each folder, `SELECT` (or `EXAMINE`), record `UIDVALIDITY` and `UIDNEXT`, then `UID SEARCH ALL` (or `UID FETCH 1:* (UID)` on servers that mishandle SEARCH). Keep the newest *N* UIDs per folder where *N* is the existing "Inbox messages to download" preference for Inbox and the same scaled split the Gmail provider uses for Sent/Archive/Trash/Spam/other.
3. Return `message_ids` as provider ids so the existing persisted backlog downloads bodies in batches of 20. `checkpoint` is the JSON of all folder cursors.

`fetch_messages(ids)` groups ids by folder, selects each once and issues `UID FETCH <set> (UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])`. Messages over 2 MB fetch `BODYSTRUCTURE` and only the text parts; attachments stay as references with their section number in `Attachment.provider_id` and are fetched on click, exactly like Gmail. Below that size the whole message is parsed and attachment bytes are kept in memory only long enough to compute size and `Content-ID`; attachment contents are still fetched on demand so the store stays a header+body cache.

`fetch_changes(checkpoint)` per folder:

- `SELECT`; if `UIDVALIDITY` differs from the cursor → treat the folder as new: report all its known local messages as deleted and queue the newest *N* again.
- New mail: `UID FETCH <highest_uid+1>:* (UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE)` → header-only upserts; bodies are queued to the backlog via `message_ids` on `RemoteChanges` (add that field; the engine already has `enqueue`).
- Flag changes: with `CONDSTORE`, `UID FETCH 1:<highest_uid> (FLAGS) (CHANGEDSINCE <modseq>)`; without it, `UID FETCH 1:<highest_uid> (FLAGS)` for Inbox on every cycle and for other folders every tenth cycle (cheap: a few bytes per message).
- Deletions: with `QRESYNC`, the `VANISHED` response; without it, `UID SEARCH ALL` on the folder and diff against the local UID set. The store needs `provider_ids_for_label(label)` for the diff.
- Folder list re-checked once per hour or when a `LIST` capability change is seen. New folders → labels; removed folders → delete their messages.

Scheduling: the existing poll interval (30 s to 15 min) drives IMAP in milestone 2. `IDLE` on Inbox is milestone 5: a dedicated thread that holds one extra connection and, on an untagged `EXISTS`/`EXPUNGE`/`FETCH`, calls the existing `sync_now` path. One connection per account for sync, one for IDLE; attachment and raw fetches reuse the sync connection under a mutex.

Server-side search: `search` can be implemented later with `UID SEARCH TEXT` across folders; `SearchCoverage` already reports "local only" truthfully.

### IMAP operations

| `OperationKind` | IMAP |
|---|---|
| `SetRead` | `UID STORE ±FLAGS.SILENT (\Seen)` grouped by folder |
| `SetStarred` | `UID STORE ±FLAGS.SILENT (\Flagged)` |
| `Archive` | move to the Archive folder (created if the user confirmed it in setup) |
| `Trash` | move to Trash; with `ImapPolicy.expunge_on_trash` and no Trash folder, `\Deleted` + `UID EXPUNGE` |
| `Untrash` | move to Inbox |
| `AddLabel { label }` | move to that folder (a user label is a folder) |
| `RemoveLabel` | move to Inbox (removing the only folder a message has means "put it back") |
| `SendDraft` | SMTP (below) |
| calendar/contact ops | `Rejected` with the existing wording; IMAP has no calendar or contacts |

A `NO` response that means the folder vanished is `Rejected`; `BYE`/connection reset is `Offline`; `[UNAVAILABLE]`/`[INUSE]` is `Temporary`; `[AUTHENTICATIONFAILED]` is `SignInRequired`.

### Sending over SMTP and the Sent copy

Move `build_raw` out of `gmail/mime.rs` into a shared `overlook_provider::outgoing` module (Gmail keeps calling it). New `overlook_provider::smtp::Transport` wraps `lettre::SmtpTransport` built from `ServerEndpoint` and a password loaded at provider construction:

1. `send` builds the raw message with the deterministic `draft_message_id`, submits over SMTP. `lettre` connection errors before `DATA` is accepted → `Offline` or `Temporary`; a 5xx reply → `Rejected(reply text)`; a dropped connection after `DATA` was sent → `Uncertain`.
2. IMAP accounts then `APPEND` the same bytes to the Sent folder with `\Seen`; `APPENDUID` gives the provider id for the `SentReceipt`. If `APPEND` fails after SMTP succeeded, the receipt still says Sent (the message left) and the Sent copy is built locally with `sample::sent_message` as the Gmail path already does; a later sync may then show the server copy as a second message, de-duplicated by `rfc_message_id`.
3. POP3 accounts have no server Sent folder: the receipt's message is the locally built copy with the local Sent label.

`reconcile_send` for IMAP: `UID SEARCH HEADER Message-ID <id>` in the Sent folder; found → `Delivered`. Not found is **not** proof of non-delivery (SMTP accepted mail is not copied to Sent by most servers), so it returns `NotDelivered` only when the Uncertain error happened before `DATA`; otherwise the operation stays Uncertain and the Outbox shows the existing "Send again / Discard" affordance. For POP3 there is nothing to search; the draft stays Uncertain for the user to decide. This is a deliberate choice against the Gmail path, where the API can prove delivery.

### POP3 model

POP3 is download-and-keep-locally. Decisions:

- Labels are local only: Inbox, Sent, Trash, Archive are created by the provider at account creation with `provider_id = None`. User labels are allowed and purely local. All mailbox operations are applied by the store transaction already; `apply` returns `Ok` immediately, except `Trash` with `Pop3Policy.delete_on_server_when_trashed`, which issues `DELE` by UIDL.
- Identity: `provider_id` is the `UIDL`. Local id `<account>:<uuid>`. A table `pop3_seen(account_id, uidl, seen_at, deleted_on_server, PRIMARY KEY(account_id, uidl))` records every UIDL ever downloaded so re-appearing or server-side-deleted messages never duplicate or vanish locally.
- `fetch_snapshot`/`fetch_changes` are the same routine: `UIDL`, diff against `pop3_seen`, return the unseen UIDLs newest-first as `message_ids`; the backlog `RETR`s them. Messages above a size cap use `TOP n 0` for headers first and are queued for full retrieval. `RemoteChanges.deleted` is always empty: mail removed from the server stays local.
- Retention: `Pop3Policy { leave_on_server: bool (default true), delete_after_days: Option<u32>, delete_on_server_when_trashed: bool }`. Deletions run at the end of a cycle with `DELE` + `QUIT` (POP3 only commits on `QUIT`).
- Raw bytes are the primary record: store them under `<data dir>/raw/<account>/<message id>.eml` so `fetch_raw` and attachment extraction never need the server again. `fetch_attachment` re-parses the saved `.eml`.
- Eviction: no eviction routine exists today, which is why nothing is lost; the plan adds a `Store::is_local_primary(account)` check (true for POP3) that any future eviction must honour, and documents it in the store crate's module comment.
- Backup: for POP3 the local store is the user's only copy. The About page gains "Back up mail…" that copies the SQLite file (via `VACUUM INTO`) plus `raw/` and `attachments/` to a chosen folder. Restore is "stop Overlook, replace the data directory" in milestone 4 and a UI in a later release.

### Shared MIME mapping

New module `overlook_provider::mime::parse` (the existing `gmail/mime.rs` becomes `outgoing`): `fn message_from_rfc822(account, bytes, local_id, flags) -> Message` using `mail-parser`. It reuses `html_to_text` for the snippet, keeps `Content-ID` for inline images (the `cid:` path already exists), recognises `text/calendar` parts and runs `ics::parse_invitation`, records `References`, and produces `Attachment` rows with the MIME part path as `provider_id`. Tests: port the fixture cases from `gmail/mapping.rs` (encoded words, inline images, minimal message) to raw-message fixtures, add a malformed-message set from `crates/overlook-core/tests`.

### Add-account flow

Settings → Accounts gains an "Add account" menu: Gmail (existing sign-in), IMAP, POP3. The IMAP/POP3 path opens a new `AccountSetup` dialog in `crates/overlook-app/src/ui/account_setup.rs`:

1. Step 1: name, email, password. On "Continue" a background task fetches ISPDB autoconfig for the domain and fills incoming/outgoing servers; no result → the preset table (Fastmail, iCloud, Yahoo, Zoho, Proton Bridge, Posteo, mailbox.org, GMX, common cPanel patterns `mail.<domain>`); still nothing → step 2 with empty fields.
2. Step 2 (Advanced, always reachable): incoming host/port/security/username, outgoing host/port/security/username, "use the same password", POP3 retention policy or IMAP folder overrides. "Test connection" runs `probe` for both endpoints off the UI thread and shows the server's own error text.
3. Save: `upsert_account`, `set_servers`, keyring writes, `providers_for`, `set_sync_state(Syncing)`, switch to the account, `sync_account`. Same tail as `finish_google_sign_in`.

The account page's "Server settings…" button reopens the dialog in edit mode. Remove account deletes both password entries. Copy that currently reads "Connected through the Gmail and Google Calendar APIs." gains IMAP and POP3 variants; `RATE_LIMIT_WORDING` drops "Gmail".

### Capabilities surface

The UI already asks `has_settings_access`, `has_contacts_access`; both stay false for IMAP/POP3 (correct: no automatic replies, no address book). Calendar: `AccountProviders.calendar` is `None`, and the calendar module already handles accounts without a calendar provider through `has_calendar_access`. Confirm the Calendar sidebar hides the account rather than showing "Sign in again".

## Milestones

Each milestone ends with `cargo test --workspace` green and a note in the status log. Milestones 1 to 3 together are the product plan's phase 2; milestone 4 is phase 3.

### M0: Spike (done 14 October 2026)

Goal: confirm that `imap`, `mail-parser`, `lettre` (SMTP) and `rustls` compile on the pinned toolchain and carry one message end to end. Code: `crates/overlook-provider/examples/imap_spike.rs` (throwaway, configured by `SPIKE_*` environment variables) and `scripts/spike-mail-servers.py`, a scripted plaintext IMAP + SMTP pair shaped like Dovecot responses, used because this machine has neither Dovecot nor a usable Docker socket.

Result: the pipeline works. Against the scripted server the example logged in, listed six folders with SPECIAL-USE attributes, selected INBOX, ran `UID SEARCH ALL`, fetched envelopes then full bodies, parsed all three messages with `mail-parser` (RFC 2047 encoded words, nested `multipart/mixed` + `alternative`, an inline `cid:` image, a PDF attachment, a `text/calendar` part, `In-Reply-To`/`References`), sent one message over SMTP with AUTH PLAIN, appended the same bytes to `Sent` with `\Seen` and got `APPENDUID` back, and found it again with `UID SEARCH HEADER Message-ID`. Workspace build and all tests stay green.

Decisions and findings that change the plan above:

- **`imap` 3.0.0-alpha.15, blocking, without its TLS features.** Its `rustls-tls` feature pins `rustls-connector` 0.19, which drags in rustls 0.22 next to the rustls 0.23 `ureq` already uses. We build the TLS stream ourselves (`rustls` 0.23 with `ring`, `webpki-roots`) and hand it to `imap::Client::new`, which is generic over `Read + Write`. The tree has exactly one rustls and one crypto provider.
- **STARTTLS is negotiated by hand** on the raw socket (greeting, `STARTTLS`, OK) before the client is constructed, because the crate exposes pre-login commands only through its own `ClientBuilder`. The upgraded client needs `greeting_read = true`. Eight lines; no reason to fork the crate.
- `imap-proto` 0.16 is a direct dependency for `NameAttribute` (SPECIAL-USE variants: `\Sent`, `\Trash`, `\Junk`, `\Archive`, `\Drafts`, `\All`), which the `imap` crate does not re-export. The enum is `#[non_exhaustive]`.
- `uid_search` returns a `HashSet`; order it locally. `ENVELOPE` subjects arrive still RFC 2047 encoded, so header-only rows must run the existing `decode_rfc2047` from `gmail/mapping.rs` (move it to the shared MIME module in M1).
- `mail-parser` reports the `text/calendar` part through `attachments()` as well; the mapping should recognise it as an invitation first, as the Gmail mapping does.
- `lettre` features: `builder`, `smtp-transport`, `rustls-tls` (ring + webpki-roots), no `native-tls`, no `pool`. `Tls::Wrapper` for implicit TLS (465), `Tls::Required` for STARTTLS (587).

Not yet validated, carried into M2: TLS and STARTTLS against real servers (the scripted server is plaintext only), real-server quirks (Dovecot, Fastmail, iCloud, Gmail over IMAP), `IDLE`, large-message `BODYSTRUCTURE` partial fetches, and the SMTP "connection dropped after DATA" → Uncertain mapping, which needs a fault-injecting fake.

### M1: Shared plumbing (about a week)

- `overlook-core`: `MailServers` types, `Message.references`, `threading::assign_conversations` with tests.
- `overlook-store`: V11 migration (`account_servers`, `imap_folders`, `pop3_seen`, `messages.references_json`, index on `rfc_message_id`), `servers`/`set_servers`, `message_by_rfc_id`, `provider_ids_for_label`, `is_local_primary`. Tests for each.
- `overlook-provider`: `mime::parse` with fixtures; move `build_raw` to `outgoing`; `smtp::Transport` with a scripted fake SMTP server test (greeting, EHLO, AUTH, DATA, dropped connection → Uncertain).
- `overlook-app`: `credentials` generalisation; `apply` return type widened to `AppliedChanges`; sync engine runs threading for providers that leave `conversation_id` empty and reconciles moves by `rfc_message_id`; `RemoteChanges.message_ids` enqueued.

### M2: IMAP read path and account setup (about two weeks)

- `overlook_provider::imap::ImapProvider`: connection management, `probe`, folder discovery → labels, `fetch_snapshot`, `fetch_messages`, `fetch_changes` (UIDVALIDITY, new mail, flags, deletions, without CONDSTORE/QRESYNC first), `fetch_attachment`, `fetch_raw`.
- Scripted fake IMAP server in the crate's tests covering every response shape the provider parses, plus an `#[ignore]` integration test against Dovecot (`OVERLOOK_IMAP_TEST_URL`, user, password from the environment) that CI can run once a Dovecot step is added to `ci.yml`.
- `providers_for` builds IMAP providers; `AccountSetup` dialog with autoconfig, presets and Test connection; settings copy; "Update password" flow on `SignInRequired`.
- Exit criterion: a real IMAP account (Fastmail or Dovecot) reads, searches locally, shows attachments and threads correctly across restarts and an offline period.

### M3: IMAP write path and sending (about one week)

- `apply` for all mailbox operations with `MOVE`/`COPY` fallback and `COPYUID` provider-id updates; expunge policy.
- `send` over SMTP with `APPEND` to Sent; `reconcile_send`; Uncertain wording in the Outbox checked for the new cases.
- Pending-operation replay tests with the fake server: offline queue → reconnect → correct order, idempotent re-apply after a crash between server success and local commit.
- Exit criterion: the product plan's offline scenarios pass for IMAP (disconnect during download, during a move, during a send; restart while offline; the same folder changed from another client).

### M4: POP3 (about one week)

- `overlook_provider::pop3::Pop3Provider` with the hand-written client (`STLS`, `USER`/`PASS`, `UIDL`, `LIST`, `TOP`, `RETR`, `DELE`, `QUIT`), local labels, `pop3_seen`, retention policy, `.eml` primary storage, attachments from the saved file.
- Setup dialog retention controls; POP3 copy in settings; "Back up mail…" on the About page.
- Fake POP3 server tests; Dovecot also serves POP3 for the integration test.
- Exit criterion: a POP3 account downloads once, never duplicates on repeated polls, honours leave-on-server and delete-after-N-days, sends through SMTP, and a backup restores on a clean machine.

### M5: Hardening and reach (ongoing)

- `IDLE` on Inbox; `CONDSTORE`/`QRESYNC` fast paths; folder list refresh; server-side `search`.
- Interoperability matrix recorded here: Dovecot, Fastmail, iCloud, Yahoo, Zoho, Proton Bridge, Gmail over IMAP (`[Gmail]` folder names, `\All`), Microsoft 365 (basic auth is disabled; needs XOAUTH2, see below).
- Update `overlook-native-app.md` (install, settings, known gaps), `native-linux-mail-client-plan.md` phase table, and the README feature list.

## Out of scope for this plan, tracked as follow-ups

- **XOAUTH2 for IMAP/SMTP** (Microsoft 365, Outlook.com, Gmail over IMAP). `AuthMethod::OAuth2` is reserved in the model. Microsoft needs an Azure app registration like the Google one; the sign-in flow can reuse `google_signin.rs` with a different authority.
- **Server-side Drafts** (IMAP `\Drafts` sync). Drafts stay local, as they are for Gmail today.
- **Folder management** (create, rename, delete, subscribe) and moving messages between arbitrary folders from the UI beyond the existing label menu.
- **Local folders for Gmail accounts** and **import of `.mbox`/`.eml`**.
- **Restore UI** for POP3 backups.
- **Exchange/EWS/Graph**: a different provider, not an IMAP variant.

## Risks

| Risk | Mitigation |
|---|---|
| `imap` 3.0 alpha API churn or a parsing gap on a real server | M0 spike against two servers; `imap-next` fallback; every parsed response has a fake-server test |
| Duplicate messages after moves made by other clients | `rfc_message_id` reconciliation in M1 plus the `COPYUID` fast path for our own moves; a store test with a vanished-UID / new-UID pair |
| Blocking I/O stalls the sync thread on a hung server | Socket read/write timeouts (30 s) on every connection; the sync thread is already off the UI thread |
| POP3 data loss | `.eml` primary storage, `pop3_seen` ledger, no eviction for local-primary accounts, backup before milestone 4 ships |
| Password handling | Only in the keyring; never in logs (`tracing` fields must not include `ServerEndpoint` with the password, which is why the password is not a field of it) |
| Scope creep into Exchange | Named out of scope above |

## Status log

- 2026-10-14: Plan written. No code changed.
- 2026-10-14 (later): M0 spike done. Dependencies added to `overlook-provider` (`mail-parser`, `imap` without TLS features, `imap-proto`, `rustls`, `webpki-roots`, `lettre` SMTP features); `examples/imap_spike.rs` and `scripts/spike-mail-servers.py` added; full IMAP read + SMTP send + APPEND + reconcile path exercised against the scripted server. Findings recorded in M0.

- 2026-10-14 (later still): M1 shared plumbing done; `cargo test --workspace` green, no behaviour change for Gmail or Sample accounts. Landed:
  - `overlook-core`: `servers.rs` (`MailServers`, `ServerEndpoint`, `Security`, `AuthMethod`, `Pop3Policy`, `ImapPolicy`, plus `ServerProtocol::default_port` and `ServerEndpoint::is_insecure_remote`), `Message.references` (serde default), `threading::assign_conversations` with `normalize_message_id` / `parent_candidates`. Threading processes a batch oldest first and also resolves a message's *own* Message-ID through the lookup, so a parent downloaded after its reply joins the reply's conversation instead of starting a second one.
  - `overlook-store`: V11 (`account_servers`, `imap_folders`, `pop3_seen`, `messages.references_json`, `messages_rfc_id` index); `servers`/`set_servers`, `is_local_primary` (documented in the crate comment as the eviction guard), `message_by_rfc_id` and `messages_by_rfc_id` (several copies are legitimate), `message_id_for_provider_id`, `conversation_for_rfc_id` (the threading lookup: matches a stored message's own id, its `In-Reply-To`, or its `References`), `provider_ids_for_label`, `apply_provider_id_changes`. `merge_remote_messages` keeps the stored conversation when an incoming row has none. Accessors for `imap_folders` and `pop3_seen` rows are deferred to M2 and M4, where their shape is decided.
  - `overlook-provider`: `gmail/mime.rs` became `outgoing.rs` (`build_raw`, `message_to_raw`, both re-exported at the crate root); `mime::decode_rfc2047` moved out of `gmail/mapping.rs`; `mime::parse::message_from_rfc822` (plus `headers_from_rfc822` for header-only fetches) over `mail-parser`, with Message-IDs normalised to the bracketed form Gmail headers already use, attachments carrying their IMAP section path as `provider_id`, forwarded `message/rfc822` parts as `.eml` attachments, and an unreadable-message fallback; fixtures ported from the Gmail mapping plus a malformed set (empty, garbage, truncated multipart, bad `Date`, bad UTF-8, the hostile HTML fixture from `overlook-core/tests`). `smtp::Transport` drives lettre's `SmtpConnection` command by command so the error mapping is exact: connect/TLS/EOF before `DATA` → `Offline`, 4xx → `Temporary`, 5xx → `Rejected` (server text), failed `AUTH` → `SignInRequired`, dropped after the body was written → `Uncertain`; it refuses plaintext to non-local hosts. Tested against an in-crate scripted fake SMTP server covering each case. `MailProvider::apply` returns `AppliedChanges`; `RemoteChanges.message_ids` added. The Gmail mapping now records `References`.
  - `overlook-app`: `credentials` generalised to `save_secret`/`load_secret`/`delete_secret`/`delete_all_secrets` by kind (`oauth-tokens`, `password:incoming`, `password:outgoing`) with typed wrappers; `remove_account` deletes every kind for any non-Sample account. The sync engine gained `reconcile_incoming`, run on snapshot, change and body-download batches: adopts the stored local id for a provider id already on disk (so a body download completes the header-only row the provider minted a different uuid for), treats an upsert whose Message-ID matches a message in the same cycle's `deleted` list as a move (keeps the local id and conversation, drops it from `deleted`), and threads whatever still has no conversation. `RemoteChanges.message_ids` are queued through `enqueue_bodies` (a row without a cached body is still queued; the snapshot path's `enqueue` keeps its old semantics). `enqueue`'s presence check now goes by provider id instead of the Gmail `<account>:<id>` convention, which is identical for Gmail. `AppliedChanges.provider_ids` are written through `apply_provider_id_changes` after a successful `apply`. Tests use an in-crate scripted IMAP-shaped provider.
  - Not done / carried forward: TLS and STARTTLS for SMTP are still unverified against real servers (the fake is plaintext); the move reconciliation only pairs a vanish and an appearance seen in the same cycle (split across cycles it degrades to delete + insert, never to a duplicate); `AuthMethod::OAuth2` is modelled but no transport uses it; `Folder`/`FolderId` are untouched; no UI reads `MailServers` yet.

- 2026-10-15: M2 IMAP read path and account setup done; `cargo test --workspace` green (provider 55 tests, app 46 + 18 ignored, store 19). Verified end to end against `scripts/spike-mail-servers.py` through the two ignored tests (`real_imap_server_round_trip` in the provider, `imap_account_syncs_through_the_store_across_a_restart` in the app: snapshot, bodies, threading, an attachment fetch, reopen the database, incremental cycle), and the TLS path against Fastmail and Gmail plus hand-negotiated STARTTLS against Office 365 (handshake with the webpki roots, `NO [AUTHENTICATIONFAILED]` → `SignInRequired`, each with a bogus login). Landed:
  - `overlook_provider::imap::ImapProvider` (`imap/mod.rs`, `connection.rs`, `folders.rs`, `structure.rs`, `uidset.rs`): one connection per account under a mutex, reopened once when a reused connection turns out dead; TLS / hand-negotiated STARTTLS / plaintext only to localhost as in the spike; error mapping per the plan (`BYE`, lost connection, timeouts → `Offline`; `[AUTHENTICATIONFAILED]` or any `NO` to `LOGIN` → `SignInRequired`; `[UNAVAILABLE]`/`[INUSE]`/`[SERVERBUG]`/`[LIMIT]` → `Temporary`; other `NO`/`BAD` and a failed TLS handshake → `Rejected` with the server's words). Folders → labels per the table with policy overrides, SPECIAL-USE, then names near the top of the hierarchy; `\Drafts`, `\Flagged`, `\Important` and unselectable containers are not labels; `INBOX<delim>` is hidden in display names. `fetch_snapshot` lists, `EXAMINE`s each folder, `UID SEARCH ALL`, keeps the newest N (Inbox = the download preference; Sent/Archive 40, Trash/user 20, Spam 10, scaled like Gmail) and records their flags. `fetch_messages` groups ids by folder and UIDVALIDITY, checks sizes first, fetches whole bodies under 2 MB and goes through `BODYSTRUCTURE` + per-section text fetches (`<0.1MB>` partial) above it, assembling a synthetic message for the shared parser with attachments listed from the structure. `fetch_changes` per folder: `UIDVALIDITY` change → every known UID reported deleted and the newest N queued again; new mail by UID above the cursor as header-only rows (`BODY.PEEK[HEADER]`, so `References` are available for threading) with bodies queued through `message_ids`; flag refresh (`UID FETCH (FLAGS)` over the known set) every cycle for the Inbox and every tenth cycle elsewhere, re-fetching headers only for UIDs whose `\Seen`/`\Flagged` changed; deletions by `UID SEARCH ALL` diffed against the known set; folder list reread hourly or at once when an `EXAMINE` says the folder is gone. `fetch_attachment` fetches `BODY.PEEK[<section>]` plus its `.MIME` headers and decodes through `mail-parser`; `fetch_raw` is `BODY.PEEK[]`. `apply`, `send` and `reconcile_send` are `Rejected` placeholders (M3).
  - Checkpoint design: the checkpoint is the JSON of all folder cursors (`Checkpoint` / `FolderCursor`: label kind, UIDVALIDITY, highest UID, and the known / `\Seen` / `\Flagged` UID sets as sequence-set strings). Because the provider never touches the store, it cannot know local ids or which UIDs the store holds, so deletions are reported through a new `RemoteChanges.deleted_provider_ids` that the sync engine resolves with `message_id_for_provider_id` before move reconciliation. The engine mirrors the cursors into `imap_folders` after each successful cycle (`Store::imap_folders` / `replace_imap_folders`, `overlook_core::ImapFolder`, `imap::folder_states`); the checkpoint stays authoritative.
  - `overlook_provider::autoconfig`: Thunderbird ISPDB (`ureq`, 8 s timeout, tag scanner for the small XML), a preset table (Fastmail, iCloud, Yahoo, AOL, Zoho, Proton Bridge, Posteo, mailbox.org, GMX, WEB.DE, Gmail over IMAP, HEY, Tuta) checked first so the common cases need no network, then the `mail.<domain>` guess. Plaintext suggestions for remote hosts are refused.
  - Store: `merge_remote_messages` keeps the stored snippet and invitation when an incoming row has no cached body (a flag refresh must not blank what the body download filled in).
  - App: `providers_for` takes the store and builds IMAP providers from `Store::servers` and the incoming password (`PASSWORD_NEEDED_WORDING` when the keyring has none); `ui/account_setup.rs` is the dialog (step 1 name / address / password → background autoconfig → step 2 servers with security pickers that follow the conventional port, "use the same password for sending", Test connection probing IMAP and SMTP off the UI thread with the server's own words, Save mirroring `finish_google_sign_in`; "Set up servers manually" skips the lookup). Settings → Accounts has an "Add account" menu (Gmail / IMAP / POP3 coming soon), per-provider connection copy, "Server settings…" for IMAP accounts, and "Update password" instead of "Sign in again" for password accounts (also in the banner and the sidebar account menu). `RATE_LIMIT_WORDING`, the search coverage line, the automatic-replies and contacts copy lost their Gmail wording; the Calendar module shows an information banner for accounts without a calendar. `OVERLOOK_SIGN_IN=imap` opens the dialog at launch.
  - `scripts/spike-mail-servers.py` now serves `BODY[HEADER]`, `BODY[TEXT]`, `BODY[n.m]`, `BODY[n.m.MIME]` and partial fetches, so the provider's attachment path can be exercised against it.
  - Deferred to M3: every mailbox write (flags, moves, trash, expunge policy), SMTP send with the Sent `APPEND`, send reconciliation. Until then an IMAP account's local read/star/archive actions are queued and the replay reports `Rejected` ("Changes to IMAP mailboxes are not sent to the server yet"), which the status row shows as a sync failure; M3 should land before this ships. Deferred to M5: `CONDSTORE`/`QRESYNC`, `IDLE`, server-side search, modified UTF-7 decoding of non-ASCII folder names (they display raw), the interoperability matrix. Not verified: the dialog was launched (no panic) but not clicked through on screen; no real TLS or STARTTLS server was synced end to end (no credentials available to this session), only probed through the handshake to the login.
