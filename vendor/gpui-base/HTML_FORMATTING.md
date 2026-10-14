# Native HTML presentation

HTML paragraphs carry optional layout metadata; Markdown paragraphs do not.
The HTML parser resolves a bounded presentation vocabulary along the DOM ancestry,
then uses the existing inline highlights and flow layout. Child declarations layer
over ancestor marks. HTML whitespace is collapsed in the AST rather than by a
minifier, allowing authored `pre` / `pre-wrap` whitespace to survive.

Supported presentation:
- Tight `div` text flows versus spaced `p` text flows.
- Nonnegative paragraph margins, including 1–4-value shorthand and authored-order
  shorthand/longhand precedence. Spacing is clamped to 0–96px.
- Inherited color (named, hex, RGB/percent RGB, and RGBA with alpha), font family (first named face, native fallback), font size,
  numeric/normal/bold weight, normal/italic style, alignment and line height.
- Font sizes in px/pt/em/rem/% are clamped to 8–64px. Rem uses a 16px native
  HTML base; percentage margins deliberately use font size, not viewport width.
- Inline font sizes use the existing relative-size flow; size alone never adds
  inline-code chip padding. Left/right/center alignment is supported; justification
  falls back to left alignment.
- Hard breaks, preserved whitespace, block/list boundaries, explicit `th` header
  rows, and table footers.

Deliberate limits (not a browser CSS implementation):
- Vertical spacing uses native additive padding, not CSS margin collapse. Margins
  belong to text paragraphs; margins on containers containing only nested blocks,
  empty elements, tables, lists, headings and code blocks are not fully modeled.
- `pre` elements retain the native code-block rendering. `pre` and `pre-wrap`
  white-space declarations both preserve source whitespace with native wrapping.
- Large inline glyphs do not expand the inherited line box; tight authored line
  heights may overlap. Unitless line heights inherit as ratios; length-valued line
  heights inherit as computed lengths.
- Font fallback lists use only the first face. Legacy font size attributes support
  absolute 1–7 and relative +/− sizes against the HTML base size.
- Decoration addition is supported; explicit removal of an ancestor decoration
  is not. No arbitrary selectors, layout CSS, or external stylesheets are evaluated.
- Table header appearance is row-level: a row containing `th` is treated as a
  header row even when mixed with `td`. Rowspan and true spanning layout remain
  unsupported. GFM export still needs its first-row delimiter; HTML rendering and
  table data use the explicit authored header metadata.

Test independently with:

```
cargo test --manifest-path vendor/gpui-base/Cargo.toml --lib --features test-support
```
