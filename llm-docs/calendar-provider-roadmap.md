# Calendar Provider Roadmap

Written 22 October 2026. This roadmap extends the [IMAP calendar implementation plan](imap-calendar-implementation-plan.md). Fastmail is the first implementation target, followed by other services using the same CalDAV engine.

## Shared implementation

Build discovery, resource storage, iCalendar interpretation, recurrence, offline operations, conditional writes, conflicts, and reminders once. Keep provider presets separate from protocol behavior. Each preset supplies suggested endpoints, authentication guidance, and tested compatibility rules. All providers use the existing Overlook calendar views and Light/Dark themes.

Mail and calendars can be hosted by different companies. Preserve separate usernames, credential references, service status, and retries. A custom email domain must not prevent Fastmail recognition from the configured mail server. Include manual CalDAV setup; a successful connection is distinct from a provider passing Overlook's full release tests.

## Rollout order

| Order | Provider | Reason and required qualification |
|---|---|---|
| 1 | Fastmail | The user's provider and the first complete experience: existing-account setup, discovery, read/write sync, offline changes, recurrence, and conflicts. Use `https://caldav.fastmail.com/` and an app password with calendar access. [Connection settings](https://www.fastmail.help/hc/en-us/articles/1500000278342-Server-names-and-ports) |
| 1 alongside Fastmail | Nextcloud test instance | Exercise an independently implemented server, explicit URLs, app passwords, and separate mail/calendar hosts. This guards against building assumptions specific to Fastmail into the shared engine. [CalDAV setup](https://docs.nextcloud.com/server/stable/user_manual/en/groupware/sync_thunderbird.html) |
| 2 | iCloud | A useful target for Linux users with iPhones. Test principal discovery, host redirects, app-specific authentication, shared calendar permissions, and recurrence. [Authentication](https://support.apple.com/en-us/121539), [existing CalDAV client integration](https://caldavsynchronizer.org/help/documentation/#icloud-settings) |
| 2 | mailbox.org | A close standards-based alternative with IMAP/SMTP and CalDAV. Prioritize interoperability and likely audience fit; no verified current subscriber count is assumed. [Supported protocols](https://kb.mailbox.org/en/private/faq/compatible-web-browsers/) |
| 3 | GMX and WEB.DE | Their operator reported about 35 million monthly active accounts combined in November 2025; this is account reach, not Linux calendar usage. Test separate mail/calendar app passwords and timezone compatibility. [Audience](https://www.united-internet-media.de/de/newsroom/vermarkterblog/blog/show/e-mail-markt-2025-gmx-und-webde-legen-zu/), [GMX setup](https://hilfe.gmx.net/kalender/mobil/ios.html), [WEB.DE setup](https://hilfe.web.de/kalender/mobil/ios.html) |
| 3 | Yahoo | Documented CalDAV service; qualify discovery, app-password setup, recurrence, and scheduling before advertising support. [Calendar access](https://help.yahoo.com/kb/SLN4707.html) |
| 3 | Zoho | Business-oriented opportunity. Test regional endpoints, enabled calendars, plan restrictions, and different read/write permissions by calendar type. [CalDAV capabilities](https://www.zoho.com/calendar/help/setup-caldav-sync.html) |

These priorities are product decisions, not measured demand for Overlook. Reorder the later providers using customer requests, successful beta connections, and support cost. Avoid estimating a reachable market from total accounts alone.

Microsoft 365 and Outlook.com need a separate Microsoft Graph adapter but can share the domain model, storage, and UI. [Microsoft calendar API](https://learn.microsoft.com/en-us/graph/outlook-calendar-concept-overview). Proton Calendar currently has no CalDAV support and is outside this rollout. [Proton calendar compatibility](https://proton.me/support/subscribe-to-external-calendar)

## Fastmail delivery sequence

1. **Connection and discovery foundation:** validated configuration without secrets, durable connection identity, Fastmail preset, bounded authenticated discovery, collection permissions, and deterministic protocol tests.
2. **Read synchronization:** a proven iCalendar codec, original resource retention, recurrence/timezone projection, atomic checkpoints, independent service scheduling, and cached offline calendar views.
3. **Account integration:** existing-account Calendars settings and new-account discovery, keyring handling, collection selection, repair/disconnect, and accurate status text.
4. **Two-way synchronization:** durable versioned operations, conditional create/update/delete, safe recurrence edits, lost-response reconciliation, and conflict recovery.
5. **Release qualification:** Fastmail plus a reference server; Google/mail regressions; native Light/Dark, keyboard, accessibility, and restart tests.
6. **Meeting support:** separately verify server scheduling and email delivery before enabling attendees, invitations, or RSVP for CalDAV accounts.

The application now supports personal-event creation, editing and deletion with durable offline changes and explicit conflict review. Invitation responses and delivery remain disabled. See the [write-sync implementation record](fastmail-calendar-write-sync.md) for supported recurrence scopes and recovery behavior. Hosted-service compatibility requires live qualification; fixture tests alone do not establish it.

## Implementation status

- Provider priorities and Fastmail connection requirements documented.
- First backend increment implemented: validated CalDAV settings, durable connection identity and configuration generations, schema V13, the Fastmail preset, and authenticated calendar discovery with collection permissions.
- Deterministic HTTP/TLS fixtures cover principal/home/collection discovery, redirects, credential boundaries, authentication errors, malformed/oversized XML, and certificate/hostname rejection. Migration tests preserve Google events and pending operations.
- Second increment implemented: bounded original/expanded CalDAV REPORT snapshots, iCalendar event parsing, server-expanded recurrence, date/time-zone handling, schema V14 resource and coverage storage, atomic reconciliation, and independent calendar scheduling. Cached events survive failures and restart.
- Settings → Accounts → Calendars now supports a Fastmail preset, explicit password choice, connection validation, keyring storage, calendar visibility, reconnect, and disconnect. Day/Week/Month/Agenda consume the shared cache. Calendar views distinguish missing connections, sign-in failures, and uncached ranges. See the [read-sync implementation record](fastmail-calendar-read-sync.md).
- Third increment implemented: personal-event conditional writes, a durable offline outbox, crash recovery, recurrence occurrence edits/cancellation, and conflict review. Native Light/Dark checks exercised creation, editing, deletion, forced-kill persistence, and both conflict choices. Keyboard/focus bugs and an invalid last-download date found during those checks were fixed.
- Final validation on 22 October 2026: `cargo test --workspace --offline` — **346 passed, 28 ignored, 0 failed**. The prior POP3 failure was a test-fixture publication race, now synchronized. Release build and `cargo clippy --workspace --all-targets --offline` passed, with existing Clippy warnings and no diagnostics in the new CalDAV/connection modules. New Rust modules pass `rustfmt --check`, and `git diff --check` passes. Workspace `cargo fmt --all -- --check` still fails on widespread repository formatting differences, including unchanged `overlook-app/src/assets.rs`; unrelated files were not reformatted. See the [verification handoff](fastmail-writes-handoff.md).
- New Fastmail IMAP accounts now offer optional calendar connection during setup, using an explicitly selected incoming-mail credential. Calendar failures remain independent of mail. Stateful HTTPS workflows cover offline restart, both conflict choices, authentication recovery and lost mutation replies through the real commands/client/store. The 23 October suite passes 354 tests with 28 ignored; see [onboarding and autonomous workflows](fastmail-onboarding-and-workflows.md).
- Final personal-calendar increment: collection-token sync skips unchanged calendars, checkpoints commit atomically with their cache, and default-calendar choices persist across reconnection. An independent Radicale 3.8.3 server passed recurrence, CRUD, restart and both conflict choices over verified loopback TLS. DST dragging and multi-day labels are corrected. The final suite passes **358 tests, 29 ignored**, with the Radicale test passed separately; Clippy and release build passed. See the [completion record](fastmail-calendar-completion.md).
- Hosted Fastmail connection and online edits in both directions were observed; hosted offline/conflict verification remains inconclusive. The user has ended guided test scripts and will validate through normal use. Meeting support remains a separate milestone. Native accessibility-tree inspection could not expose the earlier test app; native control is unavailable in the current automation surface. Fresh visual/accessibility qualification remains unverified.

See the [detailed plan](imap-calendar-implementation-plan.md) for persistence, security, offline behavior, recurrence constraints, and release acceptance criteria.

## Native verification — second increment (historical)

Observed in the release build on Linux/Wayland, using an isolated `/tmp` profile with synthetic CalDAV cache data and no real calendar credentials:

- Cached events render in Day, Week, Month, and Agenda and remain visible after app restart and a calendar sign-in failure.
- Calendar connection controls render in both Light and Dark: server URL, username, explicit mail-password reuse, masked app-password input, reconnect/disconnect, last-download information, and visibility switches.
- Ctrl+comma opens settings; Tab/Enter reaches Accounts; Page Up/Page Down scrolls the form. Closing settings returns focus to Calendar and its view shortcuts work. Native checks found and corrected hidden-mail-pane focus restoration and missing settings keyboard scrolling.
- New event is disabled for the read-only calendar. Store tests also reject edits, deletes, and responses without queuing an operation.
- All test app processes and their child processes were closed after inspection. The user's normal profile, credentials, and desktop configuration were not modified.

Accessibility-tree inspection could not find the test process in AT-SPI, so screen-reader behavior and accessible names remain **unverified** despite explicit roles/labels in the implementation. Real-account keyring/authentication, Fastmail/Nextcloud interoperability, invitation delivery, narrow windows, alternative display scales, IME, suspend/resume, and crash-kill persistence are also unverified. Native screenshots showed the existing compositor/window transparency; no compositor settings were changed. These checks are development evidence, not full release qualification.
