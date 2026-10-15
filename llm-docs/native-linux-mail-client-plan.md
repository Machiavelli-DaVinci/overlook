# Native Linux Mail Client Plan

Build **Overlook, a mail-focused Linux app with the spacious layout and familiar interactions of New Outlook**, with visual design treated as a core engineering requirement. Overlook has its own polished product identity, with Linux, Arch, and Omarchy as supported environments.

The [Overlook brand system](overlook-brand.md) defines its Last Refuge family identity: a horizon-inspired O mark, an editorial serif wordmark, clear sans-serif interface typography, and restrained alpine colors. Apply the supplied [brand assets and Light/Dark tokens](overlook-brand-assets.md) throughout design and native implementation; use landscape imagery in welcome and marketing surfaces while keeping working mail areas clear and readable.

The recommended stack and sequence are **Rust + GTK4, Gmail’s API first, IMAP/SMTP second, and POP3 third**. Calendar and contact management are separate later projects.

Gmail is a service; IMAP and POP3 are protocols for accessing mail. Gmail supports those protocols, but its API gives us direct access to Gmail’s labels, threads, and synchronization model. **SMTP belongs in phase 2**, because IMAP handles mailbox access but does not send messages. [Gmail protocols](https://developers.google.com/workspace/gmail/imap/imap-smtp), [IMAP specification](https://datatracker.ietf.org/doc/html/rfc9051).

The proposed roadmap is:

| Stage | Deliverable | Estimated effort | Status (15 October 2026) |
|---|---|---:|---|
| Foundation | Visual direction, interactive native prototype, technical validation | 2–4 weeks | Done (see [the foundation plan](foundation-implementation-plan.md)) |
| Phase 1 | Beautiful, dependable Gmail client | 12–18 weeks | In daily use |
| Phase 2 | General IMAP accounts with SMTP sending | 8–12 weeks | Built through milestone M5 of the [IMAP and POP3 plan](imap-pop3-implementation-plan.md): setup dialog with autoconfiguration, folders as labels, incremental sync with `CONDSTORE`/`QRESYNC`, `IDLE`, moves, flags, SMTP with the Sent copy, server-side search. Verified against scripted servers only; the real-server matrix needs a test account. XOAUTH2 (Microsoft 365) outstanding. |
| Phase 3 | POP3 accounts using the existing SMTP system | 4–6 weeks | Built (milestone M4): download-and-keep with `.eml` primary storage, retention policy, backup. Scripted servers only. |

These are planning estimates for **two experienced full-time engineers with part-time design and testing support**: approximately 6–10 months overall. Re-estimate after the foundation stage; public Gmail approval is an additional external dependency.

**The experience should feel familiar from the first launch.** The default window would have an account and folder sidebar, a message list, and a generous reading pane. A persistent search field and restrained command bar would anchor the top.

| Area | Planned experience |
|---|---|
| Navigation | Favorites, accounts, Inbox, Drafts, Sent, Archive, and labels with readable counts |
| Message list | Clear sender/subject hierarchy, useful previews, date grouping, attachment indicators, multi-selection |
| Reading | Comfortable typography, expandable conversations, readable quoted replies, inline attachment cards |
| Compose | Inline reply and separate compose windows; rich text, signatures, attachments, and automatic draft recovery |
| Layout | Resizable panes, right or bottom reading pane, comfortable and compact density |
| Interaction | Outlook-style shortcuts, predictable focus, drag-and-drop, immediate feedback, undo where possible |
| Connectivity | Offline reading and search of downloaded mail, local drafts and queued changes, and automatic synchronization after reconnecting |
| Appearance | Exactly two built-in themes for v1.0: Light and Dark, with a consistent Overlook accent, icons, and subtle motion |
| Accessibility | Complete keyboard operation, screen-reader labels, scalable text, high contrast, reduced motion |

“Beautiful” needs concrete review criteria: realistic crowded inboxes must look as considered as empty states, long subjects must wrap gracefully, and synchronization must never move the selected message unexpectedly. Dark mode must preserve email readability without blindly inverting images and brand colors.

**Theming is a v1.0 requirement, limited to Light and Dark.** Both themes use Overlook’s own visual language consistently across supported desktops.

- Provide an Appearance setting with Light, Dark, and Follow system. Follow system selects between the same two themes; default to it when a desktop preference is available, with Light as the fallback.
- Persist the appearance setting across restarts. Apply changes immediately to all open windows without interrupting composition, selection, or synchronization. Follow desktop appearance changes only while Follow system is selected.
- Define shared semantic color tokens for surfaces, text, borders, selection, focus, and status. Apply them consistently to the app shell, message lists, reading pane, composer, dialogs, menus, and empty, loading, and error states.
- Coordinate native UI and embedded HTML reader/editor styling. Preserve attachment images and sender content colors where necessary for readability; provide an original-appearance option for messages that do not adapt safely. Changing the app theme must not change the formatting of outgoing mail.
- Defer custom themes, user-selectable accent palettes, theme imports, and a theme marketplace until after v1.0.

**For the implementation, use this proposed stack.**

| Component | Choice and purpose |
|---|---|
| Native interface | GTK4 through Rust bindings, with selected libadwaita components and custom mail views |
| HTML content | WebKitGTK for received HTML and a separately isolated rich-text editor |
| Application core | Rust domain model and command handling, independent of GTK |
| Background work | Tokio for synchronization, networking, and attachment transfers |
| Local storage | SQLite for messages, account state, drafts, and pending operations; FTS5 for search |
| Mail processing | Evaluate `mail-parser` for MIME parsing and `lettre` for message construction and SMTP |
| Credentials | Linux Secret Service integration, with explicit handling of unavailable or locked keyrings |
| Distribution | Flatpak first; an Arch package for Omarchy after the release process stabilizes |

GTK4 has maintained Rust bindings, and libadwaita provides adaptive components and appearance support. WebKitGTK also has GTK4-compatible Rust bindings. This would be a native application written in Rust, using established platform libraries. [GTK Rust documentation](https://gtk-rs.org/gtk4-rs/stable/latest/book/libadwaita.html), [WebKit Rust bindings](https://docs.rs/webkit6/latest/webkit6/).

The interface should read from the local database immediately, whether online or offline. Commit each local change and its pending operation together before reporting it as saved. Background synchronization reconciles this local state with the provider without blocking the interface. Keep unsent messages, drafts, and pending operations separate from replaceable downloaded cache data.

The shared model must distinguish **messages, conversations, labels, folders, and provider identifiers**. Gmail’s labels should remain labels; future IMAP support should not require redesigning storage.

**Offline use and automatic synchronization on reconnect are v1.0 requirements.** After initial account setup and download, users must be able to keep working without an internet connection. Save their work locally immediately and exchange changes with the provider when connectivity returns.

- **Offline mailbox access:** read and search downloaded messages, view downloaded attachments, and browse cached folders and labels. Clearly identify messages and attachments that have not been downloaded; keep available content usable instead of blocking the mailbox behind a connection error. New server mail and uncached content become available after reconnecting.
- **Offline preparation:** provide per-account download settings for message history, folders or labels, and attachments, plus a “Keep available offline” option for selected content. Show download progress and storage usage. Storage cleanup must never evict unsent messages, local drafts, their attachments, or data required by pending operations.
- **Offline editing and organization:** create and edit drafts, reply to downloaded mail, and queue supported read/unread, star, label, move, archive, and trash changes. Show those changes immediately with a pending-sync indication. Persist the queue, draft content, and outgoing attachments across app restarts and crashes.
- **Offline sending:** pressing Send while disconnected places the message in a visible Outbox marked “Waiting for connection.” Let users edit or cancel it before submission begins. Submit queued messages automatically when the account is reachable and authenticated, and show Sent only after confirmed provider acceptance. If a connection drops during submission, preserve an uncertain-delivery state and reconcile it before retrying; do not blindly resend and risk duplicates.
- **Automatic recovery:** synchronize on startup, reconnect, and resume from sleep while the app is running. Detect actual provider reachability as well as network availability. Use bounded retries with backoff for temporary failures and rate limits; request sign-in again for expired or revoked credentials without discarding local work. If the app was closed, resume from saved progress at the next launch.
- **Reconciliation and conflicts:** fetch remote changes and reconcile pending local intent before applying operations in dependency order. Make retried operations safe to repeat where the provider permits it, and persist synchronization checkpoints transactionally with downloaded changes. Merge independent changes; preserve both versions of conflicting drafts for user review. Respect remote deletions, avoid silently recreating deleted mail, and expose changes that could not be applied. A full resynchronization must preserve drafts, the Outbox, and pending local operations.
- **Clear account status:** distinguish Offline, Syncing, Up to date, Sign-in required, and Sync failed. Show the last successful synchronization and pending operation count, with manual retry where useful. One disconnected account must not block other accounts. “Saved locally” and “Synced” must have distinct meanings.

Reuse this local storage and synchronization engine in later phases, with provider-specific capabilities: Gmail and IMAP reconcile supported remote mailbox changes; POP3 retrieves mail into local storage and uses SMTP for queued sending, without promising IMAP-style synchronization of folders or read state.

**The foundation stage should prove the design and the hardest technical assumptions.**

Before building the complete mail engine:

- Design the inbox, conversation reader, composer, search results, account setup, and failure states in both themes.
- Establish the Light and Dark token sets and validate runtime appearance switching in the native prototype and HTML reader/editor.
- Build a native prototype with realistic sample mail and functioning pane resizing, selection, keyboard navigation, and composition.
- Validate HTML rendering using newsletters, receipts, tables, inline images, long threads, and malformed messages.
- Prove browser-based Google sign-in, keyring storage, a minimal synchronization cycle, and packaged execution.
- Prove an offline-to-online cycle early: disconnect, save a draft and mailbox action, restart the app, reconnect, and reconcile the saved work without loss.
- Test the prototype on Omarchy/Hyprland, GNOME, and KDE, including fractional scaling.

The exit criterion is a convincing everyday interaction model and a validated technical foundation. The visual prototype should already resemble the intended finished product.

**Phase 1 should deliver a Gmail client you can use every day.** Build it in four increments:

1. **Account setup and reading.** Google sign-in, multiple Gmail accounts, inbox and label navigation, conversation view, attachments, progressive initial synchronization, and clear progress indicators.
2. **Writing and organization.** Reply, reply-all, forward, rich-text composition, signatures, draft synchronization, archive, trash, spam, labels, stars, read/unread, and bulk actions.
3. **Speed and offline use.** Local search with useful filters, online Gmail search for uncached mail, configurable offline downloads, durable drafts and queued mailbox actions, a visible Outbox, automatic reconnect synchronization, conflict handling, and per-account connectivity status.
4. **Release quality.** Complete v1.0 Light and Dark theming, appearance preference persistence and system preference integration, notifications, `mailto:` handling, accessibility, account removal, migration recovery, packaging, and extended daily-use testing.

Local search must say what it covers. A search of downloaded mail should never silently imply it searched the entire account. SQLite’s FTS5 is a suitable starting point for the local index. [FTS5 documentation](https://www.sqlite.org/fts5.html).

Gmail synchronization should use history-based incremental updates and recover through a full resynchronization when the saved history position expires. For this desktop application, begin with adaptive polling and refresh on reconnect or wake; Google recommends polling for installed applications, while its push system introduces Cloud Pub/Sub infrastructure. [Gmail synchronization](https://developers.google.com/workspace/gmail/api/guides/sync), [Gmail notifications](https://developers.google.com/workspace/gmail/api/guides/push).

Start Google OAuth preparation during the foundation stage. Use the system browser, PKCE, and a loopback callback. Mail access involves restricted scopes, so public release verification must be planned early. Keeping mail processing on the user’s machine simplifies the architecture, but does not automatically remove verification requirements. [Desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app), [Restricted-scope verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification).

Phase 1 is complete when you can use it as your primary Gmail client for several weeks, including offline periods and interrupted synchronization, without losing drafts or encountering unexplained mailbox differences. An offline session must support reading, searching, composing, and organizing cached mail; after reconnecting, new mail and queued changes must reconcile automatically, with any conflicts or uncertain deliveries clearly surfaced.

**Phase 2 should add IMAP and SMTP without changing the familiar interface.**

This phase includes server configuration, supported provider presets, TLS, provider-specific authentication, folder discovery, subscriptions, message moves, draft storage, and SMTP submission.

The difficult work is synchronization correctness: folder-specific identifiers, UID validity changes, reconnects, deletions, server capability differences, and Sent-folder behavior. Use IMAP IDLE where supported, with polling fallback. Evaluate `async-imap` against an interoperability test suite before committing to it; `lettre` is a candidate for SMTP. [async-imap](https://docs.rs/async-imap/latest/async_imap/), [lettre](https://docs.rs/lettre/latest/lettre/).

Test against several independent server implementations and real providers. “Supports IMAP” should mean a documented, tested compatibility range. Microsoft 365 authentication and organizational restrictions would need explicit testing; Outlook familiarity does not imply Exchange feature parity.

**Phase 3 should add POP3 with careful local storage behavior.**

Reuse the composer, search, attachments, outbox, and SMTP transport. Add:

- POP3 retrieval and UIDL-based duplicate tracking.
- A default that leaves messages on the server.
- Clear retention and server-deletion controls.
- Local folders, export, backup, and restore.
- Recovery from interrupted downloads and changed server state.

POP3 offers a simpler retrieval model with fewer synchronized mailbox capabilities. Consequently, downloaded mail may become the user’s primary copy, making backup and recovery release requirements. [POP3 specification](https://datatracker.ietf.org/doc/html/rfc1939).

**Reliability and polish need explicit release gates throughout the project.**

- **Draft safety:** recover saved drafts after crashes, power loss simulations, and database migrations.
- **Sending safety:** distinguish queued, sending, sent, failed, and uncertain outcomes. Investigate uncertain delivery before retrying automatically.
- **Offline synchronization:** test disconnects during downloads, mailbox changes, and sending; restart while offline; resume after sleep or an extended absence; and reconnect after the same mailbox was changed on another device. Verify preserved drafts and outgoing attachments, safe queue replay, recovery from expired sync checkpoints, clear handling of unavailable content and conflicts, and eventual agreement with the provider after successful synchronization. Repeat applicable scenarios for IMAP/SMTP and POP3 as those providers are introduced.
- **Content isolation:** disable scripts in received mail, block remote resources by default, sanitize content, and isolate the trusted composer from received HTML.
- **Credential safety:** keep tokens out of logs and ordinary configuration files; protect local mail files and document their storage policy.
- **Performance:** set initial targets of under two seconds to a usable cached inbox and under 200 milliseconds for common local searches on a defined 100,000-message test dataset.
- **Usability:** complete reading, replying, searching, filing, and attachment workflows entirely by keyboard; verify scaling and screen-reader behavior.
- **Theming:** visually verify every core screen and interaction state in Light and Dark on Omarchy/Hyprland, GNOME, and KDE. Check text and focus contrast, HTML email readability, system preference fallback, persisted overrides, and switching with an open draft. Theme changes must preserve draft content and outgoing message formatting.

Keep contact synchronization, tasks, Exchange-specific features, PST import, and advanced rules outside these three phases. Basic recipient autocomplete from previous correspondents belongs in phase 1. **Scope change, 12 October 2026:** the user moved calendaring (Google Calendar read, respond, create, delete) and Focused Inbox into phase 1 for daily use; see the native app guide.

The first milestone should be **a polished native inbox, reader, and composer prototype with a small working Gmail connection**. That gives us something concrete to judge for beauty, familiarity, responsiveness, and feasibility before committing months to the full synchronization engine.
