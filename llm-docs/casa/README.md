# Overlook CASA AL1 assessment handoff

Prepared 22 October 2026 for the TAC Security assessment of Last Refuge Software's **Overlook**, registered in the portal as a **Desktop** application. This is a developer evidence package, not an audit certification.

**Submitted to TAC on 22 October 2026 with the owner's explicit authorization.** The portal now reports **In Progress** and “Your evidence is under tester review. Access will be available once validation is complete.” All **48 controls and 50 evidence images** were verified immediately before submission. [Submission receipt](submission-status.json) · [Confirmation screenshot](portal-submission-confirmation.png).

## Live verification update — 22 October 2026

The owner completed Google sign-in using a separate application profile and an account not present in the normal profile. The tested binary was built from commit `5146faa939922efc006732fc1f59060acac8aaee` with the patched `Cargo.lock` still in the working tree. The security source changes are now included in that commit; the initial evidence below predates it. The tested binary SHA-256 is recorded in [live-login-verification.json](live-login-verification.json).

Authenticated sync downloaded **93 messages**, became **up-to-date**, and created **one unlocked OAuth credential item** in the OS keyring. Keyring inspection used metadata only; secret values were not retrieved. A [real login-session log](logs/live-google-login.txt), with account identifiers redacted, replaces the previous evidence gap for 6.5.1. Credential-pattern scans found zero matches; this is supplementary validation, not proof against every possible credential format. The live run used info-level logging; the existing synthetic test covers trace-level filtering.

The owner then used **Remove account**. Read-only checks found **zero Gmail accounts, zero cached messages and sync rows for the account, and zero matching OS-keyring items across all credential kinds**. [The removal record](live-removal-verification.json) preserves the counts. Completion of the guarded application path demonstrates that its revocation step returned success; this is an inference from the reviewed path and completed cleanup, not an independent revoked-token replay or raw HTTP trace.

Controls **6.5.1** and **2.2.1** each have updated saved comments and **two portal attachments**: initial source/test evidence plus the corresponding [login](evidence/casa-6.5.1-live.png) or [removal](evidence/casa-2.2.1-live.png) live supplement. Portal proof is saved for [login](portal-live-login-proof.png) and [removal](portal-live-removal-proof.png). After reload, all 48 comments and applicability selections matched the canonical responses, with **50 images** saved in total.

The isolated test instance was closed, and its temporary mail profile/cache and raw logs were removed. Only sanitized evidence remains in this repository; no keyring secret was read during verification. [Cleanup record](live-session-cleanup.json). The normal Overlook profile was not touched by cleanup. Removal of temporary files is not a claim of forensic secure erasure.

## Portal status

All **48 controls** have saved developer comments, applicability selections, and uploaded PNG evidence documents: **33 applicable / 15 not applicable**. Initially one image was uploaded per control; live supplements are noted above. After uploads, the portal was reloaded and all comments and selections were compared with `responses.json`; all 48 evidence links were present, with **zero mismatches**. An uploaded image was also opened and confirmed to render from the portal.

- [Saved-row verification](portal-verification.json)
- [Assessment overview screenshot](portal-overview.png)
- [Portal screenshot](portal-evidence-saved.png)
- [Uploaded attachment screenshot](portal-upload-proof.png)
- [Control-by-control explanations, validation and limitations](control-matrix.md)
- [Exact portal requirements and guidance](requirements.json)
- [Exact responses and local evidence filenames](responses.json)

**The final Submit action was performed with the owner's explicit authorization to submit the current evidence and let TAC request anything additional.** The reviewed-checklist checkbox was selected, and the portal redirected to the assessment list with evidence access disabled because it is under tester review. Assessor decisions about native-app scanning evidence and release alignment remain follow-up items below. No audit pass or certification is claimed; applicability selections are not assessor results.

The portal dropped some rapid comment/applicability autosaves. Those entries were repaired individually and checked after reload. For subsequent edits, verify persistence by reloading rather than relying on the value visible immediately after typing.

## Assessed source and build

The initial evidence covers an **unreleased local v1.5.5 assessment candidate** based on Git commit `b80893a5927d` (full commit recorded in the manifests). At collection time, security changes were uncommitted. The later live build and commit are identified above. The candidate was built and locally launched for the live test, but **not installed, published, or distributed by this assessment workflow**. Existing released binaries do not necessarily contain these fixes.

- [Source hashes](source-manifest.json): exact files referenced in the evidence.
- [Build provenance](build-provenance.json): release command, baseline and binary SHA-256.
- [Evidence bundle hashes](bundle-manifest.json): integrity inventory of the saved documentation, images and logs.
- [Tracked security changes](security-fixes.patch): review snapshot; new TLS fixture files are separately retained under `crates/overlook-app/tests/fixtures/`.
- Release binary: `target/release/overlook`, reports `overlook 1.5.5`.
- Binary SHA-256: `701c68bc49d45ad693f0ce3a2ac9a21f6cdcd259e2e16ebd8970b0d0114e7057`.

The initial 48 PNGs in `evidence/` are **rendered source/test documents**, explicitly labeled as such. Later live supplements are rendered records of actual observations/log excerpts, also explicitly labeled. None is presented as a native application screenshot. They contain code excerpts with source hashes and line references, architecture explanations, test results and explicit limitations. Real mailbox contents, OAuth credential values, keyring secret values and production credential files are not included in this package.

## Security changes included

1. Updated vulnerable Rust dependencies: `ammonia` 4.2.4, `rustls` 0.23.46, `rustls-webpki` 0.103.16 and `webpki-roots` 1.0.10.
2. Hardened the OAuth callback: exact loopback Host, method and path checks; state validation before consuming a callback; rejection of duplicate/ambiguous parameters; robust percent decoding; defensive response headers.
3. Added Google grant revocation to account removal. A shared client lifecycle prevents cloned providers from refreshing or persisting credentials after disconnect. Network and keyring failures leave removal incomplete and report the failure. Work runs off the UI thread.
4. Redacted credential-bearing Debug output and excluded protocol/keyring logging targets from both log sinks, including when `RUST_LOG=trace` is requested.
5. Required HTTPS for Google HTTP clients, update/autoconfiguration clients and remote email resources. Google authenticated clients refuse redirects. Existing TLS and public-address resource checks remain in place.
6. Removed updater package-path interpolation into shell program text; the fixed script receives the path as a quoted positional argument.
7. Made outgoing-password keyring read failures fail closed instead of silently falling back to the incoming password.
8. Added synthetic TLS fixtures and regression coverage for the changed security behavior.

**Behavior to review before release:** removing a Gmail account now requires connectivity to revoke its grant. Revocation can invalidate that user's authorization for the Google project across devices and scopes, including calendar/contact access. The confirmation text explains this. A failed disconnect leaves that client disabled until removal is retried; the account remains available for retry. Ordinary offline mail access is otherwise unchanged. Remote images served only over plain HTTP are blocked.

## Validation

| Check | Result | Evidence |
| --- | --- | --- |
| `cargo test --workspace --offline` on final source | 294 passed, 0 failed, 26 ignored | [Full test log](logs/tests.txt) |
| Actual Chromium consent and redirect test with synthetic TLS servers | Passed | [Consent log](logs/browser-consent.txt) |
| Actual Chromium private-pipe DOM/script/frame isolation test | Passed | [Isolation log](logs/browser-isolation.txt) |
| Synthetic login-credential logging at trace level | Passed; not a live Google login | [Logging sample](logs/synthetic-login.txt) |
| Actual user-completed Google sign-in and authenticated sync | Passed: 93 messages, up-to-date sync, one stored OAuth keyring item | [Live verification](live-login-verification.json), [sanitized log](logs/live-google-login.txt) |
| Actual user-completed account removal | Passed local cleanup: account/cache/sync/keyring counts all zero; successful remote revocation path inferred, no independent token replay | [Removal verification](live-removal-verification.json) |
| `cargo build --release --locked --offline -p overlook` | Passed | [Release log](logs/release-build.txt) |
| `cargo clippy --workspace --all-targets --offline` | Completed with warnings; not warning-clean | [Clippy log](logs/clippy.txt) |
| `cargo fmt --all -- --check` | Failed; repository has broad formatting differences, not reformatted in this task | [Formatting log](logs/format.txt) |
| `git diff --check` | Passed | Working-tree check |
| `cargo-audit` 0.22.2 / RustSec | 7 initial vulnerability advisories; 0 after updates; 8 maintenance warnings retained | [Before](logs/dependencies-before.json), [after](logs/dependencies-after.json), [summary](logs/dependency-summary.txt) |

The RustSec database commit was `640bf5a9f06e66d8721e8772766531fe77c8630d`, updated 22 October 2026. The unmaintained dependency warnings concern `instant`, `paste`, `rustls-pemfile`, `rustybuzz`, `ttf-parser`, `zbus_names`, `zvariant` and `zvariant_derive`. They were not suppressed. This scan covers Cargo.lock, not installed Chromium, PDF tools or the distribution's system libraries.

Ignored tests were not represented as passing. Two explicitly selected browser integration tests were run separately as recorded above. The fixtures trust a disposable synthetic certificate only under `cfg(test)`; production certificate validation was not weakened. No live privileged update installation was performed. A real Google sign-in/removal was performed later, with the observation/inference distinction described above.

## Follow-up items for TAC review and release

| Controls | Remaining evidence or decision | Next step |
| --- | --- | --- |
| 4.1.1–4.1.2 and controls marked for DAST | TAC acceptance of outbound/native TLS and native regression evidence in place of hosted-web scans | Explain that the registered application is a desktop client, has no public Overlook server, and uses only a transient loopback callback. Ask TAC which native tests or reports they require. No Qualys SSL Labs result or hosted-web DAST result is claimed. |
| 6.1.1 | Acceptance of RustSec/cargo-audit and scope of OS/helper dependency inventory | Confirm the accepted scanner with TAC. Supply additional approved scanning and OS/helper inventory if requested; follow up on the eight maintenance warnings. |
| 6.4.1 | Complete DNS-zone evidence if TAC requires it | Owner confirmed Cloudflare registrar/DNS and active controlled subdomains. Public queries confirm nameservers/apex/www, but are not a complete zone export. Obtain a Cloudflare DNS configuration screenshot/export if requested; avoid account tokens and unrelated account details. |
| All controls relying on changes | Release and evidence alignment | The security source changes are included in commit `5146faa939922efc006732fc1f59060acac8aaee`, but the dependency fixes in **Cargo.lock remain uncommitted**. Include those updates in the release, identify the exact shipped commit/build, and refresh evidence if source changes. Do not claim an older published binary implements these changes. |

The local mail cache is not application-encrypted. Credential protection depends on the OS Secret Service or encrypted Secret Portal-backed keyring and the OS-user boundary. These facts are disclosed; no broader storage-encryption claim is made.

The assessment has been submitted with these limitations disclosed. Respond to any TAC requests for additional reports or evidence when received. Refresh the relevant responses/evidence if the portal is reopened for changes, and verify persistence. Do not interpret the applicability selections or submission confirmation as TAC approval.

## Reproduction and maintenance

From the repository root:

```sh
cargo test --workspace --offline
cargo test -p overlook --offline chromium_controlled_redirect_consent -- --ignored --exact browser_engine::tests::chromium_controlled_redirect_consent
cargo test -p overlook --offline chromium_private_pipe_live_dom -- --ignored --exact browser_engine::tests::chromium_private_pipe_live_dom
cargo test -p overlook --offline synthetic_login_logs_redact_credentials -- --nocapture
cargo build --release --locked --offline -p overlook
cargo clippy --workspace --all-targets --offline
cargo fmt --all -- --check
git diff --check
```

The synthetic network tests need permission to bind local loopback sockets. Browser tests require Chromium and use their own private fixture/profile environment. `cargo-audit` was installed from crates.io under `/tmp/overlook-casa-tools`; retain the saved JSON reports because that temporary tool location is not durable.

`build_evidence.py` regenerates the initial `responses.json`, `control-matrix.md`, `source-manifest.json` and 48 images from source and saved audit/DNS/log inputs. Run `build_live_evidence.py` afterward to generate live supplements and apply their updated responses and matrix sections. The live builder uses the saved observed-session JSON and verifies the sanitized log hash. These scripts require Python with Pillow and `fc-match`; the bundled Python used here was `/home/jason/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3`.

Regeneration is local only. It does not update the TAC portal. If source changes, compare manifests, regenerate the affected evidence, upload it through the UI, and perform another reload verification. Do not upload a newly generated source manifest alongside stale images.
