# Fastmail calendar read synchronization

Historical read-path implementation record, 22 October 2026. The subsequent [write-sync increment](fastmail-calendar-write-sync.md) enables personal-event edits and supersedes the read-only restrictions below. This is the second increment of the [calendar provider roadmap](calendar-provider-roadmap.md). It enables **provider-to-Overlook synchronization and offline viewing**. Event creation, edits, deletion, and invitation responses from Overlook remain disabled for CalDAV calendars until conditional writes and durable offline operations are implemented.

## Connecting an existing account

1. Open **Settings → Accounts** and select the Fastmail IMAP account. The calendar empty state also opens these settings.
2. In **Calendars**, check the suggested `https://caldav.fastmail.com/` endpoint and full Fastmail login username. A custom mail domain is supported by recognizing the configured incoming server.
3. Enter a Fastmail app password with calendar permission. Alternatively, explicitly select **Use this account’s saved mail password** if that password includes calendar access. Mail authentication alone does not establish calendar permission.
4. Choose **Connect calendars**. Overlook validates discovery and downloads the initial calendar range before committing the new connection. Passwords are saved only in the system keyring.
5. Use **Show calendars** to choose what appears. Calendar views also have visibility controls. **Sync now** refreshes the calendar independently of mail.

Settings can be reached with Ctrl+comma, then Tab/Enter to select Accounts. Page Up/Page Down scrolls the settings form. Closing settings restores keyboard focus to the active module.

Blank passwords on reconnect retain only a separately stored calendar password for the same endpoint and username. Switching from a shared mail password to a separate password requires entering one. A new connection owns a new keyring identity; a failed or superseded setup cannot replace the previous connection’s secret. The old owned secret is removed after successful replacement. Cleanup failures are reported.

Disconnect removes this connection’s local calendars, cache, and owned keyring secret. It does not remove remote events or the incoming-mail password. Deleting a saved password does not revoke the app password at Fastmail.

## Reading and offline behavior

- Day, Week, Month, and Agenda read the same local event store used by existing calendars.
- The initial range is 60 days back and 366 days forward. Navigating outside it requests the visible range with a small margin. Previously downloaded ranges remain cached.
- Calendar polling has a separate in-flight guard and retry deadline. Missing mail credentials, mail probe failure, and mail backoff do not block it. Calendar failure does not change mail status.
- Cached events remain visible after restart or a connection failure. An uncached range has explicit wording; it is not presented as proof of an empty calendar.
- Date-only and floating times use the system time zone. Coverage records its zone; projections from another zone remain on disk but are withheld from views and reminders until refreshed.
- Remote changes replace the downloaded window atomically. Events outside that window are retained. A changed resource ETag invalidates its older projections and their coverage, preventing a changed recurring master from leaving falsely current occurrences elsewhere.

## Protocol and parser decisions

The client issues a complete `calendar-query` for original resources and a second query using `calendar-data/expand` for the same padded time range. Resource URLs and ETags must agree between reports. A race, missing required property, HTTP error, oversized response, malformed data, or incomplete collection inventory rejects the whole snapshot. It does not erase the prior cache.

Recurrence expansion is delegated to the CalDAV server for this read-only increment. The standard requires expanded occurrences to omit recurrence rules and resolves referenced time zones to UTC. The client rejects unexpanded recurrence rather than displaying an incomplete series. This avoids introducing a partial local recurrence implementation. [RFC 4791, section 9.6.5](https://www.rfc-editor.org/rfc/rfc4791.html#section-9.6.5)

`ical` 0.11.0 parses folded lines, properties, parameters, events, and alarms. Its packaged `LICENSE` is Apache-2.0; registry metadata reports the license file rather than an SPDX expression. Structural bounds and semantic validation surround the parser. `jiff` supplies date, time-zone, and duration handling. [ical parser documentation](https://docs.rs/ical/0.11.0/ical/)

Original calendar text is retained as returned by the XML parser, including unknown properties, recurrence definitions, and custom time-zone components. XML line-ending normalization can change CRLF to LF. Display projections are never serialized back to the provider. Occurrence identities include the calendar and recurrence identity, so moving an exception does not create a duplicate occurrence.

Supported display fields include title, description, location, organizer, attendees and response status, dates/times, durations, and supported relative display reminders. Email/procedure alarms and attached resources are not executed. Unsupported or malformed temporal data fails the snapshot with an actionable message. There is no local recurrence expansion or live invitation delivery in this increment.

## Persistence and lifecycle

Schema V14 adds CalDAV collection ownership, original resources with ETags, occurrence-to-resource mapping, and completed range coverage. Snapshot application updates calendars, events, resource data, coverage, and sync status in one transaction. Existing visibility preferences survive refresh; reconnect preserves them by remote collection URL.

Connection identity and generation guard every background commit and status update. Setup also reserves an operation identity, invalidated by a newer setup, closing/switching its dialog, or account removal. Late work cannot recreate a disconnected connection. Store mutation APIs reject edits, deletes, and RSVP on read-only CalDAV collections, including paths outside the visible UI.

The read path is a dedicated CalDAV snapshot service alongside the existing mail/Google provider assembly. It reuses the core calendar/event model and calendar UI. It does not yet implement the mutation-oriented `CalendarProvider` trait. Google’s adapter, OAuth scopes, and submitted CASA evidence remain unchanged.

Transport protections from the first increment remain: HTTPS, certificate/hostname checks, explicit endpoint authorization, same-origin redirects, credential-safe errors, no external entities, and bounded responses. REPORT responses are limited to 8 MiB, a snapshot to 32 MiB, and event/resource/component counts are bounded. Cross-origin shards, custom CA configuration, and DNS/SRV discovery are not enabled. The 23 October discovery correction adds a bounded same-origin `/.well-known/caldav` fallback when a bare server root returns HTTP 404 or 405; explicit resource URLs and later principal/home failures do not trigger that fallback.

### Fastmail root discovery correction — 23 October 2026

The first real-account setup attempt reported HTTP 404. Unauthenticated read-only PROPFIND probes reproduced a 404 at `https://caldav.fastmail.com/`; `/.well-known/caldav` returned a 301 redirect to `https://caldav.fastmail.com/dav/calendars`, while `/dav/` requested authentication. No credentials were read or used for these probes. The documented Fastmail root is a discovery starting point, not necessarily a DAV resource. [Fastmail settings](https://www.fastmail.help/hc/en-us/articles/1500000278342-Server-names-and-ports), [service discovery](https://www.rfc-editor.org/rfc/rfc6764.html#section-5).

Overlook now follows the well-known service hint after an initial bare-root 404/405. It retains the original request deadline, redirect limits, HTTPS validation, and same-origin credential restriction. It does not substitute another calendar for an invalid explicit URL, or restart discovery after a principal/home failure. Existing builds can use `https://caldav.fastmail.com/.well-known/caldav` explicitly as a setup workaround. Successful authenticated setup and subsequent synchronization still need confirmation from the user's live test.

## Validation and remaining qualification

Automated tests cover parsing, date-only DST transitions, floating/zoned times, moved and cancelled occurrences, malformed/unexpanded data, REPORT requests, ETag races, credential boundaries, atomic reconciliation, range-scoped deletion, restart persistence, visibility, disconnect/reconnect, cancelled setup, write rejection, and independent mail/calendar failures.

Final test and native verification results are recorded in the provider roadmap. Native checks use an isolated demo profile with synthetic events and no real calendar credentials. The ignored `export_caldav_native_fixture` test creates that profile only in an explicitly supplied new `/tmp/overlook-caldav-native-*` directory.

Live Fastmail and Nextcloud qualification remains necessary before declaring provider compatibility verified. Incremental `sync-collection`, conditional writes, offline editing/conflicts, meetings, and additional provider presets remain follow-up work. This increment uses complete bounded range reports on refresh.
