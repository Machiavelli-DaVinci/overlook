# Fastmail live calendar qualification

Started 23 October 2026, America/Phoenix. This record separates user observations, independently verified results, and checks still pending. It supplements the synthetic/native results in [the write-sync record](fastmail-calendar-write-sync.md).

## Scope

Use the user's connected Fastmail account, restricted to the designated `test event` and clearly named `Overlook sync test` events in its existing personal calendar. No invitations or guests. The user operates Overlook's native UI; the agent inspects the same test events in Fastmail's web UI and checks only their local SQLite records read-only. No credential export or real-calendar cleanup is part of this test.

## Results

| Scenario | Status / evidence |
|---|---|
| Connect and initial download | User confirmed connection and seeing the event after using `/.well-known/caldav`. Independently confirmed the cached calendar is writable and the same event exists in Fastmail's web UI. |
| Baseline event | `test event`, October 24, 2026, 09:00–10:00 America/Phoenix; location `home`; description `this is a test`. These are test values, not real appointment data. |
| Overlook → Fastmail edit | Passed. User saved `Overlook sync test — desktop edit` / `Overlook desk`; the agent observed both in Fastmail, a changed cached provider ETag, and no pending test operation. |
| Fastmail → Overlook edit | Passed. Agent saved `Overlook sync test — web edit` / `Fastmail web desk` / `Edited in Fastmail to verify download.` in the web UI. User confirmed all three fields appeared in Overlook after Sync now. |
| Create and delete | Two helper events created and verified in Fastmail: `Overlook sync test — provider choice` on October 24 at 10:00, and `Overlook sync test — offline delete` at 12:00. Overlook creation/deletion pending. |
| Offline edit/create/delete and restart persistence | User reported that all changes survived restart and Overlook was closed. Independent verification is inconclusive: the default profile still showed the previous three events and no pending test operations. The offline-created event was absent and the deletion target remained present. User requested moving on without further profile investigation. Do not count this as an independently verified pass or a confirmed application failure. |
| Conflict: keep local edits while retaining unrelated provider edits | Pending. Fastmail-side candidate saved: 09:00 event title `Overlook sync test — competing web title`, location `Fastmail location to preserve`. |
| Conflict: use provider version | Pending. Fastmail-side candidate saved: 10:00 event title `Overlook sync test — provider wins`, location `Fastmail chosen location`. |

## Execution notes

The original discovery attempt returned HTTP 404 at Fastmail's root URL. The provider implementation now tries the same-origin well-known endpoint for an initial bare-root 404/405; see [discovery correction](fastmail-calendar-read-sync.md#fastmail-root-discovery-correction--23-october-2026). That fix passed 350 workspace tests (28 ignored). The live connection succeeded using the explicit well-known URL; the automatic fallback itself has fixture coverage and still needs a separate authenticated confirmation if advertised as live-tested.

Offline and conflict scenarios must prevent Overlook from uploading while the web client changes the same resource. A visible offline state alone is insufficient evidence: verify a durable pending operation and unchanged provider event, then restore connectivity and inspect the result. Do not mark a scenario passed from code coverage alone.

After the user requested moving on, both competing web edits were saved and verified in Fastmail. They do not establish that a conflict occurred. The user subsequently ended the guided test workflow and asked the agent to continue building with its own automated tests. Do not ask the user to resume the script or perform more fixture testing; acceptance testing will happen through normal use at the end. The live offline/conflict scenarios remain inconclusive, and the prepared test events were left in place.

## Automated continuation — 23 October 2026

The [onboarding and workflow increment](fastmail-onboarding-and-workflows.md) adds a stateful local HTTPS calendar server and tests the actual editor commands, CalDAV transport/parser, sync coordinator and SQLite together. Offline create/edit/delete, database reopen, both conflict choices, permission recovery, and lost PUT/DELETE responses are now covered without the user's account. These are successful simulated workflows, not additional hosted Fastmail qualification.
