"""Build ready-to-upload logo files for app stores, OAuth consent screens, and social cards.

Outputs go to branding/logos/. Requires fontTools (with brotli) and rsvg-convert; Pillow
is used only to verify the results. Run: python3 branding/source/build_logos.py
"""
from pathlib import Path
import subprocess
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "logos"
OUT.mkdir(exist_ok=True)

MIDNIGHT, IVORY, PAPER, SLATE, AMBER = "#101C24", "#F0EBDD", "#FAF8F2", "#365F73", "#FF9D32"
RIDGE = "M 21 79 L 48 59 L 72 77 L 92 64 L 109 78"


def svg(content, width, height, title="Overlook"):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
            f'viewBox="0 0 {width} {height}" role="img" aria-label="{title}"><title>{title}</title>{content}</svg>\n')


def mark(ink, beacon):
    return (f'<circle cx="64" cy="64" r="49" fill="none" stroke="{ink}" stroke-width="10"/>'
            f'<path d="{RIDGE}" fill="none" stroke="{ink}" stroke-width="8" stroke-linejoin="round" stroke-linecap="round"/>'
            f'<circle cx="79" cy="42" r="5" fill="{beacon}"/>')


def outline(font_path, text, size, tracking=0.0):
    """Returns (svg path, advance width) for text set at `size` px, baseline at y=0."""
    font = TTFont(font_path)
    glyphs, cmap = font.getGlyphSet(), font.getBestCmap()
    scale = size / font["head"].unitsPerEm
    pen, cursor = SVGPathPen(glyphs), 0.0
    for char in text:
        if char == " ":
            cursor += glyphs[cmap[ord("n")]].width * scale * 0.55
            continue
        glyph = glyphs[cmap[ord(char)]]
        glyph.draw(TransformPen(pen, (scale, 0, 0, -scale, cursor, 0)))
        cursor += glyph.width * scale + tracking
    return pen.getCommands(), cursor


def render(svg_text, path, width, height=None):
    args = ["rsvg-convert", "-w", str(width)] + (["-h", str(height)] if height else []) + ["-o", str(path)]
    subprocess.run(args, input=svg_text.encode(), check=True)


# 1. App logo tiles: midnight rounded square (transparent corners) and full-bleed square.
tile = (f'<rect x="4" y="4" width="120" height="120" rx="28" fill="{MIDNIGHT}"/>'
        f'<g transform="translate(10 10) scale(.84375)">{mark(IVORY, AMBER)}</g>')
square = (f'<rect width="128" height="128" fill="{MIDNIGHT}"/>'
          f'<g transform="translate(14 14) scale(.78125)">{mark(IVORY, AMBER)}</g>')
paper = (f'<rect x="4" y="4" width="120" height="120" rx="28" fill="{PAPER}"/>'
         f'<g transform="translate(10 10) scale(.84375)">{mark(MIDNIGHT, SLATE)}</g>')
(OUT / "overlook-logo.svg").write_text(svg(tile, 128, 128))
(OUT / "overlook-logo-square.svg").write_text(svg(square, 128, 128))
(OUT / "overlook-logo-paper.svg").write_text(svg(paper, 128, 128))
for size in (120, 256, 512, 1024):
    render(svg(tile, 128, 128), OUT / f"overlook-logo-{size}.png", size, size)
    render(svg(square, 128, 128), OUT / f"overlook-logo-square-{size}.png", size, size)
render(svg(paper, 128, 128), OUT / "overlook-logo-paper-512.png", 512, 512)

# 2. Social preview card (GitHub 1280x640): lockup, editorial line, endorsement.
word_path, word_w = outline(ROOT / "fonts/SourceSerif4Display-Regular.otf", "Overlook", 150)
line_path, line_w = outline(ROOT / "fonts/SourceSerif4Display-Regular.otf", "A clear view of what matters.", 56)
by_path, by_w = outline(ROOT / "fonts/InterVariable.ttf", "BY LAST REFUGE", 22, tracking=4.5)
W, H = 1280, 640
mark_size, gap = 150, 40
lockup_w = mark_size + gap + word_w
lx = (W - lockup_w) / 2
card = (
    f'<rect width="{W}" height="{H}" fill="{MIDNIGHT}"/>'
    f'<rect width="{W}" height="{H}" fill="url(#sky)"/>'
    f'<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">'
    f'<stop offset="0" stop-color="#1B3140" stop-opacity=".55"/><stop offset="1" stop-color="{MIDNIGHT}" stop-opacity="0"/></linearGradient></defs>'
    f'<g transform="translate({lx:.1f} 212) scale({mark_size/128:.4f})">{mark(IVORY, AMBER)}</g>'
    f'<g transform="translate({lx + mark_size + gap:.1f} 330)"><path d="{word_path}" fill="{IVORY}"/></g>'
    f'<g transform="translate({(W - line_w)/2:.1f} 430)"><path d="{line_path}" fill="#C6D5D5"/></g>'
    f'<circle cx="{(W - by_w)/2 - 22:.1f}" cy="531" r="5" fill="{AMBER}"/>'
    f'<g transform="translate({(W - by_w)/2:.1f} 538)"><path d="{by_path}" fill="#A8B7BA"/></g>'
)
(OUT / "overlook-social-preview.svg").write_text(svg(card, W, H, "Overlook — A clear view of what matters."))
render(svg(card, W, H), OUT / "overlook-social-preview.png", W, H)

try:
    from PIL import Image
    for png in sorted(OUT.glob("*.png")):
        with Image.open(png) as im:
            print(f"{png.name}: {im.size[0]}x{im.size[1]} {im.mode} {png.stat().st_size/1024:.0f} KB")
except ImportError:
    print("built logos (install Pillow to verify dimensions)")
