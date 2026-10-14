# Message formatting: policy, review, and regression coverage

## Why this is a pipeline problem

Email HTML is not ordinary Markdown. A `<div>` line, a zero-margin Outlook paragraph,
wrapping text, an explicit `<br>`, and a genuinely separate paragraph must not all
receive the same paragraph gap. Formatting must survive both sanitization and native
layout; accepting a CSS declaration without rendering it is not sufficient.

The review traced Gmail MIME decoding, original body storage, structural normalization,
security sanitization, native HTML parsing/layout, quote folding, outgoing alternatives,
composer quote previews, and printing. A read-only structural sample of 250 recent local
messages found 203 with tables, 165 with stylesheets, and 139 with media queries. Only
aggregate structure counts were retained; real message text is not committed as fixtures.

Original received HTML remains unchanged in SQLite. Changes to display policy do not
require redownloading messages and must not rewrite stored originals or outgoing content
when a user changes the app theme.

## Design rules

1. **Security and presentation are separate concerns.** Remove executable markup,
   handlers, unsafe URL schemes, hidden tracking/preheader content, and unapproved CSS.
   Preserve explicitly supported, bounded presentation rather than stripping all styling.
2. **Use one safe presentation contract.** Typography, block spacing, alignment, and
   whitespace need matching sanitizer and renderer support. Reject unsupported resource
   functions, positioning, imports, and arbitrary CSS; do not enable raw stylesheets.
3. **Keep structural distinctions.** HTML flow lines and authored zero margins are not
   Markdown paragraphs. Preserve hard breaks and intentional blank blocks. Markdown's
   existing rendering defaults must remain independent of email HTML defaults.
4. **Respect inheritance.** Inner styling overrides outer styling. Block text colors and
   typography apply to descendants; semantic emphasis and explicit resets need deliberate
   handling rather than an inside-out OR of flags.
5. **Theme adaptation is local and attribute-scoped.** Never rewrite text nodes, URLs,
   stored HTML, or sender formatting in outgoing mail because of the current theme.
6. **Resource consent applies to every local preview.** Expanding a quoted original in
   the composer must not bypass remote-image blocking. Printing observes the same consent.
   Outbox previews use that same shared preview policy, not outgoing serialization directly.
7. **Do not hide novel content.** A prefix shared with an earlier message is insufficient
   evidence that an entire quote is redundant. Prefer showing extra history over hiding an
   inline answer or annotation. Printing includes full visible history.
8. **Both MIME alternatives preserve Unicode and visible content.** UTF-8 must never be
   processed one byte at a time as characters. Generated plain-text quotes use the same
   visibility-clean source as their HTML alternative.

## Review findings

The previous implementation removed all inline CSS except color; applied a generic
paragraph gap to both `<p>` and `<div>`; deleted blank blocks and boundary breaks; and
ignored block color inheritance. Sender font size and line height were replaced by app
fallbacks. Additional findings included Unicode corruption in outgoing HTML, hidden
content revealed by outgoing quote sanitization, composer quote previews permitting
remote images, string-wide theme color replacement, and printing dropping folded history.

Native table parsing also needs actual header/footer semantics rather than treating every
HTML table as a Markdown table. List item block boundaries and mixed-size text must be
checked in the renderer, not only by examining serialized sanitized HTML.

## Regression workflow

- `crates/overlook-core/tests/fixtures/formatting/` contains synthetic, sender-shaped mail.
  Never commit private mailbox exports, access tokens, tracking links, or personal addresses.
- Core integration tests exercise security, supported styling, visible text, whitespace,
  quote preservation, and normalization stability.
- Native renderer tests exercise parsed structure and layout. Sanitizer tests alone cannot
  detect excessive gaps, mixed-font shaping, table header mistakes, or selection regressions.
- MIME tests decode the generated alternatives and check actual Unicode text, not merely
  the beginning of an encoded HTML part.
- `cargo test --workspace` runs application/core/provider/store tests.
- `cargo test --manifest-path vendor/gpui-base/Cargo.toml --lib --features test-support`
  runs the vendored native renderer tests.

An optional privacy-preserving diagnostic accepts NDJSON objects containing an `html`
field and prints aggregate stability/resource counts only:

```sh
cargo run -p overlook-core --example formatting_audit < local-message-input.ndjson
```

Do not commit that input file. The diagnostic does not contact mail providers, modify
stored messages, or print message content. It checks parsed-DOM cleaning/reader
idempotence, ignoring equivalent entity spellings and attribute order—not visual
equivalence to Outlook. The full stored-HTML audit passed with zero DOM instability.
Plain-text display separately preserves blank runs, indentation, CRLF, and hard breaks.

## Explicit limits and follow-up gates

See [the native presentation contract](../vendor/gpui-base/HTML_FORMATTING.md) for
exact capabilities, sizing bounds, and deliberate fallback behavior.

This is a native text/layout renderer, not a browser engine. Arbitrary stylesheet cascade,
media queries, responsive multi-column newsletters, CSS positioning, complex nested layout
tables, and true row/column spanning require additional layout capabilities or a separate
HTML rendering architecture. A safe inline presentation subset is not a promise of exact
Outlook/WebKit fidelity for those messages.

Quoted CID images also need source inline parts to be carried into outgoing MIME; retaining
a `cid:` URL alone does not make the image available to recipients. This needs a dedicated
MIME/attachment lifecycle change rather than a spacing patch.

Release review should compare the fixture corpus visually at multiple pane widths and
scales, in Light and Dark, including selecting/copying text and blocked/allowed images.
Automated structure, security, and layout tests reduce regressions; they do not replace
that visual check. Do not declare browser-grade fidelity based solely on passing unit tests.
