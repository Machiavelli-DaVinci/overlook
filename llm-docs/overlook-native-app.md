# Overlook Native App: Build, Run, Test

Status as of 12 October 2026: foundation milestones 2 to 5 implemented (see the [foundation plan](foundation-implementation-plan.md)), plus the three daily-use features the user asked for: Focused Inbox, attachments, and Google Calendar. This is a native Rust + GPUI application with a durable local store, a sample provider, and working Gmail accounts (sign-in, download, incremental sync, mailbox actions, send).

## Build and run

Requirements on Arch/Omarchy: `rustup` with Rust 1.99 (pinned in `rust-toolchain.toml`), Vulkan drivers, Wayland. No GTK or WebKit packages are needed.

```bash
cargo build
```

```bash
cargo run -p overlook
```

Data lives in `~/.local/share/overlook/` (`overlook.sqlite` plus `outgoing/` for attachments on queued messages). Override it for experiments:

```bash
OVERLOOK_DATA_DIR=/tmp/overlook-demo cargo run -p overlook
```

On first launch Overlook seeds a **Sample account** (“Jason Alexander <jason@example.com>”) with the fictional mailbox from the design preview. The account is labelled as a sample everywhere; its connection is simulated and starts **disconnected**.

## Focused Inbox

A local view, not a Gmail feature. `overlook_core::focus::classify_focus` puts a conversation in Focused when the user said so (per-message override or per-sender rule), when the sender is someone the user has written to, or when Gmail marks the mail personal or important; promotions, social, forums, and updates go to Other, as do obvious automated senders when Gmail gives no hint. The Inbox shows Focused/Other tabs with a filter menu; the reader toolbar offers Move to Other/Focused with an "always from this sender" rule. Toggle in the layout menu. Schema v2 stores category, importance, and overrides.

## Attachments

Cards in the reader download on click (Gmail attachments endpoint), open with the system handler, and offer Save as… through a file dialog. Files live under `<data dir>/attachments/<account>/<message>/`. Outgoing attachments are copied into `<data dir>/outgoing/` and sent as MIME parts.

## Calendar

Google Calendar on the same OAuth client, with the `calendar.events` and `calendar.readonly` scopes (added 12 Oct 2026; existing accounts must sign in again, and the calendar banner says so while mail keeps working). Calendars and events live in schema v3/v4 tables; the first download covers 60 days back and a year ahead per visible calendar with `singleEvents=true`, then incremental changes through Google sync tokens (a 410 restarts the window). Week, Day, and Agenda views; event details with Join, Open in Google Calendar, guests and their responses; Accept/Tentative/Decline for invitations (PATCH of the attendee list, `sendUpdates=all`); New event with a client-generated base32hex id so retries never duplicate; delete for events you own. Working-location entries show in the day header, not as events. All calendar writes go through the pending-operation queue and the shared sync engine; calendar failures never block mail.

## Gmail

1. Create a Google Cloud OAuth client of type **Desktop app** with the Gmail API enabled and the scopes `gmail.modify`, `gmail.send`, `openid`, `userinfo.email` on the consent screen (Testing mode with test users is fine for development; restricted-scope verification is needed for public release).
2. Save the downloaded JSON as `~/.config/overlook/google-oauth-client.json` (owner-only permissions). It is never embedded in the binary or committed.
3. In Overlook open the account menu (chevron button at the bottom of the sidebar) → **Add Gmail account…**, or launch with `OVERLOOK_SIGN_IN=gmail`. The system browser opens Google's consent screen; Overlook listens on a loopback port with PKCE, exchanges the code, reads the user's email, and stores tokens in the system keyring (Secret Service via `oo7`). Tokens refresh automatically; a revoked grant turns into the **Sign-in required** state with a "Sign in again" action, and local work is kept.
4. The first sync lists the 150 most recent messages (ids only, cheap), then downloads full bodies in batches of 20, committing after every batch. The backlog is persisted (`settings` key `download_backlog:<account>`), so quota pauses, going offline, or a crash never lose progress. Later syncs use `users.history.list` from the saved checkpoint; an expired checkpoint triggers a full resynchronization that preserves pending local intent.
5. Mailbox actions map to `batchModify` (UNREAD/STARRED/INBOX/user labels) and `messages.trash`/`untrash`. Send builds RFC 822 with threading headers (lettre) and posts to `messages.send`; a transport failure after the request is **uncertain** and is reconciled with `rfc822msgid:` before any retry.

Gmail meters usage in quota units per user per minute. Overlook spaces requests about 170 ms apart and, when Gmail still answers with a quota error, enters the **Paused** state and backs off (60 s, growing to 5 minutes) instead of hammering the API. In a fresh project the observed budget allowed roughly 40 full-message downloads per minute, so a first download of 150 messages can take a few minutes. Check **APIs & Services → Gmail API → Quotas** in Google Cloud if downloads pause often.

Environment switches for development:

| Variable | Effect |
|---|---|
| `OVERLOOK_DATA_DIR` | Profile directory |
| `OVERLOOK_CONFIG_DIR` | Directory holding `google-oauth-client.json` (default `~/.config/overlook`) |
| `OVERLOOK_SIGN_IN=gmail` | Start the Google sign-in at launch |

Keyboard additions: `Ctrl 1` Mail, `Ctrl 2` Calendar, `Ctrl Shift A` new event; in the calendar `←`/`→` move by period, `T` today, `D`/`W`/`A` views.
| `OVERLOOK_SAMPLE_ONLINE=1` | Start the sample connection connected, so queued work syncs at launch |
| `OVERLOOK_UTC_OFFSET_MINUTES` | Override the local time zone offset used for day groups |
| `RUST_LOG=overlook=debug` | Tracing output |

## Tests

```bash
cargo test --workspace
```

52 tests cover: appearance resolution (Follow system is not a third theme), recipient parsing, conversation summaries, reply/forward drafts, HTML sanitization and remote-image blocking, local time formatting, PKCE/OAuth request building, the sample provider, store durability across reopen, atomic change+operation (a failing change leaves no operation), Outbox queue/cancel, send completion, FTS search and coverage, settings, and the sync engine (offline leaves work queued; ordered replay; uncertain send reconciled without duplicates; cancelled send never sent).

The manual scenario used to validate the shell (keyboard only, with `wtype` on Hyprland):

1. Launch disconnected. Press `s` (star), `j`, `e` (archive), `Ctrl R`, type a reply, `Ctrl Enter`. Status reads “You’re offline. 5 changes will sync when you’re back online.” Outbox shows 1.
2. `kill -9` the process. Relaunch disconnected: the Outbox item and five pending changes are still there, read from SQLite.
3. Relaunch with `OVERLOOK_SAMPLE_ONLINE=1`: the queue replays in sequence, the reply is accepted by the provider, a Sent copy is stored, the draft row disappears, the toast reads “Sent: …”, and the status bar reads “Up to date · Last synced …”.

Reference captures from this run are in `branding/previews/overlook-native-*.png`.

## Workspace

| Crate | Role |
|---|---|
| `overlook-core` | Domain types (accounts, labels, folders, messages, conversations, drafts, pending operations, sync status), appearance resolution, status wording, HTML sanitizer, time helpers. No UI, no database. |
| `overlook-store` | SQLite schema v1 (`PRAGMA user_version`), WAL + `synchronous=FULL`, FTS5 index with triggers, atomic `apply_change` (local change + pending operation in one transaction), drafts/Outbox, settings, search coverage, known recipients. |
| `overlook-provider` | `MailProvider` trait, `SampleProvider` (in-process server with a connection switch and a “drop the next send” fault), Gmail OAuth/PKCE preparation (no network yet). |
| `overlook` (crates/overlook-app) | GPUI application: theme mapping, fonts, assets, sync engine, and the mail window. |

### Key behaviours

- **Local-first.** Every view reads from the store. `apply_change` writes the change and its operation together; the UI reloads from the store afterwards rather than mutating in memory.
- **Outbox.** Send validates recipients, moves the draft to `Queued`, and records a `SendDraft` operation in the same transaction. Queued and failed items can be edited (which cancels the send) until submission begins. Sent is reported only after the provider returns a receipt; `complete_send` stores the Sent copy, removes the draft, and marks the operation applied atomically.
- **Uncertain delivery.** If the provider reports a dropped connection after accepting a message, the draft and operation become `Uncertain`, replay stops, and the next cycle calls `reconcile_send` before any retry.
- **Sync engine** (`crates/overlook-app/src/sync.rs`) runs on a background thread: probe → `Syncing` → replay pending operations by `seq` → `UpToDate` with a timestamp, or `Offline` / `SignInRequired` / `SyncFailed(reason)`. It runs at startup when reachable, after each local change while connected, every 30 seconds, and from Sync now / F5.
- **Themes.** `theme.rs` holds the two palettes from `branding/tokens.json` and projects them onto the component library. The preference (Light, Dark, Follow system) is stored in `settings`; Follow system observes the window appearance, which GPUI reads from the desktop settings portal. Switching changes colors only.
- **Received HTML** is sanitized by `overlook_core::sanitize` (allow-list of tags, no styles/scripts/forms/frames, `http(s)`/`mailto`/`cid` only, remote images stripped by default with a per-message “Show images”). The result is rendered by the kit’s native HTML text view. `<br>` inside paragraphs becomes a paragraph break because the view only honours block-level breaks.
- **Fonts.** Inter and Source Serif 4 Display are embedded and registered at startup; the wordmark is the outlined SVG.

### Keyboard

Window: `Ctrl N` new message, `Ctrl E` / `Ctrl K` search, `Ctrl R` reply, `Ctrl Shift R` reply all, `Ctrl F` forward, `Ctrl U` toggle read, `Ctrl Enter` send (from any composer field), `Esc` save and close composer / clear search, `F5` sync.
Message list: `J`/`↓` next, `K`/`↑` previous, `E`/`Backspace` archive, `Delete` trash, `S` star, `U` toggle read, `Enter` expand latest message.
Composer header fields: `Enter` moves to the next field.

### Layout and density

The sidebar and list panes are resizable; sizes persist in `settings`. The layout menu in the header switches the reading pane between right and below, and density between comfortable and compact; both persist.

## Known gaps (tracked for the next sessions)

- Gmail: no attachment download for received mail yet, no progressive download beyond the newest 150 messages, no `cid:` inline images, and bodies are fetched one request per message (the Gmail batch endpoint would cut request count).
- Composer is plain text. Rich text, signatures, and HTML output are phase 1 items; `body_html` is already stored separately.
- Attachment download for received mail is not implemented (cards show “Not downloaded”). Outgoing attachments are copied into the protected `outgoing/` store and persisted with the draft.
- Multi-select and drag-and-drop are not implemented.
- Hyprland draws no title bar; the kit’s window controls are not shown there. GNOME and KDE behaviour, fractional scaling, and screen readers (Orca) have not been checked yet.
- Flatpak tooling is absent on this machine; `packaging/` holds the desktop entry, AppStream metadata, and a manifest draft that still need a build.
- Omarchy applies a default window opacity (0.985 active, 0.96 inactive) to every app, including Overlook. That is compositor policy, not an app setting.
