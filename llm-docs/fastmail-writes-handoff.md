# Fastmail calendar writes — implementation and verification handoff

Updated 22 October 2026 after resuming the user's requested pause. The POP3 failure has been investigated and fixed, the remaining local checks have finished, and the native calendar write workflows have been exercised. All source changes are saved and uncommitted. No installation, release, live Fastmail mutation, or real-account credential use occurred. Preserve the prior read-sync implementation, `.claude/settings.local.json`, and the historical `llm-docs/casa/` evidence.

## Current continuation — 23 October 2026

The personal-calendar implementation is complete for normal-use evaluation in the project release build. See [the current completion record](fastmail-calendar-completion.md) for collection-token sync, persistent default-calendar settings, DST fixes and independent Radicale interoperability. The current full suite passes **358 tests, 29 ignored**, with the Radicale test run and passed separately. Clippy and release build passed. The hosted connection and online edits passed in both directions; see [live qualification](fastmail-live-qualification.md). Hosted offline/conflict testing and fresh native accessibility/visual qualification remain incomplete and must not be inferred from automated results. The user ended guided testing; do not ask them to run more scripts. The sections below preserve earlier verification history.

## Live setup follow-up — 23 October 2026

The user began connecting their real account in the project build and received HTTP 404. Public unauthenticated PROPFIND probes confirmed Fastmail's bare root returns 404 and `/.well-known/caldav` redirects to `/dav/calendars`. Discovery now falls back to that same-origin service hint only for an initial bare-root 404/405. Explicit URLs and later discovery failures remain errors; credentials cannot follow redirects to another origin. See the [read-sync correction](fastmail-calendar-read-sync.md#fastmail-root-discovery-correction--23-october-2026).

Four new protocol regressions cover the successful redirect/principal/home sequence, explicit missing resources, later discovery failures, and fallback error/credential boundaries. The final workspace result is now **350 passed, 28 ignored, 0 failed**. Logs for this follow-up use `/tmp/overlook-caldav-discovery-{tests,clippy,release}.log`. The agent used no account credentials and performed no live calendar mutations; successful authenticated setup remains pending the user's retry.

The release binary was rebuilt successfully. Clippy passed with existing warnings and no CalDAV-module diagnostics; changed-module formatting and `git diff --check` passed. Workspace formatting still reports the existing repository differences. No native UI code changed in this follow-up, no test app was launched, and the user's running app was left open. It can retry immediately with the explicit well-known URL, or use the root URL after restarting the rebuilt project binary.

## Implemented scope

See [write-sync details](fastmail-calendar-write-sync.md) and the [read-sync record](fastmail-calendar-read-sync.md). V15 stores a durable outbox per CalDAV resource. Local event projection and queued intent commit atomically. Conditional CRUD, stable UID/URL, preflight GET, strong ETags, lost-response reconciliation, revision/connection guards, offline coalescing, and conflict choices are wired into the existing calendar UI. Personal events, multi-day dates, occurrence edits/cancellation, and whole-series deletion are implemented. Meeting writes, series-wide editing, and following-series splits remain deferred.

Key files:

- Core: `crates/overlook-core/src/caldav_change.rs` and `calendar_connection.rs`.
- Provider: `crates/overlook-provider/src/caldav/`, including lossless resource editing in `edit.rs` and conditional reconciliation in `write.rs`.
- Store: `crates/overlook-store/src/caldav.rs`, `caldav_outbox.rs`, `caldav_tests.rs`, and `calendar_connections.rs`.
- Application: `crates/overlook-app/src/caldav_sync.rs`, `ui/calendar.rs`, `ui/calendar_changes.rs`, and `ui/calendar_setup.rs`.

Pending flags derive from outbox mappings; there is no `events.pending` SQL column. Legacy pending flags are ORed, not overwritten. Preflight-only network errors leave unsubmitted changes editable; ambiguous mutations remain frozen until reconciled. Later recurrence exceptions use the selected occurrence's dates. Changed recurring schedules and unsupported RANGE exceptions fail safely. CalDAV edits do not add a synthetic self-attendee. Password-only repair preserves connection identity and queued changes. Rejected changes have Retry and Discard local changes actions.

## POP3 failure resolved

`pop3::tests::send_goes_over_smtp_and_keeps_a_local_sent_copy` raced the scripted SMTP server. The server sends the QUIT reply before its worker returns and records the transcript. The client could return successfully before `seen.len()` was inspected.

The fake server now exposes `wait_for_connections`, using a condition variable with a bounded deadline. POP3, corresponding IMAP assertions, and the application sent-copy test wait for transcript publication. Production SMTP behavior was not changed. The complete workspace suite passes with the fix.

## Final automated results

Checks were run against the final source tree, including the native-discovered keyboard/focus fixes and last-download timestamp regression:

| Check | Result |
|---|---|
| `cargo test --workspace --offline` | **346 passed, 28 ignored, 0 failed**. App 67, core 87, browser-document 16, message-formatting 12, provider 123, store 41. Ignored tests were not part of this result. |
| `cargo clippy --workspace --all-targets --offline` | Passed with existing warnings; no diagnostics in the new CalDAV/connection modules. |
| Release build | Passed; that binary was used for the final native verification. |
| `rustfmt --edition 2021 --check` on the new CalDAV/connection Rust modules | Passed. |
| `cargo fmt --all -- --check` | Still fails on widespread repository formatting differences, starting in unchanged `overlook-app/src/assets.rs`. Existing compact files were not reformatted wholesale. |
| `git diff --check` | Passed. |

Tests cover conditional writes and conflict races, lost acknowledgments, parser preservation, later recurring occurrences, store revision guards, pending snapshot protection, and child-process kill/reopen durability. Application coordinator tests use the real store and resource editor with injected network outcomes.

Final local logs (temporary, not repository artifacts):

- `/tmp/overlook-caldav-resume-final-tests.log`
- `/tmp/overlook-caldav-resume-final-clippy.log`
- `/tmp/overlook-caldav-resume-release-final.log`
- `/tmp/overlook-caldav-resume-new-fmt.log`
- `/tmp/overlook-caldav-resume-fmt.log`

## Native verification completed

Observed with the release binary on Linux/Wayland in disposable profiles, using synthetic accounts visibly marked Demo and no credentials:

- Timed multi-day creation, Ctrl+S save, repeated title editing, and reopening the editor.
- Deleting an unsent creation clears both the local event and outbox row.
- All-day creation survives a forced process kill and restart with its Pending outbox entry intact.
- Deleting a cached provider event removes its projection and retains a durable deletion intent.
- Light/Dark conflict panels are reachable with Tab/Enter. Keep my changes preserves the local title and the provider's unrelated location edit, with the new base ETag. Use provider version clears the local operation and retains the provider resource for the next display refresh. The database was inspected to confirm each result.
- Settings opens immediately after conflict resolution. Keyboard navigation reaches Accounts and its calendar controls. The final status reads “Calendar settings saved. Sync to download events.” after coverage invalidation, instead of an epoch date.

Native checks found and fixed end-date input shortcut interception, Enter interception on focused buttons, and focus loss when a conflict action removes its own button. Up/Down now selects visible-range events while the calendar itself has focus; Enter edits and Delete removes the selection. Conflict previews are prepared during model refresh instead of parsing during render. Last-download queries ignore the zero timestamp used only to retain the saved timezone after coverage invalidation; a store regression assertion covers this.

Every owned native test process and its descendants were closed and verified exited. The final app PID was 3415424. Final Cargo commands also completed. The user's normal profile, credentials, and desktop configuration were not modified.

The existing desktop transparency lets background text appear in window-only screenshots. These screenshots remain under `/tmp` and are not published or checked into the project. AT-SPI enumeration did not expose the test process; accessible-name and screen-reader validation did not pass and must not be claimed. IME composition, alternative scales, narrow windows, and sleep/wake remain unverified.

## Next qualification work

New-account integration, collection-token synchronization, default-calendar selection and independent reference-server qualification are implemented and tested; see the current completion record. Remaining qualification is a native visual/accessibility pass when native automation becomes available, plus broader hosted observations through normal use. The user explicitly declined further guided test scripts. Use the shared headless commands, loopback HTTPS service and `scripts/qualify-caldav.py` for further development. Meeting delivery/RSVP remains a separate milestone.

For another local test, read `/home/jason/.codex/RTK.md` and the GPUI skill. The ignored `caldav::tests::export_caldav_native_fixture` test creates a new disposable directory with `OVERLOOK_CALDAV_FIXTURE_DIR=/tmp/overlook-caldav-native-...`, `OVERLOOK_CALDAV_WRITE_FIXTURE=1`, and optionally `OVERLOOK_CALDAV_CONFLICT_FIXTURE=1`. Existing profiles now contain the results of verification and should not be treated as pristine fixtures.

`/tmp/overlook-caldav-native-driver.py` guards the exact owned release process, active window, and workspace around every input. It supports launch, focus, key, type, screenshot, tree, stop, and crash. Stop the app before replacing the binary. Close all owned descendants after testing. These temporary helpers and logs may be removed by normal `/tmp` cleanup; the repository's exporter and tests are the durable source of fixtures.
