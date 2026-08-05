#!/usr/bin/env python3
"""Brume UI icon set.

The set is Lucide, ISC licensed, generated into assets/icons/ by
tools/build-icons.ps1. This module holds no path data any more: it reads those
files back. That is deliberate and load-bearing. While the paths lived here,
kit.py rewrote all 44 icons on every run, so regenerating the mark would have
silently reverted the icon set to something else.

WHY THE SET CHANGED. It used to be drawn here, to a house rule of 90 degrees,
with 45 permitted only where an object carries direction and arcs only where an
object genuinely is round. It was internally consistent, and it was not good
enough to look at. Lucide is the same 24 grid, the same 2px stroke and the same
currentColor, so nothing downstream had to change. It uses round caps and joins
where this set used butt and miter, which is most of the visible difference.

The mapping from Brume's names to Lucide's lives in tools/build-icons.ps1, next
to the code that acts on it, rather than being restated here where it could
drift.

cleave is the exception and is still composed below. It is the mark itself
mapped onto the icon grid, the one glyph in that directory that is Brume's own,
and no part of Lucide.
"""

import os

# ---------------------------------------------------------------- geometry
GRID, LIVE, STROKE = 24, 20, 2
LENS_R, DISC_R = 7, 9          # 9 is the system radius scaled to this grid

ICON_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "assets", "icons")

# --- brand glyph -------------------------------------------------------------
# NOT a UI icon. This is the mark itself, filled, mapped onto the icon grid so it
# sits correctly beside the set in menus and about dialogs. The v2.0 file drew a
# stroked approximation of it, which is why it did not match the logo. It uses
# the small-size redraw because it renders at icon sizes.
#   mark live area 4..28 -> icon live area 2..22, so scale 20/24 and shift -4/3.
BRAND_SCALE = LIVE / 24
BRAND_SHIFT = 2 - 4 * BRAND_SCALE


def brand_glyph(cleave_sm):
    return (f'<g transform="translate({BRAND_SHIFT:.4f} {BRAND_SHIFT:.4f}) '
            f'scale({BRAND_SCALE:.5f})" fill="currentColor" stroke="none">'
            f'<path d="{" ".join(cleave_sm)}"/></g>')


def render(name, cleave_sm=None):
    """The SVG for one icon.

    cleave is composed from the mark geometry. Everything else is read straight
    off disk, so this function cannot produce an icon that differs from the one
    the app is actually shipping.
    """
    if name == "cleave":
        body = "  " + brand_glyph(cleave_sm)
        return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" '
                'stroke="currentColor" stroke-width="2" stroke-linecap="butt" '
                f'stroke-linejoin="miter" role="img" aria-label="{name}">\n{body}\n</svg>\n')

    path = os.path.join(ICON_DIR, f"{name}.svg")
    if not os.path.exists(path):
        raise FileNotFoundError(
            f"{path} is missing. Run: powershell tools/build-icons.ps1")
    with open(path, encoding="utf-8") as f:
        return f.read()


# Reading order for the contact sheet and the docs: grouped the way the chrome
# uses them, matching the map in tools/build-icons.ps1. Names only, no geometry,
# so this is cheap to keep in step and cannot contradict what ships.
#
# Anything on disk but missing here still appears, appended alphabetically. A new
# icon therefore shows up without this list being touched; it just lands at the
# end until someone files it in the right group.
ORDER = [
    "back", "forward", "reload", "home",
    "tab", "tab-new", "tab-pin", "tab-audio", "split", "sidebar",
    "lock", "shield", "shield-off", "info", "warning", "private",
    "download", "upload", "bookmark", "history", "extensions", "menu", "more",
    "settings", "search", "zoom-in", "zoom-out", "print",
    "fullscreen", "fullscreen-exit", "maximize", "restore", "close", "minimize",
    "copy", "trash", "edit", "external", "check", "chevron-down",
    "chevron-right", "plus",
    "theme",
    "cleave",
]


def _names():
    """Every icon on disk, in ORDER, then anything ORDER did not mention."""
    if not os.path.isdir(ICON_DIR):
        return []
    on_disk = {f[:-4] for f in os.listdir(ICON_DIR) if f.endswith(".svg")}
    listed = [n for n in ORDER if n in on_disk]
    return listed + sorted(on_disk - set(listed))


ALL = _names()
