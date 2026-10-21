# Overlook Google Data Flow Review

Reviewed 2026-10-19 against application version 1.5.1, commit `863bad52c243c4fe2436aed42eb61a35690adbca`.

The reviewed application implements Google sign-in, mail, calendar, and contacts on the user's computer. Its Google provider calls Google's HTTPS endpoints without a Last Refuge application server, token broker, or synchronization service. OAuth tokens use the operating system keyring; mailbox data uses local SQLite and files. No application telemetry, remote crash reporting, or log-upload integration was found in the reviewed runtime paths and dependency configuration.

This supports asking Google to reconsider whether the server-related CASA assessment requirement applies. It does not support claiming that the application never contacts other services, never transmits information derived from email, or encrypts all local data.

## Verified application paths

| Function | Implementation and evidence |
| --- | --- |
| Sign-in | [google_signin.rs](../crates/overlook-app/src/google_signin.rs), lines 57–68 and 111–119: the desktop process binds `127.0.0.1` on an ephemeral port, creates PKCE/state, receives the browser callback, exchanges the code, and retrieves user information. [ui/mod.rs](../crates/overlook-app/src/ui/mod.rs), lines 893–918, opens the system browser. This loopback listener is on the user's machine, not a hosted callback service. |
| Token exchange and refresh | [oauth.rs](../crates/overlook-provider/src/gmail/oauth.rs), lines 113–165 and 190–191: requests go to Google's token and user-info endpoints. No developer token-exchange endpoint is configured. |
| Credential persistence | [credentials.rs](../crates/overlook-app/src/credentials.rs), lines 71–88 and 114–123, uses `oo7::Keyring` for token serialization and retrieval. [providers.rs](../crates/overlook-app/src/providers.rs), lines 38–49, persists refreshed tokens through the same keyring. The application has no plaintext token-file fallback. |
| Gmail | [api.rs](../crates/overlook-provider/src/gmail/api.rs), lines 8–9, 101–177, and 228–246: direct authenticated requests to Gmail and Google's batch endpoint. [provider.rs](../crates/overlook-provider/src/gmail/provider.rs), lines 530–555, downloads attachments and submits outgoing messages through Gmail. |
| Calendar and contacts | [calendar.rs](../crates/overlook-provider/src/gmail/calendar.rs) uses `https://www.googleapis.com/calendar/v3`; [provider.rs](../crates/overlook-provider/src/gmail/provider.rs), lines 258–347 and 609, uses `https://people.googleapis.com/v1`. Call sites construct URLs from these Google endpoints. |
| Mailbox and offline state | [main.rs](../crates/overlook-app/src/main.rs), lines 43–49 and 75–82, opens `overlook.sqlite` in the local data directory. [sync.rs](../crates/overlook-app/src/sync.rs) writes downloaded messages, contacts, events, checkpoints, and queued changes to that store. Queued actions replay from the desktop to the configured provider. |
| Attachments and backup | [ui/mod.rs](../crates/overlook-app/src/ui/mod.rs), lines 1343–1375, writes downloaded attachments to the local profile. [backup.rs](../crates/overlook-app/src/backup.rs), lines 31–38, copies SQLite, raw mail, and attachments to the user's selected filesystem directory. No cloud backup service is implemented. |

## Other data flows that must be disclosed accurately

### Remote email content

Remote email resources are blocked by default: [prefs.rs](../crates/overlook-core/src/prefs.rs), lines 34–39 and 88. A user can enable them for a message or through preferences: [reader.rs](../crates/overlook-app/src/ui/reader.rs), lines 488 and 609–635.

When enabled, the application fetches images, fonts, and stylesheets directly from their resource URLs. The renderer intercepts requests, permits only eligible GET requests, and uses a separate HTTP agent: [browser_engine.rs](../crates/overlook-app/src/browser_engine.rs), lines 796–806, 915–951, and 1290–1341. It does not attach Gmail authorization, cookies, or referrer headers. Private network destinations and unsafe redirects are blocked.

These requests still reveal the client's IP address and the requested URL to the destination. A URL can contain a sender's tracking identifier or other information. Consequently, a blanket statement that no email-derived information ever reaches a third-party server would be inaccurate. Disclose this user-enabled flow and ask Google how it affects assessment applicability.

The embedded Chromium renderer runs locally with a private temporary profile and a local pipe. Background networking, crash reporting, synchronization, and extensions are disabled in its launch configuration; Chromium requests are intercepted rather than continued. See [browser_engine.rs](../crates/overlook-app/src/browser_engine.rs), lines 380–423 and 786–794. This is not a remote rendering service.

### Updates and account discovery

[updater.rs](../crates/overlook-app/src/updater.rs), lines 143–172 and 214–239, queries GitHub releases and downloads release assets/checksums. The update client sends the application version in its user agent, not Google tokens or mailbox content. Checks are enabled by default.

For IMAP/POP setup, [autoconfig.rs](../crates/overlook-provider/src/autoconfig.rs), lines 107–117, can request Thunderbird's provider database using the entered address's domain. This is separate from Google API synchronization and does not send the account password or full email address in that request.

IMAP, POP3, and SMTP connect to the account's configured mail servers from the desktop. User-directed forwarding or sending can transmit mail through those providers; these operations are not developer-hosted synchronization.

### Local data and user exports

Mailbox SQLite, attachments, backups, and print/source exports are ordinary files. The application does not apply database or file encryption to them. Operating system disk encryption, filesystem permissions, and the selected keyring's protections are separate. Do not claim that all locally stored data is encrypted by Overlook.

Logging is configured as local formatted output in [main.rs](../crates/overlook-app/src/main.rs), lines 103–105. Some messages contain account addresses, calendar names, or mail subjects; for example, [sync.rs](../crates/overlook-app/src/sync.rs), line 541, logs a sent subject. No log uploader was found. Do not claim that logs contain no user data.

Notifications go to the desktop notification service. Saving, printing, opening attachments, clicking links, and backing up can hand data to user-selected files or external applications. Those applications and user-configured network filesystems may have their own data handling. The print/export paths are in [ui/mod.rs](../crates/overlook-app/src/ui/mod.rs), lines 1258–1338.

Contact photo URLs returned by Google are passed to GPUI's image widget in [contacts.rs](../crates/overlook-app/src/ui/contacts.rs), line 407, and [reader.rs](../crates/overlook-app/src/ui/reader.rs), line 732. In the pinned Linux GPUI initialization, the default HTTP client is `NullHttpClient`, and the application/component initialization inspected does not replace it. These UI references therefore do not establish successful network photo downloads in this build. Reassess this path if an HTTP client is installed later.

## Verification results and scope

- The offline build of all workspace tests succeeded.
- All 273 regular workspace tests passed; 25 explicitly ignored tests were not part of that run. The initial sandboxed attempt could not bind local test servers; the completed run had local socket access.
- The separate `chromium_controlled_redirect_consent` test passed with real sandboxed headless Chromium and a synthetic local server. It verifies zero resource requests before consent, permitted image loading after consent, blocking a redirect to a private address, and omission of cookie, authorization, proxy-authorization, and referrer headers.
- The regular `resource_fetch_is_anonymous_and_gated` test also passed, covering redirects and request headers.
- Clippy completed with existing warnings. Formatting checks reported existing differences. No application source was changed in this review.

This is a source and controlled-test review of this checkout, not a certification, a traffic capture of a real Google account, or verification of an installed/released binary. The Google Cloud project, its other OAuth clients, and the submitted questionnaire were not inspected. The OAuth configuration parser accepts both `installed` and `web` JSON sections, so the loopback implementation alone does not verify the client type registered in Cloud Console.

Google HTTP agents use the library's default proxy handling, which can honor operating-system process proxy settings. There is no application-specified Last Refuge proxy. User-installed proxies, external applications, and OS backup/telemetry configuration are outside the source-level finding.

## Implications for the Google reply

Google's [restricted-scope verification documentation](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification) ties the third-party assessment requirement to restricted data accessed from or through a third-party server. The reviewed provider architecture supports requesting an applicability review; the optional remote-content flow should be included rather than omitted. Only Google can confirm the classification for the submitted application.

The existing [verification notes](google-verification.md) need care when reused: their unconditional statement that the restricted scopes require a yearly assessment does not address the server condition, and their “only in a local database” wording omits files, keyring storage, and local logs. The `oauth.rs` scope comment also incorrectly calls `gmail.send` restricted; Google's [Gmail scope table](https://developers.google.com/workspace/gmail/api/auth/scopes) classifies it as sensitive. These comments are not evidence of the assessment requirement.

A factually supported description is:

> Overlook is an installed Linux desktop application. Its Google authorization, token exchange, and Gmail, Calendar, and People API calls execute on the user's computer against Google's endpoints. OAuth tokens are stored through the operating system keyring; downloaded data and offline state are stored in local SQLite and files. The reviewed application has no developer-operated backend that receives Google tokens or mailbox data. Remote email images, fonts, and stylesheets are blocked by default; when enabled by the user, those resources are fetched directly from their hosts without Google authorization, cookies, or referrer headers. Please confirm whether AL1 is required for this architecture and identify the data flow that brings it into scope.

No message was sent to Google as part of this review.
