# Fastmail calendar writes and offline reconciliation

Implementation record, 22 October 2026. This third increment extends the [read-sync implementation](fastmail-calendar-read-sync.md) with personal-event creation, editing, deletion, offline persistence, and explicit conflict resolution. Fastmail is the first target; the transport and outbox are CalDAV code reusable by other qualified services.

## User workflow

Connect calendars under Settings → Accounts using an app password with calendar access. A collection becomes editable when the provider reports create, update, and delete privileges. New event, the event editor, delete, and drag-to-move use the CalDAV outbox. Ctrl+S saves the open event editor. With the calendar itself focused, Up/Down selects a visible-range event, Enter edits it, and Delete removes it. These shortcuts yield to focused inputs and buttons. Timed, all-day, and multi-day appointments are supported; all-day end dates in the editor are inclusive and become exclusive DTEND dates in iCalendar.

Saving commits the local event and queued resource change in one SQLite transaction. The view immediately reflects edits and deletions, with pending indicators and a Review changes action. Subsequent offline edits coalesce until an upload attempt may have reached the provider. Creating then deleting an event before dispatch cancels the local creation without contacting the service. Mail authentication and its operation queue remain independent.

Review changes shows the local title, time, location and description, a bounded preview of the provider resource, and the reason for a conflict. **Keep my changes** reapplies the fields the user edited onto the captured provider version and retries conditionally. **Use provider version** discards the local change and downloads the provider's current projection on the next successful sync. If the provider deleted the resource, **Recreate my version** restores the saved resource; for repeating events that means the complete saved series, not just one occurrence. Changes made at the provider after the user's choice cause another conflict.

A failed preflight while offline leaves an unsubmitted change editable. Once a mutation might have reached the service, its revision is frozen while Overlook checks the result. This may temporarily prevent further edits to that resource; it avoids clearing newer edits with an older acknowledgment. Conflicts and rejected changes are retained across restarts. Rejected requests have Retry and Discard local changes actions; discarding requires a later sync to restore any provider copy.

Reconnect/disconnect is blocked while the connection has queued changes. To repair a separate calendar password without discarding them, enter the new password and reconnect with the same server, username, and credential choice. This updates only the existing keyring entry and retries synchronization. Account removal still intentionally removes that account's local data, including unsent changes.

## Persistence and synchronization

- V15 adds `caldav_outbox`. Each calendar resource has at most one row, containing its original strong ETag/body, desired body or deletion marker, ordered local edits, revision, connection identity/generation, saved time zone, state, and any conflicting provider body/ETag. Credentials never enter the outbox.
- Local projection changes and the queued intent commit atomically. Pending display state is derived from the outbox's resource mappings. Pending tombstones prevent downloaded snapshots from resurrecting locally deleted events; downloads also preserve edited resources and disappeared collections with outstanding changes.
- A worker persists Checking before network reconciliation. Revision and connection guards reject stale edits, results from superseded configuration, and late completion after account removal. A normal preflight failure can return an unsubmitted change to Pending; an ambiguous earlier upload stays Checking.
- Every attempt GETs the exact stable resource URL. Already-matching content or an already-absent deletion acknowledges the previous attempt. Otherwise a changed/missing base becomes a conflict. Creates use a stable UID/URL and `If-None-Match: *`; updates/deletes use `If-Match` with a strong ETag. HTTP 409/412 captures the provider version for review.
- A network failure after mutation retains the exact attempted body. Restart checks the same URL before any retry. Comparison conservatively normalizes folding/line endings and component/property order, retaining all properties; provider rewrites of metadata may therefore require manual review after a lost acknowledgment.
- Acknowledgments clear only the expected revision and invalidate downloaded coverage. A following snapshot supplies canonical provider projections. The saved display time zone remains available if that download fails. Choosing the provider version can leave its event temporarily absent while offline until the refreshed projection downloads.
- Each pass handles up to 100 actionable resources; conflicts do not starve later queued changes. Authentication/network/rate/server failures back off independently of mail. Downloads and writes keep HTTPS, verified certificates, bounded content, same-origin credentials and lifecycle guards. Mutations never follow redirects.

The conditional write rules follow [CalDAV RFC 4791 §5.3.2](https://www.rfc-editor.org/rfc/rfc4791.html#section-5.3.2) and [HTTP RFC 9110 §13.1.1](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.1).

## iCalendar editing and deliberate limits

The bounded editor retains unknown properties, nested alarms, timezone definitions, and unrelated exceptions. It escapes TEXT values, folds at 75 UTF-8 octets, and updates timestamps/sequence. It does not rebuild resources from the flattened event display model. Raw resources remain capped at 2 MiB, with nesting/content-line limits; no more than 128 offline edits are retained for one undispatched resource.

This increment supports personal appointments. Resources containing ATTENDEE properties or another person's ORGANIZER are rejected for mutation. Invitation delivery, RSVP, Google Meet creation and scheduling are not implemented for CalDAV. Existing meetings continue to display.

One occurrence of a repeating event can be edited or cancelled; deleting an entire series is also supported. The untouched master and other exceptions remain intact. New exceptions retain the selected occurrence's time rather than the master's first date. Series-wide edits, new recurrence-rule creation, and “this and following” splits are not exposed. If the provider changes a recurring schedule during a conflict, the user must accept the provider version, sync, then edit the refreshed occurrence. An unfamiliar custom TZID that cannot safely identify an occurrence fails without uploading. A saved time-zone mismatch also blocks further edits/rebasing until the original zone is restored.

## Validation and remaining qualification

Automated verification covers iCalendar preservation, escaping/folding, date-only boundaries, later recurring occurrences, conditional create/update/delete, remote deletion, preflight conflicts, 412 races, weak ETags, redirects/origin guards, lost mutation responses, store revision guards, pending snapshot protection, create→delete cancellation, restart, and forced process-kill persistence. The application coordinator is tested against injected network outcomes with the real store and resource editor.

Live Fastmail and a separate reference CalDAV implementation remain required before declaring provider compatibility. Synthetic protocol servers and a disposable native profile do not validate a hosted account's permissions, actual app-password/keyring behavior, provider canonicalization or meeting delivery. Incremental collection tokens remain later work. Optional new Fastmail account integration and stateful HTTPS workflow tests were added on 23 October; see [onboarding and autonomous verification](fastmail-onboarding-and-workflows.md).

### Verification follow-up — 22 October 2026

The POP3 failure from the paused run was a race in the scripted SMTP server. Its worker sends the QUIT reply before returning and appending the transcript; the client could finish before an assertion inspected that transcript. The fixture now has a bounded condition-variable wait for recorded connections. POP3, IMAP, and the application sent-copy assertions use that synchronization. Production SMTP behavior was not changed.

Native verification used the release binary on Linux/Wayland at the current desktop scale, in a 1320 × 920 logical-pixel window. All accounts and resources were synthetic and visibly marked Demo; no real credentials were entered. Observed:

- Create a timed appointment spanning two dates, save with Ctrl+S, edit its title repeatedly, and reopen the editor. The queued revision advances and the event stays personal (no accidental self-attendee).
- Delete that unsent creation: the event and queued creation disappear without a server write.
- Save an all-day event, force-kill the owned app and its children, reopen the same profile, and find both the event and its Pending outbox entry intact.
- Delete an existing cached provider event: its view disappears and a durable resource deletion remains queued.
- Open the conflict panel using Tab/Enter in Light and Dark. Keep my changes produces a queued resource containing the local title, the provider's unrelated location update, and the new provider ETag. Use provider version removes the local outbox row and retains the provider resource, awaiting a fresh display download.
- Open Settings immediately after resolving a conflict, reach Accounts with Tab/Enter, and scroll to the calendar setup with Page Down. Calendar permission, credential-choice, password-repair guidance, and visibility controls render in Dark. The earlier read increment separately verified these controls in Light.

Native checks found and fixed shortcut interception in the new end-date field, Enter interception on focused buttons, and focus loss when resolving/closing calendar changes. Up/Down now provides keyboard event selection. Conflict previews are prepared on model refresh rather than parsed during render. The zero-timestamp coverage marker used after writes is excluded from last-download status, avoiding an epoch date in Settings.

The final release build was reopened to verify that Settings now reads “Calendar settings saved. Sync to download events.” after coverage invalidation. The complete final workspace suite passed **346 tests**, with **28 ignored** and **0 failed**. Release build, Clippy (with existing warnings), new-module formatting, and whitespace checks passed. Repository-wide formatting still fails on existing differences. All owned native app processes and their descendants were closed after verification.

The system's existing window transparency allows background desktop text to appear in window-only screenshots. Screenshots remain local under `/tmp`; they are not published as project assets. AT-SPI enumeration still did not expose the test application, so screen-reader behavior remains unverified. IME composition, different display scales, narrow-window layouts, sleep/wake behavior, live Fastmail/reference-server interoperability, and real credential rotation remain unverified.

Final check results and cleanup are recorded in the [handoff](fastmail-writes-handoff.md). These are implementation and local-fixture verification results, not hosted-provider release qualification.
