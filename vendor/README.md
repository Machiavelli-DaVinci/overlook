# Vendored crates

`gpui-base` is a copy of the crates.io release named in its `Cargo.toml` (Apache-2.0), wired in
through `[patch.crates-io]` in the workspace `Cargo.toml`. Every change is marked "Overlook patch"
in `src/text/format/html.rs`; nothing else is modified. The patches make the HTML view render mail
the way a browser-based client does:

- `<br>` is a hard line break inside a paragraph (upstream dropped it inline and made it an
  empty block at block level), so Gmail's `<div>`-and-`<br>` bodies keep their lines.
- The running text in a block is flushed only before a block-level child, so inline runs and
  line breaks inside a `<blockquote>` or list item stay one paragraph.
- `<hr>` renders as a rule, `<pre>` as a code block that keeps its lines, and `<ol start>` is honoured.

To upgrade: copy the new release here, re-apply the marked hunks, run `cargo test --workspace`,
and re-check `llm-docs/overlook-native-app.md#rendering`. Delete this directory and the patch entry
when upstream ships equivalents.
