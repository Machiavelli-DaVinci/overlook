# Overlook Brand Kit

Open the [visual brand book](../branding/index.html) to explore the identity. The complete written specification is [Overlook Brand System](overlook-brand.md). Written documentation lives in `llm-docs/`; reusable artwork, fonts, tokens, previews, and build sources live in `branding/`.

**Overlook — A clear view of what matters.** A Last Refuge product.

## Contents

Paths below are relative to `branding/`, unless they start with `../llm-docs/`.

| File or directory | Use |
|---|---|
| `index.html`, `brand.css` | Visual brand book with Light/Dark product preview |
| `mail-preview.html` | Interactive application design using fictional sample mail |
| `assets/overlook-lockup-{light,dark,mono}.svg` | Portable outlined horizontal signatures |
| `assets/overlook-wordmark-{light,dark,mono}.svg` | Portable outlined wordmarks |
| `assets/overlook-mark-{light,dark,ink,ivory}.svg` | Standalone horizon marks |
| `assets/overlook-symbolic.svg` | Single-color small-size mark using `currentColor` |
| `assets/overlook-app-icon.svg` | Scalable app icon |
| `assets/overlook-icon-*.png` | App icons from 16 to 512 pixels |
| `assets/overlook-lockup-*.png` | Transparent 1600-pixel signature exports |
| `assets/overlook-alpine-dawn.png` | Original generated hero artwork |
| `logos/` | Ready-to-upload logos: 120 px OAuth consent-screen tile, 256–1024 px tiles, full-bleed squares, paper variant, 1280×640 social preview (see `logos/README.md`) |
| `fonts/` | Source Serif 4 and Inter, including licenses and native-use font files |
| `tokens.json`, `tokens.css` | Light/Dark semantic color specifications |
| `contrast-check.json` | Measured contrast of key UI pairs |
| `previews/` | Captured presentation and application previews |
| `../llm-docs/overlook-art-direction.md` | Exact final image-generation prompt and provenance |
| `source/build_assets.py` | Rebuild SVG/PNG exports and validate contrast |
| `source/build_logos.py` | Rebuild the platform logos and social preview in `logos/` |
| `source/prepare_mail_preview.py`, `source/mail-prototype.html` | Rebuild the branded preview from the preserved prototype |
| `source/package_brand.py` | Rebuild the portable brand archive, including documentation |
| `overlook-brand-kit.zip` | Portable copy of `branding/` and `llm-docs/` |
| `vendor/` | Locally bundled Lucide library and license for the preview |

Fonts, icons, and artwork are local; the visual brand book works without external requests. Open the HTML directly, or run `python3 -m http.server --bind 127.0.0.1 8771` from the repository root and open `http://127.0.0.1:8771/branding/`. Serving the repository root keeps links to `llm-docs/` accessible.

The mail preview is a design prototype. Actions affect temporary sample data only, and disappear on reload. It does not send email or implement the planned offline synchronization engine.

The primary logo is supplied for light backgrounds, the inverse for dark backgrounds. For small icons, use the size-specific PNG or symbolic mark. Wordmarks are outlined paths and do not depend on font installation. Keep the font license files with redistributed fonts, and the Lucide license with the preview library.

Generated landscape artwork was created with the built-in image generation tool. It is an original fictional scene informed by the atmosphere of [Last Refuge](https://lastrefuge.ai/). The exact prompt is in [Overlook Art Direction](overlook-art-direction.md). Logos and layout were authored as editable vector/code assets.

## Maintaining the kit

The selected brand direction is the baseline for implementation. Keep the existing assets and semantic tokens consistent with [the brand specification](overlook-brand.md); avoid inventing additional v1 themes or replacing approved typography in isolated screens.

The ready-to-use artwork and HTML need no build step. To regenerate:

1. With Python `fonttools`, `brotli`, and `Pillow`, plus `rsvg-convert`, run `python3 branding/source/build_assets.py`. Palette and theme definitions in that source generate the JSON/CSS tokens and contrast report alongside the artwork; change the source when changing generated values.
2. Run `python3 branding/source/prepare_mail_preview.py` to reapply the identity to the included prototype. Its style mapping is explicit, so align it with token changes when revising the palette.
3. Inspect the brand book and both app appearances, and refresh the screenshots in `branding/previews/` after visual changes. These are reference captures, not automatically generated build outputs.
4. Run `python3 branding/source/package_brand.py` to refresh the distributable ZIP after updating assets or documentation.

Keep font and icon licenses beside the assets they cover when distributing the kit. General brand and product documentation remains in `llm-docs/`.
