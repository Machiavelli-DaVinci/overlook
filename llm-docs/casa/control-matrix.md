# Overlook CASA control evidence

Scope: native Linux desktop app, v1.5.5 candidate based on b80893a5927d3d6d65592f845409b7b5984b351d. The evidence includes unreleased local security fixes. This is a developer evidence package; TAC determines control acceptance. No real user mailbox, tokens, passwords, or production credential files were used.

The PNGs are rendered source/test documents, not screenshots of a live Google login. Source hashes and line references identify the reviewed material.

## 1.1.1 Authentication is resistant to brute force attacks

Applicable: Yes

Google Account authentication is performed in the system browser. Overlook is a native Linux public OAuth client and does not operate an authentication server or collect Google passwords. IMAP/POP3/SMTP authentication is performed by each user-selected mail provider. The local OS user controls access to the installed application and its profile. Brute-force prevention and MFA for Google accounts are provided by Google; there is no proprietary Overlook password login to brute-force. The local callback additionally requires an unpredictable state and PKCE.

Authorization/PKCE and forged-callback regression tests passed. Provider-side brute-force/MFA behavior was not independently penetration-tested.

- `crates/overlook-app/src/google_signin.rs:57`
- `crates/overlook-provider/src/gmail/oauth.rs:234`

[Evidence image](evidence/casa-1.1.1.png)

## 1.1.2 System generated initial passwords or activation codes shall be securely randomly generated and expire after a short period.

Applicable: No

Google Account authentication is performed in the system browser. Overlook is a native Linux public OAuth client and does not operate an authentication server or collect Google passwords. IMAP/POP3/SMTP authentication is performed by each user-selected mail provider. The local OS user controls access to the installed application and its profile. Overlook does not issue initial passwords, activation codes, or password-reset codes. This control is not applicable to an Overlook-managed password lifecycle.

Source review; see the referenced code excerpts.

- `crates/overlook-app/src/google_signin.rs:57`
- `crates/overlook-provider/src/gmail/oauth.rs:136`

[Evidence image](evidence/casa-1.1.2.png)

## 1.1.3 Passwords shall be stored in a form that is resistant to offline attacks.

Applicable: Yes

Google passwords are never received. OAuth access/refresh tokens and optional IMAP/POP3/SMTP passwords are stored through oo7 in the OS Secret Service or encrypted Secret Portal-backed keyring. A mail client must retrieve server passwords to authenticate, so one-way password-verifier hashing is not the applicable storage model. Keyring failures are surfaced, with no application plaintext fallback. Encryption strength and unlock protection depend on the selected OS keyring. The SQLite mail cache is not application-encrypted.

Source review; see the referenced code excerpts.

- `crates/overlook-app/src/credentials.rs:71`
- `crates/overlook-app/src/credentials.rs:114`
- `crates/overlook-app/src/providers.rs:80`

[Evidence image](evidence/casa-1.1.3.png)

## 1.2.1 Default credentials shall not present on publicly exposed interfaces.

Applicable: No

Overlook exposes no public login or administrative interface and ships no default mail credentials. The fictional Sample provider is local demonstration data, not an account on a server. The only HTTP listener is a temporary IPv4 loopback OAuth callback requiring per-attempt state and PKCE; it has no password login.

Source review; see the referenced code excerpts.

- `crates/overlook-app/src/google_signin.rs:57`
- `crates/overlook-app/src/main.rs:62`

[Evidence image](evidence/casa-1.2.1.png)

## 1.3.1 Out of band verifier shall expire in a reasonable timeframe.

Applicable: No

Google Account authentication is performed in the system browser. Overlook is a native Linux public OAuth client and does not operate an authentication server or collect Google passwords. IMAP/POP3/SMTP authentication is performed by each user-selected mail provider. The local OS user controls access to the installed application and its profile. Overlook has no SMS/email/telephone OTP or proprietary out-of-band verifier. OAuth uses the system browser and loopback callback, not the deprecated manual OOB flow. As supplemental native-flow protections, each attempt uses fresh state and PKCE, has a 600-second timeout, and consumes one validated callback.

PKCE RFC 7636 vector, state validation, duplicate-parameter and wrong-host callback tests passed. No proprietary OOB verifier exists to test.

- `crates/overlook-app/src/google_signin.rs:17`
- `crates/overlook-app/src/google_signin.rs:83`
- `crates/overlook-provider/src/gmail/oauth.rs:234`

[Evidence image](evidence/casa-1.3.1.png)

## 1.3.2 Out of band verifier shall only be used once.

Applicable: No

Google Account authentication is performed in the system browser. Overlook is a native Linux public OAuth client and does not operate an authentication server or collect Google passwords. IMAP/POP3/SMTP authentication is performed by each user-selected mail provider. The local OS user controls access to the installed application and its profile. Overlook has no SMS/email/telephone OTP or proprietary out-of-band verifier. OAuth uses the system browser and loopback callback, not the deprecated manual OOB flow. As supplemental native-flow protections, each attempt uses fresh state and PKCE, has a 600-second timeout, and consumes one validated callback.

PKCE RFC 7636 vector, state validation, duplicate-parameter and wrong-host callback tests passed. No proprietary OOB verifier exists to test.

- `crates/overlook-app/src/google_signin.rs:17`
- `crates/overlook-app/src/google_signin.rs:83`
- `crates/overlook-provider/src/gmail/oauth.rs:234`

[Evidence image](evidence/casa-1.3.2.png)

## 1.3.3 Out of band verifier shall be securely random

Applicable: No

Google Account authentication is performed in the system browser. Overlook is a native Linux public OAuth client and does not operate an authentication server or collect Google passwords. IMAP/POP3/SMTP authentication is performed by each user-selected mail provider. The local OS user controls access to the installed application and its profile. Overlook has no SMS/email/telephone OTP or proprietary out-of-band verifier. OAuth uses the system browser and loopback callback, not the deprecated manual OOB flow. As supplemental native-flow protections, each attempt uses fresh state and PKCE, has a 600-second timeout, and consumes one validated callback.

PKCE RFC 7636 vector, state validation, duplicate-parameter and wrong-host callback tests passed. No proprietary OOB verifier exists to test.

- `crates/overlook-app/src/google_signin.rs:17`
- `crates/overlook-app/src/google_signin.rs:83`
- `crates/overlook-provider/src/gmail/oauth.rs:234`

[Evidence image](evidence/casa-1.3.3.png)

## 1.3.4 Out of band verifier shall be resistant to brute force attacks

Applicable: No

Google Account authentication is performed in the system browser. Overlook is a native Linux public OAuth client and does not operate an authentication server or collect Google passwords. IMAP/POP3/SMTP authentication is performed by each user-selected mail provider. The local OS user controls access to the installed application and its profile. Overlook has no SMS/email/telephone OTP or proprietary out-of-band verifier. OAuth uses the system browser and loopback callback, not the deprecated manual OOB flow. As supplemental native-flow protections, each attempt uses fresh state and PKCE, has a 600-second timeout, and consumes one validated callback.

PKCE RFC 7636 vector, state validation, duplicate-parameter and wrong-host callback tests passed. No proprietary OOB verifier exists to test.

- `crates/overlook-app/src/google_signin.rs:17`
- `crates/overlook-app/src/google_signin.rs:83`
- `crates/overlook-provider/src/gmail/oauth.rs:234`

[Evidence image](evidence/casa-1.3.4.png)

## 2.1.1 The application shall not reveal passwords or session tokens in URL parameters. In cases where the application provides an API, the application shall prevent (or give developers an option) to prevent exposing sensitive information like API keys or session tokens within the URL query strings

Applicable: Yes

Google access tokens are sent in Authorization: Bearer headers. Token exchange, refresh, and revocation secrets are form POST bodies over HTTPS, never URL query parameters. The native loopback URL carries a one-time authorization code and state, not an access or refresh token; responses use no-store and no-referrer. Overlook exposes no public API. Attached evidence is native source/regression evidence, not a web DAST scan.

Source review plus OAuth encoding/callback tests and credential logging regression passed; TAC to confirm native-app DAST coverage.

- `crates/overlook-provider/src/gmail/api.rs:155`
- `crates/overlook-provider/src/gmail/oauth.rs:127`
- `crates/overlook-provider/src/gmail/oauth.rs:164`

[Evidence image](evidence/casa-2.1.1.png)

## 2.2.1 Users shall have the ability to logout of the application. Logout or session expiration shall invalidate all stateful session tokens, including refresh tokens.

Applicable: Yes

Overlook native Linux desktop app. Remove account disables the shared Gmail client, drains in-flight work, revokes the Google grant over HTTPS, clears in-memory tokens, then deletes OS-keyring entries and local account data. Failed revocation or keyring deletion reports incomplete removal and retains retry information. On 2026-10-22 the owner completed a live sign-in/removal test: authenticated sync downloaded 93 messages; before removal one OAuth keyring item existed; afterward the Gmail account, its cached messages/sync state and all matching keyring items were absent. The completed guarded cleanup demonstrates the revocation path returned success; no independent revoked-token replay or HTTP trace was captured. Removal requires connectivity and affects the user’s grant across this Google project’s devices/scopes. Provider passwords are removed locally, not changed at their issuer. See live supplement and source/regression evidence.

Live user-driven sign-in/removal and before/after profile/keyring metadata verified. Remote revocation success is inferred from the guarded application path; no independent token replay performed.

[Initial source/test evidence](evidence/casa-2.2.1.png) · [Live verification supplement](evidence/casa-2.2.1-live.png)

## 2.2.2 The application shall provide the option (or acts by default) to terminate all other active sessions, including stateful refresh tokens, after a successful password change (including change via password reset/recovery), and that this is effective across the application, federated login (if present), and any relying parties.

Applicable: No

Overlook does not provide an account-password change/reset service or issue cross-device sessions. Google and the selected mail provider own password changes and their session policies. Updating a saved IMAP password changes the local credential, not the provider password. Rejected Google refresh credentials map to SignInRequired. App disconnect explicitly revokes the Google grant; no claim is made that Overlook controls Google’s other sessions.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/api.rs:113`
- `crates/overlook-app/src/providers.rs:98`

[Evidence image](evidence/casa-2.2.2.png)

## 2.2.3 Non-revocable sateless authentication tokens must expire within 24 hours of being issued

Applicable: No

Overlook does not issue non-revocable stateless session tokens. It consumes Google-issued, revocable OAuth tokens, honors expires_in, and treats access tokens as expired 60 seconds early. The default expiry when omitted is one hour. Refresh tokens are stored in the keyring and explicitly revoked on disconnect. Returned ID tokens are not used to authorize local users or API requests.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/oauth.rs:113`
- `crates/overlook-provider/src/gmail/oauth.rs:58`

[Evidence image](evidence/casa-2.2.3.png)

## 2.3.1 Cookie-based session tokens shall have the 'Secure' attribute set.

Applicable: No

No Overlook cookie-based authentication or web session exists. API requests use bearer headers and the loopback completion page sets no cookies. The user’s system browser may retain Google’s own sign-in cookies; Overlook does not create, read, or administer them. The isolated mail renderer has no authenticated browser session. A cookie-attribute DAST finding against a hosted Overlook application is therefore not applicable.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/api.rs:155`
- `crates/overlook-app/src/google_signin.rs:135`

[Evidence image](evidence/casa-2.3.1.png)

## 2.3.2 Cookie-based session tokens shall have the 'HttpOnly' attribute setCookie-based session tokens shall have the 'HttpOnly' attribute set.

Applicable: No

No Overlook cookie-based authentication or web session exists. API requests use bearer headers and the loopback completion page sets no cookies. The user’s system browser may retain Google’s own sign-in cookies; Overlook does not create, read, or administer them. The isolated mail renderer has no authenticated browser session. A cookie-attribute DAST finding against a hosted Overlook application is therefore not applicable.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/api.rs:155`
- `crates/overlook-app/src/google_signin.rs:135`

[Evidence image](evidence/casa-2.3.2.png)

## 2.3.3 The application shall use session tokens rather than static API secrets and keys, except with legacy implementations.

Applicable: Yes

Google API access uses dynamically issued short-lived OAuth bearer tokens and refresh tokens. The Desktop OAuth client identifier and its non-confidential client-secret field do not independently authorize user data. IMAP/POP3/SMTP support uses the provider’s password/app-password mechanism as a legacy-protocol exception; those secrets are keyring-protected and transmitted only after remote TLS setup.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/oauth.rs:136`
- `crates/overlook-provider/src/gmail/api.rs:104`
- `crates/overlook-provider/src/smtp.rs:96`

[Evidence image](evidence/casa-2.3.3.png)

## 2.3.4 Stateless session tokens shall use digital signatures, encryption, and other countermeasures to protect against tampering, enveloping, replay, null cipher, and key substitution attacks.

Applicable: No

Overlook does not mint or accept client-supplied JWT/stateless session tokens as a local authorization mechanism. Google opaque bearer tokens are sent only to Google for provider-side validation. The application does not decode or trust a JWT payload or select signature algorithms. Unsupported/invalid credentials fail through the API error path. No app-managed stateless-token signer or verifier exists for web DAST.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/api.rs:206`

[Evidence image](evidence/casa-2.3.4.png)

## 2.4.1 Verify the application ensures a full, valid login session or requires re-authentication or secondary verification before allowing any sensitive transactions or account modifications.

Applicable: Yes

Mail, calendar, and contact operations require an authenticated provider. Google calls attach a valid token, refresh on expiry, and stop on invalid grants/401. Google enforces scope/resource permissions. Missing keyring credentials prevent construction of an authenticated provider. Local settings and offline cached mail belong to the OS user; Overlook has no separate application password or financial transaction service.

Source review; see the referenced code excerpts.

- `crates/overlook-app/src/providers.rs:98`
- `crates/overlook-provider/src/gmail/api.rs:104`

[Evidence image](evidence/casa-2.4.1.png)

## 3.1.1 The application shall enforce least privilege access control rules on a trusted service layer.

Applicable: Yes

Overlook is a single-OS-user desktop client, not a multi-tenant server. Google is the trusted service enforcing token scopes, account ownership, calendar sharing, and contacts permissions. Gmail requests are scoped to users/me using that account’s bearer token. Local profile data is owned by the OS user; editing a local preference cannot grant Google permissions. Selected Gmail, calendar, contacts and Gmail-settings scopes support the implemented mail, calendar, contact editing and settings features; no Drive/admin/domain-wide-delegation scope is requested.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/api.rs:9`
- `crates/overlook-provider/src/gmail/oauth.rs:197`
- `crates/overlook-app/src/providers.rs:40`

[Evidence image](evidence/casa-3.1.1.png)

## 3.1.2 All user and data attributes and policy information used by access controls shall not be able to be manipulated by end users unless specifically authorized.

Applicable: Yes

Overlook is a single-OS-user desktop client, not a multi-tenant server. Google is the trusted service enforcing token scopes, account ownership, calendar sharing, and contacts permissions. Gmail requests are scoped to users/me using that account’s bearer token. Local profile data is owned by the OS user; editing a local preference cannot grant Google permissions. Selected Gmail, calendar, contacts and Gmail-settings scopes support the implemented mail, calendar, contact editing and settings features; no Drive/admin/domain-wide-delegation scope is requested.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/api.rs:9`
- `crates/overlook-provider/src/gmail/oauth.rs:197`
- `crates/overlook-app/src/providers.rs:40`

[Evidence image](evidence/casa-3.1.2.png)

## 3.1.3 Access controls shall fail securely including when an exception occurs.

Applicable: Yes

Overlook is a single-OS-user desktop client, not a multi-tenant server. Google is the trusted service enforcing token scopes, account ownership, calendar sharing, and contacts permissions. Gmail requests are scoped to users/me using that account’s bearer token. Local profile data is owned by the OS user; editing a local preference cannot grant Google permissions. Selected Gmail, calendar, contacts and Gmail-settings scopes support the implemented mail, calendar, contact editing and settings features; no Drive/admin/domain-wide-delegation scope is requested. Missing credentials, keyring errors, rejected refresh tokens and authorization errors fail closed. A failed outgoing-password read no longer falls back to the incoming password.

Provider error-mapping tests and disconnected-client tests passed; source shows no unauthenticated fallback.

- `crates/overlook-app/src/providers.rs:80`
- `crates/overlook-provider/src/gmail/api.rs:113`

[Evidence image](evidence/casa-3.1.3.png)

## 3.1.4 Sensitive resources shall be protected against Insecure Direct Object Reference (IDOR) attacks targeting creation, reading, updating and deletion of records, such as creating or updating someone else's record, viewing everyone's records, or deleting all records.

Applicable: Yes

There is no public Overlook CRUD API. User-controlled identifiers/search inputs reach provider clients, where Google validates the bearer token’s ownership and scope. Gmail paths use users/me; Calendar and People APIs enforce their own sharing/ownership rules. Local SQLite queries for conversations/search bind account IDs. The local profile is a single OS-user trust boundary, not a multi-user service. Native source evidence is supplied for assessor scope review.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/gmail/api.rs:9`
- `crates/overlook-store/src/lib.rs:343`
- `crates/overlook-store/src/lib.rs:1166`

[Evidence image](evidence/casa-3.1.4.png)

## 3.1.5 Application shall enforce a strong anti-CSRF mechanism to protect authenticated functionalityThe application or framework shall enforce a strong anti-CSRF mechanism to protect authenticated functionality, and effective anti-automation or anti-CSRF protects unauthenticated functionality.

Applicable: Yes

The browser-facing sensitive endpoint is the transient OAuth callback. It is bound to 127.0.0.1 on a random port; a valid GET /callback, exact Host authority, matching per-attempt state and unambiguous parameters are required before the listener consumes a response. PKCE S256 binds code exchange. There are no cookie-authenticated state-changing web endpoints. Untrusted mail cannot navigate or submit forms in the isolated renderer.

Forged state, wrong Host/path/method, duplicate Host/state and callback pollution regression tests passed. This is native regression evidence, not a general web DAST report.

- `crates/overlook-app/src/google_signin.rs:148`
- `crates/overlook-provider/src/gmail/oauth.rs:240`

[Evidence image](evidence/casa-3.1.5.png)

## 3.1.6 Directory browsing shall be disabled unless deliberately desiredDirectory browsing shall be disabled unless deliberately desired.

Applicable: No

Overlook operates no document-root HTTP server, hosted directory listing, or public web application. Its only HTTP listener accepts a fixed loopback /callback route during sign-in and rejects other paths; it never serves local directories. Local files are not exposed by a browser server.

Source review; see the referenced code excerpts.

- `crates/overlook-app/src/google_signin.rs:148`

[Evidence image](evidence/casa-3.1.6.png)

## 3.2.1 Application shall implement only secure and recommended OAuth 2.0 flows, such as the Authorization Code Flow or the Authorization Code Flow with PKCE, while avoiding the use of deprecated flows like the Implicit Flow or the Resource Owner Password Credentials Flow.

Applicable: Yes

Google sign-in uses Authorization Code with PKCE S256 in the external system browser and a loopback redirect. A fresh 48-byte random verifier and 24-byte state are generated per attempt. The token exchange sends grant_type=authorization_code and code_verifier. No implicit or resource-owner-password flow is implemented.

RFC 7636 Appendix B challenge test and authorization URL/callback tests passed.

- `crates/overlook-provider/src/gmail/oauth.rs:234`
- `crates/overlook-provider/src/gmail/oauth.rs:271`
- `crates/overlook-provider/src/gmail/oauth.rs:328`

[Evidence image](evidence/casa-3.2.1.png)

## 3.2.2 Ensure that the application securely validates the redirect_uri and state parameters during the OAuth 2.0 authorization process to prevent open redirect and CSRF vulnerabilities.

Applicable: Yes

The redirect is constructed from the bound 127.0.0.1 ephemeral port and fixed /callback path; it is not accepted from untrusted input. The listener verifies method, path, one exact Host header and state, rejecting duplicates or mixed code/error responses before consumption. Token exchange sends the same redirect URI and original PKCE verifier. There is no response-driven redirect destination.

Callback regression rejects mismatched state, duplicate parameters, wrong Host/method/path and duplicate Host headers.

- `crates/overlook-app/src/google_signin.rs:57`
- `crates/overlook-app/src/google_signin.rs:148`

[Evidence image](evidence/casa-3.2.2.png)

## 3.3.1 Application administrative interfaces shall use appropriate multi-factor authentication to prevent unauthorized use.Application administrative interfaces shall use appropriate multi-factor authentication to prevent unauthorized use.

Applicable: No

Overlook has no exposed administrative interface, admin accounts, privileged web console or hosted application backend. All application settings are local to the OS user. Google Account authentication is external, with MFA governed by Google/user policy. This N/A determination concerns application-exposed administrative interfaces; it does not attest to MFA settings on the developer’s Google Cloud or Cloudflare accounts.

Source review; see the referenced code excerpts.

- `crates/overlook-app/src/google_signin.rs:57`
- `crates/overlook-app/src/providers.rs:98`

[Evidence image](evidence/casa-3.3.1.png)

## 4.1.1 Application shall enforce the use of TLS for all connections and default to TLS 1.2+. In cases where support for legacy clients is necessary, TLS 1.0 and 1.1 may be supported if mitigations are implemented to minimize the risk of downgrade attacks and known TLS exploits. Regardless of the TLS version in use, the application shall default to secure cipher suites and reject those with known vulnerabilities.

Applicable: Yes

Off-device Google OAuth/API, update, autoconfiguration, and remote mail-resource requests enforce HTTPS. Remote IMAP/POP3/SMTP require implicit TLS or mandatory STARTTLS before credentials; plaintext is allowed only for explicit loopback endpoints. Rustls defaults support TLS 1.2/1.3. The native OAuth loopback callback is HTTP by the installed-app protocol; it is not an Internet service. No hosted Overlook TLS server exists for SSL Labs. TAC should assess the desktop client’s outbound TLS controls.

Full suite and actual TLS-backed synthetic resource/Chromium consent tests passed. No SSL Labs report is claimed for this desktop executable.

- `crates/overlook-provider/src/gmail/api.rs:61`
- `crates/overlook-provider/src/imap/connection.rs:56`
- `crates/overlook-app/src/browser_engine.rs:1381`

[Evidence image](evidence/casa-4.1.1.png)

## 4.1.2 Connections to and from the server shall use trusted TLS certificates. Where internally generated or self-signed certificates are used, the server must be configured to only trust specific internal CAs and specific self-signed certificates. All others should be rejected.

Applicable: Yes

Production mail and HTTPS clients use Rustls with public WebPKI trust roots and hostname verification. IMAP TLS completes its handshake before authentication and returns errors on certificate/protocol failures; no accept-invalid-certificate switch is implemented. The test suite’s synthetic certificate is trusted only in cfg(test) fixtures, never in the production client.

TLS-backed synthetic resource tests passed. Provider/public-CA validation is delegated to the patched Rustls/WebPKI libraries. No SSL Labs scan of a desktop client is asserted.

- `crates/overlook-provider/src/imap/connection.rs:56`
- `crates/overlook-provider/src/imap/connection.rs:92`
- `crates/overlook-provider/src/smtp.rs:96`

[Evidence image](evidence/casa-4.1.2.png)

## 4.1.3 No instances of weak cryptography which meaningfully impact confidentiality or integrity of confidential data.

Applicable: Yes

Cryptographic uses: PKCE SHA-256 with a freshly generated 48-byte random verifier and 24-byte state; TLS 1.2/1.3 via Rustls with its secure default suites and ephemeral handshake keys; SHA-256 release-file integrity checks; OS-managed encrypted credential storage. Overlook does not implement its own encrypted mail database or application MAC/IV/key-rotation scheme. TLS keys/nonces are library-managed; vault encryption/keys/unlock policy are OS keyring-managed. OAuth token lifetime is provider-managed.

PKCE RFC vector and SHA-256 known-vector tests passed. Patched crypto dependencies have zero known vulnerability advisories in the current RustSec scan; maintenance warnings are documented.

- `crates/overlook-provider/src/gmail/oauth.rs:234`
- `crates/overlook-provider/src/imap/connection.rs:56`
- `crates/overlook-app/src/updater.rs:254`

[Evidence image](evidence/casa-4.1.3.png)

## 4.1.4 All cryptographic modules shall fail securely, and errors are handled in a way that does not enable Padding Oracle attacks.

Applicable: Yes

TLS setup/handshake errors stop the connection before credentials are sent; certificate/protocol failures propagate as rejected operations. Keyring read/write/decryption failures become VaultError and do not trigger plaintext fallback. Overlook exposes no decryption API or network endpoint that reveals padding errors. TLS record cryptography and vault error handling are delegated to the respective libraries/OS service.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/imap/connection.rs:92`
- `crates/overlook-app/src/credentials.rs:81`
- `crates/overlook-app/src/providers.rs:80`

[Evidence image](evidence/casa-4.1.4.png)

## 5.1.1 Protect against HTTP parameter pollution.

Applicable: Yes

The only inbound HTTP query parser is the loopback OAuth callback. Recognized parameter names are URL-decoded and duplicate code/state/error fields are rejected, including encoded aliases. Mixed code+error, missing/wrong state, wrong method/path/Host and duplicate Host headers are rejected. Outbound query values are encoded before use. No public Overlook HTTP application/API exists.

OAuth callback pollution/forgery regression tests passed. Native targeted tests are supplied; a hosted-app DAST scan was not performed.

- `crates/overlook-provider/src/gmail/oauth.rs:286`
- `crates/overlook-app/src/google_signin.rs:148`

[Evidence image](evidence/casa-5.1.1.png)

## 5.1.2 URL redirects and forwards are limited to allowlisted URLs or a warning is displayedURL redirects and forwards are limited to allowlisted URLs or a warning is displayed when redirecting to untrusted content.

Applicable: Yes

Overlook has no server-side redirect/forward endpoint. The loopback callback returns a fixed completion page. Mail documents cannot automatically navigate: href values are isolated for host hit-testing, scripts/forms/frames are removed or disabled, and external links require a user action to open in the system browser. Remote resource redirects are limited to five hops, HTTPS, public resolved IPs, allowed ports/types, and no embedded credentials; private targets and downgrade redirects are rejected.

Real Chromium consent/redirect test and anonymous resource-fetch tests passed. This is native control evidence; external destinations intentionally opened by users are outside the renderer.

- `crates/overlook-app/src/browser_engine.rs:941`
- `crates/overlook-app/src/browser_engine.rs:1411`

[Evidence image](evidence/casa-5.1.2.png)

## 5.1.3 Avoid the use of eval() or other dynamic code execution features. When there is no alternative, any user input is sanitized and sandboxed before being executed.

Applicable: Yes

The mail renderer’s host uses fixed CDP evaluation scripts for DOM preparation/measurement and input hit-testing. Untrusted HTML is passed as serde_json-serialized data to DOMParser, not concatenated as executable JavaScript. Scripts, forms, frames and event handlers are stripped, page script execution is disabled, and all external fetches are intercepted. No user-authored eval or executable template feature exists.

Real Chromium test confirmed injected scripts did not execute and unsafe elements were removed; adversarial document tests passed.

- `crates/overlook-app/src/browser_engine.rs:931`
- `crates/overlook-app/src/browser_engine.rs:873`

[Evidence image](evidence/casa-5.1.3.png)

## 5.1.4 Protect against template injection attacks by ensuring that any user input being included is sanitized or sandboxed.

Applicable: Yes

There is no server-side template engine accepting user expressions. Native UI renders text values; generated HTML escapes user-controlled headers, and untrusted message HTML goes through structural sanitization. The isolated browser receives HTML as JSON-serialized data rather than interpolated executable code. Fixed print-page scripting does not evaluate user content.

HTML formatting/adversarial document tests and real Chromium isolation test passed.

- `crates/overlook-app/src/ui/mod.rs:1509`
- `crates/overlook-app/src/browser_engine.rs:931`
- `crates/overlook-core/src/sanitize.rs:90`

[Evidence image](evidence/casa-5.1.4.png)

## 5.1.5 Prevent Server-Side Request Forgery (SSRF)

Applicable: Yes

Overlook has no hosted SSRF endpoint, but untrusted email resource fetches are treated as an SSRF-equivalent risk to the local device. Chromium is network-isolated. Explicitly consented resources are fetched by a separate anonymous HTTPS client without auth/cookie/referrer or proxy inheritance. PublicResolver filters actual connection IPs (including redirects), rejecting loopback/private/link-local/metadata/documentation ranges. Limits cover ports, MIME types, size, timeout and redirect count. User-configured mail servers are a separate intentional connection path.

Actual TLS resource tests and real Chromium consent tests verified consent gating, no identifying headers, private-redirect denial, and safe redirect handling.

- `crates/overlook-app/src/browser_engine.rs:1330`
- `crates/overlook-app/src/browser_engine.rs:1381`
- `crates/overlook-app/src/browser_engine.rs:1398`

[Evidence image](evidence/casa-5.1.5.png)

## 5.1.6 Protect against XPath or XML injection attacks

Applicable: No

Overlook has no XPath engine, XML query interface, or DTD/external-entity resolver. The limited ISPDB autoconfiguration XML is scanned as text for fixed tag names over HTTPS; it does not evaluate XPath, expand DTDs, or fetch entities. Email HTML/SVG is handled by the isolated/sanitized rendering paths. Thus an XML/XPath injection endpoint is absent.

Source review; see the referenced code excerpts.

- `crates/overlook-provider/src/autoconfig.rs:158`
- `crates/overlook-provider/src/autoconfig.rs:179`

[Evidence image](evidence/casa-5.1.6.png)

## 5.1.7 Context-aware output escaping or sanitization protects against reflected, stored, and DOM based XSS.

Applicable: Yes

Received HTML is untrusted. Ammonia allowlists remove executable elements, event handlers, forms/frames and unsafe schemes; inline CSS is restricted. The isolated browser additionally disables page scripts and applies CSP, DOM filtering and network interception. User header text in generated pages is HTML-escaped. Ammonia was updated to 4.2.4 for the current CSS URL filtering advisory.

Adversarial HTML/CSS tests and real Chromium script-injection checks passed; current RustSec scan reports no known vulnerabilities.

- `crates/overlook-core/src/sanitize.rs:90`
- `crates/overlook-core/src/browser.rs:34`
- `crates/overlook-app/src/browser_engine.rs:873`

[Evidence image](evidence/casa-5.1.7.png)

## 5.1.8 Protect against database injection attacks

Applicable: Yes

SQLite queries use bound rusqlite parameters for account IDs, mail IDs, user text and search expressions. Dynamic SQL fragments are selected by application code rather than supplied as user SQL. Full-text search uses a bound MATCH value and bound account ID. No network-accessible database or SQL query API is exposed.

Store suite passed, including search, account-scoped queries and transactional operations. Evidence is source plus native tests, not a web DAST scan.

- `crates/overlook-store/src/lib.rs:1166`
- `crates/overlook-store/src/lib.rs:405`

[Evidence image](evidence/casa-5.1.8.png)

## 5.1.9 Protect against OS command injections

Applicable: Yes

Application subprocesses use Rust Command with separate arguments. The updater’s terminal fallback now uses a constant bash script and passes the package path as positional $1, quoted at use; package filenames are not interpolated into shell program text. Mail HTML does not execute commands. The browser helper is a local executable with fixed isolation flags.

Source review and workspace checks passed. No live privileged installation was performed as part of evidence collection.

- `crates/overlook-app/src/updater.rs:323`
- `crates/overlook-app/src/browser_engine.rs:491`

[Evidence image](evidence/casa-5.1.9.png)

## 5.1.10 Protect against local file inclusion or remote file inclusion attacks.

Applicable: Yes

No server-side include/file-include endpoint exists. The mail renderer blocks file/javascript navigation, strips frames/objects, and intercepts all resource fetches. Remote resources require public HTTPS. Downloaded attachment names replace directory separators/NUL and neutralize leading-dot names; destinations are constructed below the account/message attachment directory. User-selected local attachments/backups are intentional local file operations, not server includes.

Real Chromium injected file-frame and unsafe-element checks passed; resource policy rejects file URLs and private targets.

- `crates/overlook-app/src/ui/mod.rs:1602`
- `crates/overlook-app/src/browser_engine.rs:1321`
- `crates/overlook-app/src/browser_engine.rs:1381`

[Evidence image](evidence/casa-5.1.10.png)

## 5.2.1 Protect against malicious file uploads by limiting uploads to expected file types and preventing direct execution of uploaded content.

Applicable: Yes

There is no hosted upload directory or server-side execution path. As a mail client, Overlook intentionally supports arbitrary user-selected attachments; they are MIME-encoded/sent as data. Received attachments are stored as files under sanitized names and are not automatically executed. Inline previews are limited to supported image/PDF paths; external opening requires explicit user action. Remote renderer resources have MIME/size/port restrictions. Overlook does not claim antivirus scanning of attachments.

Source review plus mail/attachment and browser isolation tests. Applicability reflects a desktop mail client, not a generic hosted file-upload service.

- `crates/overlook-app/src/ui/mod.rs:1602`
- `crates/overlook-app/src/ui/mod.rs:1582`
- `crates/overlook-app/src/browser_engine.rs:1440`

[Evidence image](evidence/casa-5.2.1.png)

## 6.1.1 The app only uses software components without known exploitable vulnerabilities.

Applicable: Yes

A RustSec cargo-audit 0.22.2 scan of Cargo.lock initially found seven vulnerability advisories. Updated ammonia 4.2.3→4.2.4, rustls 0.23.45→0.23.46, rustls-webpki 0.103.15→0.103.16, and webpki-roots 1.0.9→1.0.10. Re-scan reports zero known vulnerabilities. Eight transitive unmaintained-package warnings remain disclosed, not suppressed. This covers Rust dependencies; the installed Chromium, PDF tools and OS libraries require distribution security updates. TAC must confirm scanner acceptance and any required native/OS inventory coverage.

RustSec database commit 640bf5a9f06e66d8721e8772766531fe77c8630d, updated 2026-10-22. Full before/after JSON is retained in the evidence bundle.

- `llm-docs/casa/logs/dependency-summary.txt:1`

[Evidence image](evidence/casa-6.1.1.png)

## 6.2.1 Disable debug modes in production environments

Applicable: Yes

Release packaging builds cargo --release --locked; Rust release debug assertions are disabled by default. No production web debug console or TCP DevTools endpoint is exposed. The optional renderer uses a private pipe and private runtime directory. Diagnostic logging can be requested locally through RUST_LOG, but protocol/vault logging is unconditionally filtered to prevent wire-secret disclosure even at trace level. This is a desktop production-mode assessment, not a hosted web debug scan.

Release build and native runtime isolation checks are recorded separately. Source reviewed for pipe-only browser debugging.

- `.github/workflows/release.yml:30`
- `crates/overlook-app/src/browser_engine.rs:498`
- `crates/overlook-app/src/main.rs:118`

[Evidence image](evidence/casa-6.2.1.png)

## 6.3.1 The origin header shall not be used for authentication of access control decisions.

Applicable: Yes

Origin headers are not used to authenticate or authorize requests. Provider operations rely on OAuth bearer tokens or mail-provider authentication. The local OAuth callback validates state, PKCE context, method/path and exact Host authority; supplying an Origin header does not bypass those checks. No public CORS-protected Overlook service exists.

Forged callback regression tests passed; no Origin-based authorization mechanism was found in the reviewed source.

- `crates/overlook-app/src/google_signin.rs:148`
- `crates/overlook-provider/src/gmail/api.rs:155`

[Evidence image](evidence/casa-6.3.1.png)

## 6.4.1 The application shall not be susceptible to subdomain takeovers.

Applicable: Yes

Owner statement received 2026-10-22: Cloudflare is the DNS provider and registrar, and all lastrefuge.ai subdomains point to active services under the owner’s control. Public DNS confirms Cloudflare nameservers barbara.ns.cloudflare.com and clark.ns.cloudflare.com; apex and www resolve to 104.251.123.207. Overlook’s OAuth redirect uses 127.0.0.1, not a company subdomain. Public queries are not a complete DNS-zone export; TAC may request the Cloudflare zone/configuration screenshot to substantiate the full inventory.

Owner attestation plus public DNS observation. Registrar ownership and every zone record were not independently verified in Cloudflare.

- `llm-docs/casa/logs/public-dns.txt:1`

[Evidence image](evidence/casa-6.4.1.png)

## 6.5.1 The application shall not log credentials or payment details. Session tokens shall only be stored in logs in an irreversible, hashed form.

Applicable: Yes

Overlook native Linux desktop app. OAuth tokens, client-secret values and PKCE verifiers have redacted Debug output; both log sinks exclude protocol/keyring logging even when trace is requested. A live Google sign-in was completed by the owner on 2026-10-22 using an isolated profile and the patched v1.5.5 build. Authenticated sync downloaded 93 messages and reported up-to-date; OS-keyring metadata confirmed one stored OAuth item without reading its secret. A real login-session log sample is attached with account identifiers redacted. Credential-pattern checks found no matches; this is supplementary evidence, not proof against every possible secret format. Live logging was info level; the earlier synthetic regression tests trace-level filtering. No payment process exists. See the live supplement for build identity and limitations.

User-completed live Google login, authenticated sync, OS-keyring metadata, and sanitized real log verified on 2026-10-22. Secret values were not retrieved. Earlier synthetic trace-level test remains supplemental.

[Initial source/test evidence](evidence/casa-6.5.1.png) · [Live verification supplement](evidence/casa-6.5.1-live.png)

## 6.6.1 Browser storage is securely cleared during logout.

Applicable: No

Overlook does not store authentication tokens in browser cookies, localStorage, sessionStorage or IndexedDB. Its credentials are in the OS keyring, deleted after successful disconnect. The system browser’s Google cookies remain under Google/browser ownership and are not an Overlook session. The mail renderer has no authenticated browser session and uses a disposable private profile cleaned up on teardown. This N/A concerns browser-auth storage, not the separately documented local mail cache.

Native credential storage and renderer teardown source reviewed. Google’s independent browser SSO session is intentionally not cleared by Overlook.

- `crates/overlook-app/src/credentials.rs:114`
- `crates/overlook-app/src/browser_engine.rs:448`

[Evidence image](evidence/casa-6.6.1.png)

## 6.7.1 The application shall securely store access tokens, API keys, and other server-side secrets.

Applicable: Yes

No Overlook server-side secret store or backend exists. OAuth access/refresh tokens and mail-provider passwords are stored using oo7 in OS Secret Service (or encrypted Secret Portal-backed storage in a sandbox), scoped by application/account/kind. Configuration and SQLite do not store these credentials. Keyring failures are surfaced without plaintext fallback. The desktop OAuth client JSON contains a public-client identifier and non-confidential client-secret field; it is not a substitute for user authorization.

Source review; see the referenced code excerpts.

- `crates/overlook-app/src/credentials.rs:17`
- `crates/overlook-app/src/credentials.rs:71`
- `crates/overlook-app/src/credentials.rs:114`
- `crates/overlook-app/src/providers.rs:80`

[Evidence image](evidence/casa-6.7.1.png)
