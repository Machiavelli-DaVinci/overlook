# Recipient autocomplete correction — 2026-10-23

Release version: **1.5.7**.

The reply composer had recipient completion wired exclusively to **To**. Typing
in Cc or Bcc cleared suggestions because To was not focused. The To lookup also
searched only the final comma/semicolon-separated token, even when the cursor was
editing an earlier recipient. Suggestions always appeared below To, and choosing
one always replaced the last token in To.

## Changes

- To, Cc and Bcc use the focused field and its UTF-8 selection to identify the
  recipient being edited. Each suggestion list appears immediately below that
  field. New messages, replies, reply-all and forwards share this behavior.
- Focus and selection changes refresh suggestions without marking the draft
  dirty. Cursor-blink notifications reuse the existing query. Searches continue
  to use account contacts and the cached mail index without database reads on
  each keystroke.
- A suggestion captures the draft ID, recipient field, text and selection.
  Acceptance rejects stale state and selections spanning multiple recipients,
  replaces only the active recipient, preserves neighboring addresses and uses
  the input's atomic replacement operation to retain undo history.
- Completion is suppressed during IME composition. This is a code-level guard;
  real IME behavior has not been verified in a desktop session.
- Recipient tokenization and draft parsing now agree about quoted names and
  angle addresses. Display names containing punctuation are quoted and escaped
  so selecting a contact such as `Doe, Jane` does not produce extra recipients
  when the draft is saved or reopened.

## Verification

- Seven new core regression tests cover all three field identities, appending
  after reply recipients, editing the first/middle recipient, mixed separators,
  Unicode byte offsets, quoted/escaped names, empty tokens, stale text/selection
  rejection and preservation of surrounding recipients.
- `cargo test --workspace --offline`: **365 passed, 29 ignored**. Tests using
  loopback mock servers required execution outside the network sandbox.
- `cargo clippy --workspace --all-targets --offline`: passed with existing
  warnings; no new warnings in the changed completion logic.
- `git diff --check` and the new module's `rustfmt --check`: passed.
- `cargo build --release --offline`: passed; rebuilt
  `/home/jason/coding/omarchy-overlook/target/release/overlook`.
- Workspace `cargo fmt --all -- --check` remains blocked by existing formatting
  differences across the repository, beginning in `assets.rs`.
- Live desktop interaction and visual placement remain unverified because
  native UI automation is unavailable in this session. The automated tests cover
  the recipient model and the full existing application/provider suite, not a
  rendered composer interaction.

Implementation: `crates/overlook-app/src/ui/composer.rs`,
`crates/overlook-core/src/recipients.rs`, and address rendering/parsing in
`crates/overlook-core/src/model.rs`.
