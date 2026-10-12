"""Instance static Inter weights from the bundled variable font.

GPUI's text engine selects faces by family and weight; a variable font exposes only its
default instance, so bold text came out nearly regular. Run with fontTools installed:
    python3 branding/source/build_static_fonts.py
"""
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "fonts" / "InterVariable.ttf"
OUT = ROOT / "fonts" / "static"
OUT.mkdir(exist_ok=True)
for name, weight in [("Regular", 400), ("Medium", 500), ("SemiBold", 600), ("Bold", 700)]:
    font = TTFont(SRC)
    static = instancer.instantiateVariableFont(font, {"wght": weight, "opsz": 14}, updateFontNames=True)
    # Name the family plainly "Inter" so the app can request it by that name.
    style = name
    full = "Inter" if name == "Regular" else f"Inter {name}"
    for rec in static["name"].names:
        if rec.nameID in (1, 16):
            rec.string = "Inter"
        elif rec.nameID in (2, 17):
            rec.string = style
        elif rec.nameID == 4:
            rec.string = full
        elif rec.nameID == 6:
            rec.string = f"Inter-{name}"
    static.save(OUT / f"Inter-{name}.ttf")
    print("wrote", OUT / f"Inter-{name}.ttf")
