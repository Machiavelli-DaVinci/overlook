# Fastmail onboarding and autonomous workflow verification

23 October 2026. This increment follows the user's instruction to stop assigning manual test scripts and continue building with isolated automated tests. No real account data, credentials, calendar events, browser pages, desktop settings, or installed application were changed in this increment.

## Account setup

The server-settings page for a new Fastmail IMAP account offers **Connect calendars using this mail password**. It is off by default and appears only for the actual incoming host `imap.fastmail.com` (case-insensitive, accepting a trailing DNS dot). The address's domain is not used to infer the service. Calendar login uses the incoming username, so custom-domain addresses with a different Fastmail login work.

Changing the incoming host or username clears the choice. Saving the new account saves its calendar configuration after mail credentials have reached the system keyring, then starts the independent calendar worker. No second password is stored. Offline discovery or insufficient calendar permission leaves the connection repairable under Settings → Accounts → Calendars and does not interrupt mail. Existing connections are never replaced by this setup path. Other CalDAV providers and existing accounts retain the Settings connection flow.

## Shared commands and HTTPS fixture

`caldav_sync::queue_change` and `resolve_change` now contain the durable editor and conflict commands. The native UI calls these functions, then refreshes its view and schedules work. Tests call the same commands instead of reproducing their logic. Each command rejects a mismatched account.

The provider's `fake-servers` feature exposes `caldav::fake::CalendarServer`, a stateful HTTPS service bound to an ephemeral loopback port. Its client trusts only the existing public fixture certificate, verifies its synthetic hostname, disables proxies and uses a resolver pointing only at that socket. It uses synthetic credentials and cannot contact hosted Fastmail. The fixture is absent from the ordinary release build. Its worker is joined when the test ends, and databases use temporary directories.

The server implements collection discovery, reports, resource reads and conditional writes. It enforces ETags, can simulate another client's edits, reject authentication, go offline, and drop a mutation response after saving it. It is deliberately a personal-event test fixture, not an independent CalDAV reference implementation or recurrence-expansion server.

## Verified scenarios

- Onboarding requires opt-in, uses the incoming login, rejects lookalike/unrelated hosts, persists through reopen, and refuses to overwrite an existing calendar connection.
- Calendar authentication failure retains cached events, connection identity and working mail status; restoring access resumes download.
- Offline edits coalesce, an all-day multi-day event is created, and a cached event is deleted. All four queued resources and local projections survive SQLite close/reopen; the server remains unchanged while offline.
- Concurrent provider edits cause two real conflicts through HTTPS. Keep my changes preserves the local title/notes and the remote location. Use provider version discards local changes without another write. Creation/deletion still complete while other events need review.
- Lost PUT and DELETE responses leave Checking operations. After reopen, GET reconciliation clears them without a second mutation or duplicate event.

Run the focused workflows with `cargo test -p overlook caldav_sync --offline`. Loopback sockets must be allowed by the test runner's sandbox. The initial sandbox-only run could not bind sockets; the permitted loopback run passed.

The full workspace run passed **354 tests, 28 ignored, 0 failed**. The ignored tests were not run. The four new workflows also pass with `TZ=Europe/Berlin`, including the October daylight-saving transition and date-based multi-day assertions. Clippy passed with existing warnings and no diagnostics in the new modules. The release binary rebuilt successfully. New-module formatting and `git diff --check` passed; workspace formatting still reports the pre-existing differences beginning in `assets.rs`.

No live-service pass is inferred from these results. The new onboarding toggle's native visual/accessibility behavior remains unverified in this increment because native UI control is unavailable; prior native CRUD/conflict observations remain recorded separately. The user is not being asked to perform this verification.

Temporary logs: `/tmp/overlook-fastmail-workflows.log`, `/tmp/overlook-fastmail-workflows-dst.log` and `/tmp/overlook-fastmail-onboarding-{tests,clippy,fmt,release}.log`. Subsequent collection-token synchronization and independent reference-server qualification are complete; see [the current completion record](fastmail-calendar-completion.md). Meeting support remains a separate milestone.
