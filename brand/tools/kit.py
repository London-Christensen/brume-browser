#!/usr/bin/env python3
"""Brume brand kit. One file, all the geometry.

Edit the constants in the GEOMETRY block and re-run. Everything else in the kit
is generated from here, so nothing needs to be kept in sync by hand.

The wordmark is baked in as outlines, so this script needs no font files and no
network access to rebuild the entire kit.
"""
import json, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SVG, GEN = f"{ROOT}/assets/svg", f"{ROOT}/assets/svg/generated"
ICN = f"{ROOT}/assets/icons"

# ============================================================ PALETTE
HAAR, HAAR_DARK = "#4A5C6B", "#9DB2C0"
INK, PAPER, LAMPLIGHT = "#101418", "#F3F4F5", "#C6A87C"

# ============================================================ GEOMETRY
# Canvas is 32. Live area is 4..28, so 24 units, with a 4-unit margin.
#
# Three numbers govern everything, and all three are measured off Archivo Medium
# rather than chosen:
#   MODULE 3   Archivo Medium's stem weight, 0.088em, measured from the 'l'.
#   RADIUS 9   half its bowl, 0.55em, measured from the 'o'. (Cleave has no
#              curves, but the module and radius share one origin, so any future
#              asset in the system stays commensurate.)
#   LIVE  24   the ascender height, so mark and wordmark match in every lockup.
MODULE, RADIUS, LIVE = 3, 9, 24

# Cleave: a square opened along a stepped cut running
#   (10,4) -> (10,16) -> (22,16) -> (22,28)
# Each piece is pulled back MODULE/2 from that line, opening a 3-unit gap. The
# two pieces are congruent: rotate either 180 degrees about (16,16) to get the
# other. That is the whole mark.
CLEAVE = ["M4 4H8.5V17.5H20.5V28H4Z",
          "M11.5 4H28V28H23.5V14.5H11.5Z"]

# Small-size redraw for 16px and 24px. The cut opens to 4 units so it survives
# antialiasing; every other edge is unchanged.
CLEAVE_SM = ["M4 4H8V18H20V28H4Z",
             "M12 4H28V28H24V14H12Z"]

# ============================================================ WORDMARK
# Archivo Medium (wght 500), lowercase, -0.03em, converted to outlines at a
# 100-unit em. SIL OFL 1.1. Ink bounds and baseline offsets travel with it.
_W = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                 "wordmark.json")))
WORD_D, WORD_W, WORD_H = _W["d"], _W["w"], _W["h"]
WORD_X0, WORD_YTOP = _W["x0"], _W["ytop"]

# ============================================================ BUILDERS
def _open(vb, fill, label="Brume", extra=""):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" fill="{fill}"'
            f'{extra} role="img" aria-label="{label}">')


def mark(fill="currentColor", small=False):
    ps = CLEAVE_SM if small else CLEAVE
    return f'{_open("0 0 32 32", fill)}\n  <path d="{" ".join(ps)}"/>\n</svg>\n'


def wordmark(fill="currentColor"):
    vb = f"{WORD_X0} {WORD_YTOP} {WORD_W} {WORD_H}"
    return f'{_open(vb, fill)}\n  <path d="{WORD_D}"/>\n</svg>\n'


def lockup_h(fill="currentColor"):
    """Mark left, wordmark right. Mark height == ascender height. Gap == 9 units
    at mark scale, which is 3 modules."""
    ms = WORD_H / LIVE
    gap = 9 * ms
    total = WORD_H + gap + WORD_W
    return (f'{_open(f"0 0 {total:.2f} {WORD_H:.2f}", fill)}\n'
            f'  <g transform="translate({-4*ms:.4f} {-4*ms:.4f}) scale({ms:.5f})">'
            f'<path d="{" ".join(CLEAVE)}"/></g>\n'
            f'  <g transform="translate({WORD_H + gap - WORD_X0:.2f} {-WORD_YTOP:.2f})">'
            f'<path d="{WORD_D}"/></g>\n</svg>\n')


def lockup_v(fill="currentColor"):
    """Mark centred above. Stacked gets 1.5x the mark so it holds its own against
    the wordmark's width; vertical gap is again 3 modules at that scale."""
    mh = WORD_H * 1.5
    ms = mh / LIVE
    gap = 9 * ms
    mx = (WORD_W - mh) / 2
    total_h = mh + gap + WORD_H
    return (f'{_open(f"0 0 {WORD_W:.2f} {total_h:.2f}", fill)}\n'
            f'  <g transform="translate({mx - 4*ms:.4f} {-4*ms:.4f}) scale({ms:.5f})">'
            f'<path d="{" ".join(CLEAVE)}"/></g>\n'
            f'  <g transform="translate({-WORD_X0:.2f} {mh + gap - WORD_YTOP:.2f})">'
            f'<path d="{WORD_D}"/></g>\n</svg>\n')


def tile(bg=INK, fg=PAPER, small=False):
    """OS icon slot. Radius 21.9% of canvas matches the platform squircle masks.
    Glyph fills 62.5% of the width."""
    ps = CLEAVE_SM if small else CLEAVE
    s = 40 / LIVE
    t = 12 - 4 * s
    return (f'{_open("0 0 64 64", "none")}\n'
            f'  <rect width="64" height="64" rx="14" fill="{bg}"/>\n'
            f'  <g transform="translate({t:.3f} {t:.3f}) scale({s:.5f})" fill="{fg}">'
            f'<path d="{" ".join(ps)}"/></g>\n</svg>\n')


def favicon():
    """Ships as favicon.svg. Follows the browser colour scheme on its own."""
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" '
            'role="img" aria-label="Brume">\n  <style>\n'
            f'    path {{ fill: {INK}; }}\n'
            f'    @media (prefers-color-scheme: dark) {{ path {{ fill: {PAPER}; }} }}\n'
            f'  </style>\n  <path d="{" ".join(CLEAVE_SM)}"/>\n</svg>\n')


# ============================================================ UI ICONS
# The set is Lucide, written into assets/icons/ by tools/build-icons.ps1.
# icons.py reads those files rather than holding paths of its own, so this
# module can render an icon without being able to contradict the shipped one.
import icons as _I


def icon(name):
    return _I.render(name, CLEAVE_SM)


def main():
    for d in (SVG, GEN, ICN):
        os.makedirs(d, exist_ok=True)
    w = lambda p, t: open(p, "w").write(t)

    # sources, currentColor
    w(f"{SVG}/mark.svg", mark())
    w(f"{SVG}/mark-sm.svg", mark(small=True))
    w(f"{SVG}/wordmark.svg", wordmark())
    w(f"{SVG}/lockup-h.svg", lockup_h())
    w(f"{SVG}/lockup-v.svg", lockup_v())

    # colourways
    for nm, fn in (("mark", mark), ("wordmark", wordmark),
                   ("lockup-h", lockup_h), ("lockup-v", lockup_v)):
        w(f"{GEN}/{nm}-ink.svg", fn(INK))
        w(f"{GEN}/{nm}-paper.svg", fn(PAPER))
    w(f"{GEN}/mark-haar.svg", mark(HAAR))
    w(f"{GEN}/mark-sm-ink.svg", mark(INK, small=True))
    w(f"{GEN}/mark-sm-paper.svg", mark(PAPER, small=True))
    w(f"{GEN}/mark-sm-haar.svg", mark(HAAR, small=True))
    w(f"{GEN}/tile-dark.svg", tile())
    w(f"{GEN}/tile-light.svg", tile(PAPER, INK))
    w(f"{GEN}/tile-dark-sm.svg", tile(small=True))
    w(f"{GEN}/favicon.svg", favicon())
    # cleave only. The rest of assets/icons/ is Lucide and belongs to
    # build-icons.ps1; this loop used to write all 44 and would now overwrite
    # the set with whatever this file thought it was.
    w(f"{ICN}/cleave.svg", icon("cleave"))

    n = sum(len(os.listdir(d)) for d in (SVG, GEN, ICN))
    print(f"{n} SVG files written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
