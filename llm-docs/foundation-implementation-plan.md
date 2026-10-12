# Overlook Foundation Implementation Plan

This document records the concrete implementation sequence for the foundation stage described in the [product plan](native-linux-mail-client-plan.md). It separates **user requirements that are fixed** from **technical decisions that were validated on 12 October 2026** and from **questions that remain open**. Update it as milestones land.

## Fixed requirements (do not re-litigate)

- Native Linux mail client written in Rust, by Last Refuge, named Overlook. Outlook on the web is the interaction reference; the [brand system](overlook-brand.md) is the visual reference.
- Mail only in v1. Gmail API first, IMAP/SMTP second, POP3 third.
- Exactly two themes, Light and Dark, plus a Follow system setting that chooses between them. Persisted, applied instantly, never changing outgoing mail formatting.
- Offline work is core: local-first reads, atomic local change + pending operation, visible Outbox, truthful status language ("Saved on this device" is not "Synced" or "Sent").
- Received HTML is untrusted: no scripts, remote resources blocked by default, isolated from the composer.

## UI framework decision: GPUI

On 12 October 2026 the user directed that the native UI be built with **Rust and GPUI** (Zed's GPU-accelerated UI framework). The earlier GTK4/libadwaita/WebKitGTK proposal is withdrawn. Evidence gathered on this machine:

| Check | Result |
|---|---|
| `gpui` on crates.io | `gpui` 0.2.2 (older snapshot); `gpui-pre` 0.3.9 (newer snapshot used by GPUI Kit) |
| Component library | `gpui-kit` 0.7.3 re-exports `gpui-pre` 0.3.9 as `gpui`, plus `gpui-component` (inputs, textarea, resizable panels, virtual lists, native HTML/Markdown text view, menus, dialogs, AccessKit accessibility) and 1,830 bundled Lucide icons |
| Clean build of the full dependency set | 39 s wall clock on this 16-core machine |
| Platform | Wayland (Hyprland) with Vulkan (NVIDIA + Intel ICDs present) |
| System appearance | GPUI reads `org.freedesktop.appearance color-scheme` through the settings portal (`ashpd`); `xdg-desktop-portal-gtk` is installed and currently reports prefer-dark |
| Fonts | Inter and Source Serif 4 are not installed system-wide; GPUI loads bundled font bytes through `text_system().add_fonts`, so the app bundles them |
| `gpui` 0.2.2 alone | Failed to compile on Rust 1.99 because of an old `xattr` 0.2.3 dependency; `gpui-kit`/`gpui-pre` resolves a working set. Use the kit pairing |

Decision: depend on `gpui-kit = "0.7.3"` (which pins `gpui-pre` 0.3.9 as `gpui`). Do not mix `gpui` 0.2.2 into the tree.

### Consequences and tradeoffs to keep visible

1. **HTML mail rendering.** GPUI has no embedded web view. The foundation renders received mail through `gpui-component`'s native HTML text view after sanitizing with `ammonia` (scripts, styles, forms, event handlers, and remote resources removed). This is secure by construction and theme-aware, but fidelity for complex newsletters, nested tables, and CSS-heavy receipts will be below WebKit. Validation of the plan's HTML corpus (newsletters, receipts, tables, inline images, malformed messages) is a foundation gate; if fidelity is unacceptable, the fallback options are a dedicated HTML layout engine or an out-of-process renderer that produces an image plus link map. "Original appearance" means rendering sender colors inside the sanitized view, not swapping engines.
2. **Rich-text composition.** The composer uses the kit's multi-line editor with plain text in the foundation. Rich text (bold, lists, links) and HTML output are a phase 1 item; the storage model keeps `body_text` and `body_html` separate so this does not require migration.
3. **Accessibility.** GPUI Kit exposes AccessKit roles and names. Screen-reader behaviour on Linux (Orca over AT-SPI) must be tested explicitly before release; it is not assumed.
4. **Window chrome.** On Wayland the app draws its own title bar region. Hyprland, GNOME, and KDE behaviour (move, resize, maximize, fractional scaling at 1.25) must be checked on each.
5. **Packaging.** Flatpak tooling is not installed on this machine; the Flatpak manifest and desktop/metainfo files are authored in the repo, and the build is validated when the tooling is available. GPUI needs Vulkan inside the sandbox (standard GL/Vulkan extensions in the Freedesktop runtime).

## Workspace layout

```
Cargo.toml                    workspace (resolver 2, Rust 1.99 pinned)
crates/overlook-core          domain model, commands, sanitizer, appearance logic; no UI, no SQLite
crates/overlook-store         SQLite + FTS5 persistence: accounts, labels, messages, drafts, pending operations, settings
crates/overlook-provider      MailProvider trait, sample provider (offline/online simulation), Gmail OAuth/PKCE preparation
crates/overlook-app           GPUI application: shell, sidebar, message list, reader, composer, appearance, status
branding/                     brand assets; fonts and logos are embedded into the app binary from here
llm-docs/                     documentation
```

Rules:

- `overlook-core` and `overlook-store` must compile and test without a display. They hold the behaviour the plan's reliability gates measure.
- Messages, conversations, labels, folders, and provider identifiers are distinct types. Gmail labels stay labels; `Folder` exists for IMAP but is unused in phase 1.
- Every local change that needs provider reconciliation is written together with its `PendingOperation` in one SQLite transaction. The UI only reports "Saved on this device" after that transaction commits.
- Drafts, Outbox items, outgoing attachments, and pending operations live in tables that cache eviction never touches.

## Milestone sequence

1. **Environment and plan** (done 12 Oct 2026): toolchain default set, git initialised, GPUI validated, this document.
2. **Core + store with tests**: domain types, appearance resolution, HTML sanitizer, SQLite schema v1 with FTS5, atomic change+operation, draft durability, restart-while-offline test, search coverage reporting.
3. **Branded native shell**: window, header with mark and wordmark, search field, three resizable panes, message list with day groups, reader with conversation history and attachment cards, composer, status bar. Light/Dark/Follow system persisted. Keyboard navigation.
4. **Durable offline demonstration**: sample provider with a connection toggle; star/read/archive/trash and Send queue while "offline", survive restart, replay in order when "online"; uncertain-delivery state when the connection drops mid-send.
5. **Gmail slice**: OAuth (system browser, PKCE, loopback), token storage in Secret Service, label list + recent messages, history-based incremental sync, send one test message through a deliberate test workflow. Credentials are an explicit external dependency.
6. **Validation**: HTML corpus in both themes, fractional scaling, GNOME/KDE sessions, Flatpak build, screenshots refreshed in `branding/previews/`.

Judge milestones 3 and 4 by looking at the running app next to `branding/previews/overlook-mail-light.jpg` and `overlook-mail-dark.jpg`, and by the restart test, not by whether a window opens.

## Status log

- 2026-10-12: Milestones 1 to 4 complete. Workspace scaffolded; 33 tests pass; branded shell in both themes; durable offline → crash → restart → reconnect scenario demonstrated with the sample provider (captures in `branding/previews/overlook-native-*.png`). Gmail preparation (PKCE/auth URL/callback parsing) in place without network code. Packaging files drafted, not built.
- 2026-10-12 (later): Milestone 5 landed. Google sign-in (system browser, PKCE, loopback), tokens in the Secret Service, Gmail label and message download in persisted batches, history-based incremental sync with expired-checkpoint recovery, `batchModify`/trash operations, `messages.send` with Message-ID reconciliation, multi-account window with account switcher and Sign-in required/Paused states. Validated against a real Gmail account: 150 messages downloaded, incremental sync picked up new mail. Gmail's per-user quota in a fresh project is tight; request spacing and Paused-with-backoff were added after the first attempt tripped it.
- Open foundation gate: HTML fidelity. The native text view ignores `<br>` inside paragraphs and cannot lay out tables, so the sanitizer converts in-paragraph breaks to paragraphs and flattens tables into rows with wide cell gaps. Readable, but column alignment and layout-table design are lost. Decide in the next session whether to extend the native renderer or add an out-of-process renderer (see consequence 1 above).
