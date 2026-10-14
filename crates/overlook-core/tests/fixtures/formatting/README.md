# Synthetic email formatting regression corpus

All content was authored for these tests. **No personal messages were exported.**
Names, dates, subjects, inventory, code, and prose are invented; addresses and
resource URLs use reserved `example.test` domains. Client names describe markup
patterns, not captured client output. URLs are inert test strings; tests do not
fetch remote resources.

Run from the repository root:

```sh
cargo test -p overlook-core --test message_formatting
```

## Coverage

| Fixture | Representative behavior |
| --- | --- |
| `outlook_signature.html` | Word-style paragraphs, Calibri/point sizes, zero margins, bold signature, `o:p`/NBSP spacer, mailto and HTTPS links |
| `gmail_div_br.html` | Div-based paragraphs, consecutive blank blocks, double/leading/trailing breaks, signature spacing |
| `apple_inline.html` | Inline Helvetica typography, numeric weight, italic, underline/strike, alignment, line-height, margin shorthand, pre-wrap, cite history, Unicode |
| `plain_quote.txt` | Escaped plain text, single/blank lines, nested `>` history, bare URL with query parameters |
| `newsletter_layout.html` | Nested presentation tables, long editorial cells, hidden preheader/title, remote banner/tracking pixel, media-query input |
| `data_table.html` | Real three-column table with caption, thead/th, tbody, tfoot totals |
| `nested_lists.html` | Ordered-list start, unordered/ordered nesting, list spacing, definition list |
| `pre_code.html` | Preformatted indentation and blank line, escaped markup, inline code comparison, monospace/white-space |
| `forwarded_history.html` | Gmail forward wrapper plus Outlook-style original-message headers; full inline history and separators |
| `reply_history.html` | Gmail reply wrapper, nested cite history, folding for reader but full history for quotes/search |
| `unsafe_malformed.html` | Misnested/unclosed tags, scripts/style/frame/SVG payloads, comments, event handlers, unsafe URL schemes, CSS attacks, remote and CID images |
| `hidden_content.html` | Hidden attribute, display/visibility/opacity, Outlook mso-hide, zero height/font size, preview class, tracking pixel and hidden quote |
| `color_scope.html` | Dark/light and legacy font colors; literal color-like text, title attribute, URL parameters and inline code must not change |
| `css_adversarial.html` | Mixed-case safe properties alongside active CSS values and unsupported positioning/background/transform declarations |

## Contracts and assertion strategy

`message_formatting.rs` uses the actual public core APIs:

- `clean_only`: full-history sanitization used for outgoing quoted HTML; hidden
  content and active content must still be removed. No reader-specific folding or
  layout flattening is expected here.
- `sanitize_html`: visible reader body plus optional folded history, searchable
  text containing both, layout-table flattening, and remote-content accounting.
- `adapt_colors`: theme changes confined to color styling, without changes to
  DOM text, URLs, titles, image alternatives, or non-color typography/spacing.
- `html_to_text`: meaningful visible/searchable text without script/style payloads,
  including when called directly on unsanitized HTML.
- `text_to_html`: safe escaping, nested plain-text quotation, and linkification.

Assertions use parsed nodes/attributes, property-level CSS checks, meaningful text
markers, element counts/nesting, and DOM-equivalent idempotence, rather than whole
serialized HTML snapshots. CSS values in these small fixtures are intentionally
simple: known authored declarations must survive, but declaration ordering and
HTML attribute ordering are not pinned. The DOM helper retains its owning `RcDom`
because dropping that owner drains descendants of otherwise cloned handles.
Blank blocks and breaks are checked in HTML; normalized search text is **not**
required to represent every visual blank line or indentation character.

Idempotence is tested for each cleaning policy, reader body reprocessing, and
repeated same-theme color adaptation. Reprocessing a reader body alone does not
reconstruct history that was already split out. Reply history is checked separately
for completeness. Generic plain-text blockquotes preserve nesting/history but are
not required to auto-fold: the current splitter recognizes client-specific reply
wrappers and cite attributes. Forwarded history must remain inline.

## Scope and unsupported rendering

These are core transformation tests, not a browser rendering or screenshot suite.
They do not claim pixel-identical fidelity to Outlook, Gmail, or Apple Mail.

- Full CSS engines/cascade, stylesheet selectors, external stylesheets, web fonts,
  CSS variables, flex/grid, arbitrary positioning, backgrounds, and active CSS are
  unsupported. Safe inline typography/spacing is the contract; other CSS should
  not acquire browser-like behavior.
- Media queries and responsive newsletter breakpoints are not evaluated. The
  stylesheet in the newsletter is intentionally removed, not interpreted.
- Layout tables are flattened into readable flow; true data tables retain semantic
  markup. Table `rowspan`/`colspan` geometry and complex multi-level headers are
  unsupported renderer capabilities, not asserted by this corpus. The basic tfoot
  fixture does not rely on spans or browser layout.
- Exact font availability, line box metrics, margins in pixels, native renderer
  layout, and theme contrast thresholds require separate renderer tests.
- Remote-content allowance must never enable executable markup or CSS. Link
  destinations remain intact in both image policies; CID images remain available.

## Compatibility notes

The safe-style policy preserves bounded margin shorthand and individual sides.
The adversarial CSS fixture checks safe horizontal margins alongside rejection of
unbounded and active CSS values. Zero height without clipping remains visible;
height plus `overflow:hidden` is treated as a hidden preheader.

Preformatted indentation may be encoded with NBSP and newlines with `br`; tests
accept either NBSP/space indentation and either newline/break representation.
Generic plain-text quotes are not currently client-specific foldable wrappers.

Regression failures are not grounds to weaken security or preservation assertions.
Update the shared presentation contract and renderer together when adding support.
