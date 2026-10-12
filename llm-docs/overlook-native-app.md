# Overlook Native App: Build, Run, Test

Status as of 12 October 2026: foundation milestone 2 to 4 implemented (see the [foundation plan](foundation-implementation-plan.md)). This is a native Rust + GPUI application with a durable local store and a sample provider. There is no Gmail connection yet.

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

Environment switches for development:

| Variable | Effect |
|---|---|
| `OVERLOOK_DATA_DIR` | Profile directory |
| `OVERLOOK_SAMPLE_ONLINE=1` | Start the sample connection connected, so queued work syncs at launch |
| `OVERLOOK_UTC_OFFSET_MINUTES` | Override the local time zone offset used for day groups |
| `RUST_LOG=overlook=debug` | Tracing output |

## Tests

```bash
cargo test --workspace
```

32 tests cover: appearance resolution (Follow system is not a third theme), recipient parsing, conversation summaries, reply/forward drafts, HTML sanitization and remote-image blocking, local time formatting, PKCE/OAuth request building, the sample provider, store durability across reopen, atomic change+operation (a failing change leaves no operation), Outbox queue/cancel, send completion, FTS search and coverage, settings, and the sync engine (offline leaves work queued; ordered replay; uncertain send reconciled without duplicates; cancelled send never sent).

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

- No Gmail yet: OAuth request/PKCE helpers exist; the loopback listener, token exchange, Secret Service storage, and the Gmail API client are next.
- Composer is plain text. Rich text, signatures, and HTML output are phase 1 items; `body_html` is already stored separately.
- Attachment download for received mail is not implemented (cards show “Not downloaded”). Outgoing attachments are copied into the protected `outgoing/` store and persisted with the draft.
- Multi-select and drag-and-drop are not implemented.
- Hyprland draws no title bar; the kit’s window controls are not shown there. GNOME and KDE behaviour, fractional scaling, and screen readers (Orca) have not been checked yet.
- Flatpak tooling is absent on this machine; `packaging/` holds the desktop entry, AppStream metadata, and a manifest draft that still need a build.
- Omarchy applies a default window opacity (0.985 active, 0.96 inactive) to every app, including Overlook. That is compositor policy, not an app setting.
