# Fastmail personal calendars — completion record

23 October 2026. This is the current implementation and verification record; earlier handoffs retain their historical results. The user requested autonomous testing and will judge the application through normal use. No further guided live-account test sequence is pending.

## Available behavior

- Connect calendars to an existing Fastmail IMAP account in Settings → Accounts → Calendars, or opt in while adding a new Fastmail account. Custom-domain mail accounts use the configured incoming server/login. Calendar credentials and recovery remain independent of mail.
- View cached calendars in Day, Week, Month and Agenda; select visibility and a default writable calendar. The default is persisted per account using its provider URL/id, so it survives a fresh local connection identity. Missing or newly read-only defaults fall back to another writable calendar.
- Create, edit and delete personal appointments, including multi-day and all-day events. Changes are committed locally before upload and survive restart. Review conflicts using Keep my changes or Use provider version; unrelated remote fields are preserved when keeping a local edit.
- Edit or cancel an existing recurring occurrence and delete an entire series. Preserve the original resource, timezone components, unknown properties and exceptions during supported edits.
- Reconcile uncertain writes before retrying, recover from authentication/network failures, and keep mail synchronization independent.
- The calendar header stays clear after successful setup and sync, including routine background refreshes. Setup guidance, uncached ranges, offline/error states and pending changes still show relevant guidance; the sidebar retains compact sync status.
- Repeated reads use collection sync tokens to avoid downloading unchanged calendar bodies. Default settings clearly identify read-only calendars. Dragging appointments between dates keeps their local clock time across daylight-saving transitions; multi-day labels compare calendar dates instead of elapsed 24-hour periods.

## Collection synchronization

The provider requests DAV sync-token during discovery and uses the RFC 6578 sync-collection report when a matching checkpoint is available. Only a complete empty multistatus with a single valid token permits reuse of cached event projections. Changed, expired, malformed, truncated or unsupported reports fall back to a complete bounded query for that collection. Network/authentication failures retain the previous cache and checkpoint.

This is collection-level incremental synchronization: unchanged collections transfer no event bodies; a changed collection still downloads matching original and server-expanded reports. Per-resource multiget optimization is not implemented or needed for correctness. Existing response, resource, occurrence and time limits remain in force.

Schema V16 adds `caldav_checkpoints`. Tokens include the calendar, display range and timezone, and commit atomically with their snapshots. Reuse checks the previous checkpoint again inside the transaction. A concurrent local edit, invalidated coverage, changed timezone/range, disconnected account or replaced connection cannot silently certify stale events. Collections with queued edits do not acquire checkpoints. Initial tokens are captured before event reads, so concurrent server changes cause another refresh instead of being skipped.

Protocol reference: [RFC 6578](https://www.rfc-editor.org/rfc/rfc6578.html).

## Independent interoperability

`scripts/qualify-caldav.py` starts Radicale 3.8.3 with temporary storage, synthetic credentials, an ephemeral loopback port and the repository's public TLS fixture. Both the Python seed client and Rust client verify TLS against the fixture certificate and hostname. The test-only Rust resolver rejects non-loopback addresses. The ordinary application build does not include this transport.

The runner provisions two calendars and standalone, recurring and all-day resources, then invokes the ignored `reference_server_round_trip` integration test. It shuts down its exact server process and removes its temporary data on success or failure. There are no hosted-account writes or keyring access.

Qualification exercises discovery, writable collection permissions, original/expanded reports, recurrence through the Europe/Berlin autumn transition, all-day boundaries, no-change token reuse, SQLite restart with queued edits, conditional creation/update/deletion, editing and cancelling recurring occurrences, unknown-property preservation, and both conflict choices with two independent local stores talking to the reference server.

Reproduction in a disposable virtual environment:

```sh
python3 -m venv /tmp/overlook-radicale-qualification
/tmp/overlook-radicale-qualification/bin/python -m pip install 'radicale==3.8.3'
TZ=Europe/Berlin /tmp/overlook-radicale-qualification/bin/python scripts/qualify-caldav.py
```

The initial recurrence fixture omitted VTIMEZONE and exposed the server library's fallback timezone behavior. The final fixture includes its timezone definition; the test retains its 25-hour/24-hour occurrence-spacing assertions. This was a fixture correction, not evidence that Overlook can repair incorrect server-expanded timestamps.

## Verification

The workspace suite passes **358 tests, 29 ignored, 0 failed**: application 73, core 88, browser-document 16, message-formatting 12, provider 128 and store 41. The separately run Radicale test is one of those ignored tests; ordinary suite results do not imply that all ignored tests ran.

Automated HTTPS workflows also cover offline creation/editing/deletion, durable restart, both conflict choices, authentication recovery, and lost successful PUT/DELETE replies without duplicate writes. New checkpoint coverage includes remote additions/deletions, failed refresh without cursor advancement, expiration, truncated reports, timezone/range changes, and a concurrent local edit during cache reuse. Default-calendar tests cover serialization, fresh local IDs, read-only permission changes and missing choices.

The independent Radicale round trip passed separately, including both conflict choices, with `TZ=Europe/Berlin`. The seven non-ignored calendar coordinator workflows and the drag regression also passed in separate Europe/Berlin test runs. The release build succeeded. Clippy passed with existing repository warnings and no diagnostics in the CalDAV modules or new preference logic. New CalDAV modules pass `rustfmt --check`; `git diff --check` passes. Workspace formatting still reports broad pre-existing differences beginning in unchanged `overlook-app/src/assets.rs`; unrelated files were not reformatted.

Temporary logs are `/tmp/overlook-fastmail-final-{tests,clippy,release}.log`, `/tmp/overlook-fastmail-final-workflows-dst.log`, `/tmp/overlook-fastmail-final-drag-dst.log`, and `/tmp/overlook-caldav-reference-dst.log`.

## Scope and verification limits

Personal calendar synchronization is the implemented first-release scope. Meeting invitations/RSVP, new recurring-rule creation, whole-series editing, and “this and following” changes remain disabled and belong to subsequent work. A hidden calendar is still synchronized; visibility is a display choice, not a collection-subscription optimization.

Earlier hosted Fastmail connection and online edits passed in both directions. Hosted offline/conflict testing remains inconclusive, separate from the successful automated fixtures and independent-server checks. No additional real-account changes were made in this increment.

Earlier native Light/Dark CRUD, keyboard/focus and conflict observations remain in the handoff. This increment's new settings/default control has compile and logic coverage, but a fresh native visual/accessibility pass was unavailable: the current UI automation surface disables native app control. Screen-reader, IME, scale and sleep/wake qualification are not claimed. The first-release plan's complete native/hosted acceptance checklist must not be marked fully passed on the strength of headless tests.

The project release binary is `target/release/overlook`. The installed desktop application is not replaced by a Cargo build. Changes remain uncommitted; no release or system installation is performed.
