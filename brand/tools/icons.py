#!/usr/bin/env python3
"""Brume UI icon set.

Grid 24, live area 20 (2..22), stroke 2, butt caps, miter joins, square corners.

ANGLE RULE. Cleave is entirely right angles, and the v2.0 spec said the icons
were too. That rule does not survive a real browser: back and forward need
direction, search needs a lens. The rule is therefore:

  90 degrees   preferred, and the default for anything box-like
  45 degrees   permitted only where the object carries direction
               (arrows, chevrons, checks, the pencil, the bookmark notch)
  arcs         permitted only where the object genuinely is round
               (lens, clock, lock shackle, info, theme)
  nothing else. No 30s, no 60s, no arbitrary slopes.

Where a glyph is conventionally round but does not have to be, it is squared off
instead: settings is sliders rather than a gear, extensions is a module grid
rather than a puzzle piece, private is a mask rather than a hat. That is where
the set gets its character.

Each entry is a list of (d, mode) where mode is "s" for stroked or "f" for
filled. Filled is the exception, used only by the brand glyph and the theme
toggle's half-disc.
"""

# ---------------------------------------------------------------- geometry
GRID, LIVE, STROKE = 24, 20, 2
LENS_R, DISC_R = 7, 9          # 9 is the system radius scaled to this grid

_S = lambda *ds: [(d, "s") for d in ds]

ICONS = {
# --- navigation ------------------------------------------------------------
"back":        _S("M21 12H3", "M10 5L3 12L10 19"),
"forward":     _S("M3 12H21", "M14 5L21 12L14 19"),
"reload":      _S("M12 7A7 7 0 1 1 5 14", "M8 3L12 7L8 11"),
"home":        _S("M3 12L12 3L21 12V21H3Z"),

# --- tabs ------------------------------------------------------------------
"tab":         _S("M3 4H21V20H3Z", "M3 9H21", "M8 4V9"),
"tab-new":     _S("M3 4H21V20H3Z", "M3 9H21", "M8 4V9", "M13 14H19", "M16 11V17"),
"tab-pin":     _S("M8 3H16V9H19V13H5V9H8Z", "M12 13V21"),
"tab-audio":   _S("M3 9H8L13 4V20L8 15H3Z", "M17 9L21 13", "M21 9L17 13"),
"split":       _S("M3 4H21V20H3Z", "M12 4V20"),
"sidebar":     _S("M3 4H21V20H3Z", "M9 4V20"),

# --- address bar and security ----------------------------------------------
"lock":        _S("M4 11H20V21H4Z", "M8 11V8A4 4 0 0 1 16 8V11"),
"shield":      _S("M3 4H21V12L12 21L3 12Z"),
"shield-off":  _S("M3 4H21V12L12 21L3 12Z", "M4 4L20 20"),
"info":        _S("M12 3A9 9 0 1 1 12 21A9 9 0 1 1 12 3Z", "M12 11V17", "M12 7V8.5"),
"warning":     _S("M12 3L21 12L12 21L3 12Z", "M12 8V13", "M12 15.5V17"),
"private":     _S("M3 10H10V17H3Z", "M14 10H21V17H14Z", "M10 13H14"),

# --- toolbar actions --------------------------------------------------------
"download":    _S("M12 3V16", "M6 11L12 17L18 11", "M4 21H20"),
"upload":      _S("M12 21V8", "M6 13L12 7L18 13", "M4 3H20"),
"bookmark":    _S("M5 3H19V21L12 14L5 21Z"),
"history":     _S("M12 3A9 9 0 1 1 12 21A9 9 0 1 1 12 3Z", "M12 7V12H17"),
"extensions":  _S("M3 3H11V11H3Z", "M13 3H21V11H13Z", "M3 13H11V21H3Z", "M13 13H21V21H13Z"),
"menu":        _S("M3 7H21", "M3 12H21", "M3 17H21"),
"more":        _S("M12 4V6", "M12 11V13", "M12 18V20"),
"settings":    _S("M4 7H20", "M4 12H20", "M4 17H20", "M9 5V9", "M15 10V14", "M7 15V19"),
"search":      _S("M11 4A7 7 0 1 1 11 18A7 7 0 1 1 11 4Z", "M16 16L21 21"),
"zoom-in":     _S("M11 4A7 7 0 1 1 11 18A7 7 0 1 1 11 4Z", "M16 16L21 21", "M8 11H14", "M11 8V14"),
"zoom-out":    _S("M11 4A7 7 0 1 1 11 18A7 7 0 1 1 11 4Z", "M16 16L21 21", "M8 11H14"),
"print":       _S("M4 8H20V16H4Z", "M7 8V3H17V8", "M7 21V12H17V21Z"),

# --- window -----------------------------------------------------------------
"fullscreen":      _S("M3 9V3H9", "M15 3H21V9", "M21 15V21H15", "M9 21H3V15"),
"fullscreen-exit": _S("M9 3V9H3", "M21 9H15V3", "M15 21V15H21", "M3 15H9V21"),
"maximize":    _S("M4 4H20V20H4Z"),
"restore":     _S("M7 7H20V20H7Z", "M4 16V4H16"),
"close":       _S("M5 5L19 19", "M19 5L5 19"),
"minimize":    _S("M4 12H20"),

# --- content ----------------------------------------------------------------
"copy":        _S("M9 9H20V20H9Z", "M15 9V4H4V15H9"),
"trash":       _S("M4 7H20", "M9 7V4H15V7", "M6 7V21H18V7", "M10 11V17", "M14 11V17"),
"edit":        _S("M4 20V16L16 4L20 8L8 20Z"),
"external":    _S("M12 4H4V20H20V12", "M14 3H21V10", "M21 3L12 12"),
"check":       _S("M4 12L10 18L20 8"),
"chevron-down":  _S("M5 9L12 16L19 9"),
"chevron-right": _S("M9 5L16 12L9 19"),
"plus":        _S("M12 4V20", "M4 12H20"),

# --- theme ------------------------------------------------------------------
"theme":       [("M12 3A9 9 0 1 1 12 21A9 9 0 1 1 12 3Z", "s"),
                ("M12 3A9 9 0 0 0 12 21Z", "f")],
}

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
    if name == "cleave":
        body = "  " + brand_glyph(cleave_sm)
    else:
        body = "\n".join(
            f'  <path d="{d}"/>' if mode == "s"
            else f'  <path fill="currentColor" stroke="none" d="{d}"/>'
            for d, mode in ICONS[name])
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" '
            'stroke="currentColor" stroke-width="2" stroke-linecap="butt" '
            f'stroke-linejoin="miter" role="img" aria-label="{name}">\n{body}\n</svg>\n')


ALL = list(ICONS) + ["cleave"]
