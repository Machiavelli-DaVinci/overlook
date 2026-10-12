# Overlook session handoff

Continue the Overlook project from the context below. Read the saved documents and inspect the current workspace before making changes. Preserve the agreed product and brand direction.

## Workspace and instructions

Project root: `/home/jason/coding/omarchy-overlook`

Read any applicable `AGENTS.md` instructions and `/home/jason/.codex/RTK.md`. Keep written project documentation in `llm-docs/`, branding assets and previews in `branding/`, application code in `crates/`, packaging files in `packaging/`, and helper scripts in `scripts/`.

Do not assume this session's running processes, scratch profiles, or tool handles still exist. Inspect the environment and version-control state before starting. The repository was initialised with `git init` on 12 October 2026 and **has no commits yet**; the user decides when to commit.

## Read these first

1. `llm-docs/README.md` — documentation index.
2. `llm-docs/foundation-implementation-plan.md` — the validated technical foundation (GPUI), tradeoffs, workspace rules, milestone sequence, status log.
3. `llm-docs/overlook-native-app.md` — how to build, run, and test the app; architecture; keyboard; known gaps.
4. `llm-docs/native-linux-mail-client-plan.md` — authoritative roadmap, offline requirements, and release gates.
5. `llm-docs/overlook-brand.md` and `llm-docs/overlook-brand-assets.md` — the selected identity and asset inventory.
6. `branding/previews/overlook-native-*.png` — captures of the running native app (light, dark, offline Outbox, reconnected, compose) next to the earlier HTML design captures `overlook-mail-light.jpg` / `overlook-mail-dark.jpg`.

## Decisions made on 12 October 2026

- **UI framework is GPUI**, by the user's explicit direction ("Let's make sure we are using Rust and GPUI"). The GTK4/libadwaita/WebKitGTK proposal is withdrawn. The app depends on `gpui-kit` 0.7.3, which pins `gpui-pre` 0.3.9 as `gpui` and provides inputs, resizable panels, menus, notifications, and a native HTML text view. Do not mix `gpui` 0.2.2 into the tree (it also fails to build on Rust 1.99 because of an old `xattr`).
- **No embedded web view.** Received HTML is sanitized in `overlook-core` and rendered natively. Fidelity for layout-table email is limited (tables are flattened into rows); this is the main open technical risk and a foundation gate. See the plan document for the fallback options.
- **Sample account** seeds the fictional mailbox on first launch so the shell is local-first from the start. Its connection is a visible switch in the status bar; it starts disconnected. It never pretends to be a real provider.

## What exists now

- Cargo workspace (Rust 1.99 pinned): `overlook-core`, `overlook-store`, `overlook-provider`, `overlook` (GPUI app). `cargo test --workspace` passes 33 tests; `cargo build` is warning-free.
- Native shell matching the brand previews in Light and Dark: header with mark and wordmark, search, resizable sidebar/list/reader, right or bottom reading pane, comfortable/compact density, day-grouped conversation list, conversation reader with collapsed history, attachment cards, remote-image blocking with per-message “Show images”, composer (new/reply/reply-all/forward) with autosave, recipient suggestions, attachments copied into a protected store, Outbox, status bar with truthful sync wording, toasts, keyboard navigation, Light/Dark/Follow system persisted and applied instantly.
- Durable offline behaviour proven by test and by a scripted run: local change + pending operation in one transaction, Outbox queue, `kill -9`, restart offline, reconnect, ordered replay, provider acceptance before Sent, uncertain-delivery reconciliation without duplicates.
- Gmail preparation: scopes, PKCE, authorization URL, loopback callback parsing, token request body (pure functions with tests). No network, no credentials, no Secret Service yet.
- Packaging drafts in `packaging/` (desktop entry, AppStream metadata, Flatpak manifest). Not built: `flatpak-builder` is not installed here.

## Gmail (added later on 12 October 2026)

Working end to end against the user's real account: sign-in through the system browser with PKCE and a loopback listener, tokens in the GNOME keyring via `oo7`, 150 recent messages downloaded in persisted batches of 20, incremental sync through the history API, read/star/archive/trash/label through `batchModify`, send through `messages.send` with threading headers and Message-ID reconciliation. Multi-account window with an account menu (switch, add Gmail, sign in again, remove). Gmail quota is tight in a fresh project; the engine spaces requests and enters a **Paused** state with backoff when throttled. Read `llm-docs/overlook-native-app.md` → Gmail. The OAuth client file lives at `~/.config/overlook/google-oauth-client.json` and is not in the repository. **Never send mail from the user's account without an explicit request.**

## Daily-use features (added later on 12 October 2026, at the user's request)

Focused Inbox (local classification with per-sender rules), attachment download/open/save, and Google Calendar (week/day/month/agenda, invitations with Accept/Tentative/Decline in the calendar and inside the mail reader, new/edit/delete event, reminders with desktop notifications, event colors, incremental sync) are implemented and validated against the user's real account: 1,189 event instances downloaded, working-location entries shown in the day header. Calendar was deferred in the product plan; the user explicitly moved it up. See `overlook-native-app.md`.

## Settings (added later on 12 October 2026)

A Settings dialog (`Ctrl ,`) with Accounts (signatures, sender name, status, remove), General, Mail, Calendar, and About pages; see `overlook-native-app.md` → Settings. Preferences are typed in `overlook_core::prefs` and every control changes real behaviour. Not built: multiple signatures, automatic replies, rules/sweep.

## What does not exist yet

- Inline `cid:` images, progressive download of older mail history, Gmail batch requests.
- Calendar: recurring-series editing (edits apply to the single instance), attendee editing in the form, drag to move or resize, time-zone display for events in other zones, calendars from more than one account merged in one view.
- Rich-text composition, signatures, HTML output. Received-attachment download. Multi-select, drag-and-drop, undo.
- A proper table layout in the HTML view; GNOME/KDE/fractional-scaling checks; screen-reader checks; Flatpak build.

## Recommended next milestone

1. **Daily-use feedback** on mail, Focused Inbox, attachments, and calendar (the user is now living in Overlook): rendering issues, missing actions, sync correctness. Event editing and invitation handling inside mail are the most likely next asks.
2. **HTML fidelity decision.** Run the plan's corpus (newsletters, receipts, tables, inline images, long threads, malformed messages) through the sanitizer + native view. Decide between improving the native renderer (table layout, inline images via `cid:` from the attachment store), or an out-of-process renderer. Record the decision in the foundation plan.
3. **Deliberate send test** from the user's own address when they ask for it, watching the Sent copy, threading, and the Outbox states.
4. Keep the previews fresh with `scripts/capture-window.sh`, repackage the brand kit after documentation changes, and extend tests for every reliability gate touched.

Judge progress by what the running app does next to the design captures, and by `cargo test`, not by whether a window opens.

## Commands

```bash
cargo test --workspace
```

```bash
cargo run -p overlook
```

```bash
OVERLOOK_DATA_DIR=/tmp/overlook-demo OVERLOOK_SAMPLE_ONLINE=1 cargo run -p overlook
```

```bash
scripts/capture-window.sh /tmp/overlook-demo /tmp/overlook.png
```

Brand assets and the HTML design preview keep their existing maintenance commands (`branding/source/build_assets.py`, `prepare_mail_preview.py`, `package_brand.py`); see `llm-docs/overlook-brand-assets.md`.
