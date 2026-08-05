#!/usr/bin/env python3
"""Generate BRAND-KIT.md. Every SVG block is pulled from kit.py, so the spec and
the shipped assets cannot drift apart."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit as K


def block(svg):
    return "```svg\n" + svg.rstrip() + "\n```"


DOC = f"""# Brume Brand Identity System

**Product:** Brume, a lightweight privacy-focused web browser
**Studio:** Londev
**Version:** 2.0
**Mark:** Cleave

Every SVG here is standalone and copy-ready, built from rects and lines only.
No filters, no gradients, no raster, no font dependency. Strokes and fills use
`currentColor` by default, so one file serves every colourway.

This document is generated from `tools/kit.py`. Edit the geometry there and
re-run `tools/docs.py`; do not hand-edit the code blocks below.

---

## 1. Brand Rationale

Cleave is a square opened along a stepped cut. The cut runs down, across, then
down again, and the two pieces it leaves are congruent: rotate either one 180
degrees about the centre and you get the other. At a glance the mark is a solid
square with a hairline through it. The second look is what it actually is, which
is two things wearing one shape.

**Logic of the mark.** One primitive, one operation. The word cleave is its own
opposite, meaning both to split and to cling, and the mark does both at once.
That doubling is the argument the product makes: what looks like one continuous
record is two, and the thing between them is what is being withheld. The cut is
exactly one module wide, so it reads as a precise incision rather than a gap
something fell out of.

**Why it fits the positioning.** It is solid rather than stroked, so it gains
weight as it shrinks instead of losing it, which matters when the primary
placement is a 16px tab favicon. It is entirely orthogonal, which reads as
instrument rather than ornament. And it is quiet: nothing about it announces
itself, which is the correct posture for a tool whose whole promise is getting
out of your way.

---

## 2. Logo System

### 2.1 Construction

Canvas is 32. Live area is 4 to 28, so 24 units with a 4-unit margin. Three
numbers govern the system and all three are measured off the wordmark's typeface
rather than chosen:

| | Value | Origin |
|---|---|---|
| Module | 3 | Archivo Medium stem weight, 0.088em, measured from the `l` |
| Radius | 9 | Half its bowl, 0.55em, measured from the `o` |
| Live area | 24 | The ascender height, so mark and wordmark match in any lockup |

The cut runs `(10,4)` to `(10,16)` to `(22,16)` to `(22,28)`. Each piece is
pulled back half a module from that line, opening a 3-unit gap. Cleave has no
curves, but the radius is recorded because anything added to the system later
has to stay commensurate with it.

### 2.2 Primary Logomark

**Use for:** 32px and above. App icons, new-tab page, README headers, social
avatar, anywhere the mark stands alone.

{block(K.mark())}

### 2.3 Small-Size Redraw

**Use for:** 16px and 24px only. Browser tabs, bookmark bars, system tray,
Windows taskbar at small scale.

Below 32px the 3-unit cut starts closing under antialiasing, so the cut opens to
4 units. Every other edge is unchanged. This is a different drawing, not a
downscale.

{block(K.mark(small=True))}

### 2.4 Wordmark

**Use for:** website header, landing page footer, documentation masthead,
anywhere the name appears as a graphic rather than as running text.

Set in **Archivo Medium** (weight 500), lowercase, tracking -0.03em, converted
to outlines. Archivo is SIL OFL 1.1, free for commercial use, and available as a
single variable file. Because the wordmark ships as outlines there is no font
dependency at all: it cannot re-render differently on someone else's machine.

Lowercase belongs to the logotype only. In running prose the name is always
written **Brume**, capitalised.

{block(K.wordmark())}

### 2.5 Combination Mark, Horizontal

**Use for:** website header, GitHub social preview, presentation title, email
signature, anywhere wide and short.

Mark height equals the wordmark's ascender height. Gap is 3 modules at mark
scale.

{block(K.lockup_h())}

### 2.6 Combination Mark, Stacked

**Use for:** README header, splash or about dialog, square social avatar,
print collateral.

The mark goes to 1.5x here so it holds its own against the wordmark's width.
Vertical gap is again 3 modules at that scale.

{block(K.lockup_v())}

### 2.7 Monochrome Black (light backgrounds)

**Use for:** print, light-mode README, documentation footer, anywhere on Paper
or white. For true single-ink print, substitute `#000000`.

{block(K.lockup_h(K.INK))}

### 2.8 Monochrome White (dark backgrounds)

**Use for:** Brume's own dark chrome, dark-mode README, dark landing header,
stickers on dark stock, video end cards.

{block(K.lockup_h(K.PAPER))}

### 2.9 App Tile

**Use for:** macOS dock, Windows `.exe` and installer, Android adaptive icon,
apple-touch-icon, PWA manifest at 192 and 512.

Corner radius is 21.9% of the canvas, close enough to the platform squircle
masks everywhere. Glyph fills 62.5% of the width.

{block(K.tile())}

### 2.10 Adaptive Favicon

**Use for:** `favicon.svg`. Carries its own colour-scheme query, so it flips
between Ink and Paper without you shipping two files.

{block(K.favicon())}

**Maintenance note.** You do not need to keep every file above. Keep the five
sources in `assets/svg/` with `currentColor` and set the colour in CSS. The
hard-coded colourways exist only for contexts that strip CSS, which in practice
means GitHub README images and email.

---

## 3. Clear Space and Minimum Size

### Clear space

**Clear space is 3 modules on all four sides**, which is 9 mark-units, or
**0.375 times the rendered height of the mark**. It is three times the width of
the cut, so it stays visibly proportionate to the mark at any size.

A 32px logomark needs 12px of nothing around it. Nothing enters that space: not
a nav item, not a badge, not a version string, not the edge of a screenshot.

### Minimum reproduction sizes

| Asset | Digital minimum | Print minimum | Binding constraint |
|---|---|---|---|
| Small-size redraw | 16px | not for print | the 4-unit cut at 1px |
| Primary logomark | 32px | 8mm | the 3-unit cut at 1.5px |
| App tile | 32px | 8mm | tile radius legibility |
| Horizontal lockup | 120px wide | 32mm | wordmark x-height at 17px |
| Stacked lockup | 90px wide | 24mm | wordmark x-height at 18px |

Below 16px, use a solid `#4A5C6B` square with no cut. The cut stops being
legible before the square does, and a square with an invisible cut is not the
logo.

---

## 4. Colour Palette

Four colours. One primary, two neutrals, one accent.

### Haar (primary) `#4A5C6B`

The Scots word for the cold sea fog that rolls inland off the North Sea.
Desaturated steel blue at `hsl(207, 18%, 35%)`: the colour of daylight with the
information taken out of it. At 18% saturation it reads as weather rather than
as software signalling that it is software.

RGB 74 92 107 &middot; CMYK 31 14 0 58 &middot; dark-UI value `#9DB2C0`

### Ink (neutral, dark) `#101418`

Near-black with a cool cast so it sits in the same temperature family as Haar
rather than fighting it. Browser chrome background, body text on light.

RGB 16 20 24 &middot; CMYK 33 17 0 91 (for print specify a rich black: 60 40 40 100)

### Paper (neutral, light) `#F3F4F5`

Slightly cool off-white. Not `#FFFFFF`, which glares when a user switches from
the dark chrome.

RGB 243 244 245 &middot; CMYK 1 0 0 4 (print: leave unprinted stock)

### Lamplight (accent) `#C6A87C`

The single warm note in a cold system: a light seen through fog. Muted ochre,
not amber and not gold. Used **once per screen at most**, for the active state,
the focus ring, or a single hairline rule. If Lamplight appears twice on a page,
one of them is wrong.

RGB 198 168 124 &middot; CMYK 0 15 37 22

### Contrast reference

| Colour | On Paper | On Ink | Safe for |
|---|---|---|---|
| Ink `#101418` | 16.4:1 | n/a | body text, all sizes, AAA on light |
| Haar `#4A5C6B` | 6.5:1 | 2.8:1 | text on light (AA). **Not** on dark. |
| Haar reversed `#9DB2C0` | 2.1:1 | 8.7:1 | text on dark (AAA). **Not** on light. |
| Lamplight `#C6A87C` | 2.0:1 | 8.2:1 | text on dark (AAA). On light, **graphic use only**. |
| Paper `#F3F4F5` | n/a | 16.4:1 | body text on dark, AAA |

CMYK figures are naive conversions. Run a proper profile conversion before any
real print job and expect Haar to shift slightly green on uncoated stock.

Ship the palette as `assets/css/tokens.css` and import it everywhere.

---

## 5. Typography

The wordmark's own typeface does the entire system.

| Role | Typeface | Licence | Notes |
|---|---|---|---|
| Logotype | Archivo Medium, lowercase, -0.03em | SIL OFL 1.1 | Shipped as outlines. No font needed at runtime. |
| Product UI | Archivo | SIL OFL 1.1 | Browser chrome, settings, menus, dialogs. |
| Marketing / docs | Archivo | SIL OFL 1.1 | Same family, lighter weights for long-form. |
| Code | IBM Plex Mono | SIL OFL 1.1 | Devtools, config, README code blocks. |

**Why Archivo.** It is one variable file covering weight 100 to 900 and width 62
to 125, so display, UI and body all come out of a single download. It is a
grotesque with enough width range to set both a tight logotype and comfortable
body text, and it is not Inter. Two licences and roughly 60KB subset covers the
whole identity, which matters when the product's pitch is that it is
lightweight.

**Ship weights, not families.** Subset to 300, 400 and 500 at width 100. That is
one variable file with the axes pinned, or three static cuts if you would rather
not deal with variable fonts in the Tauri webview.

### Type scale

| Token | Size / line-height | Tracking | Weight | Use |
|---|---|---|---|---|
| `display` | 40 / 44 | -0.03em | 300 | Landing hero, one per page |
| `h1` | 28 / 34 | -0.025em | 400 | Page and doc titles |
| `h2` | 20 / 28 | -0.02em | 500 | Section heads |
| `body` | 16 / 26 | -0.005em | 400 | Running text |
| `micro` | 13 / 20 | +0.005em | 400 | Captions, footers, metadata |

### Product UI scale

Browser chrome runs tighter than marketing. Two sizes only.

| Token | Size / line-height | Use |
|---|---|---|
| `ui` | 13 / 18 | Tab titles, menu items, buttons, address bar |
| `ui-sm` | 12 / 16 | Secondary labels, keyboard hints, status text |

Negative tracking above 20px, near zero at body, slightly positive below 14px.
That is the whole rule.

---

## 6. Iconography

### 6.1 The set is Lucide

Versions 2.0 and 2.1 drew the icons here, to a house rule of 90 degrees with 45
permitted only where an object carries direction and arcs only where an object
genuinely is round. The rule produced a set that was internally consistent and
that nobody liked looking at. Squaring off things that are conventionally round
reads as deliberate on one icon and as a limitation across forty.

The set is now Lucide, under the ISC License. It is the same 24 canvas, the same
2px stroke and the same currentColor, so nothing in the browser chrome had to
change: the icons are painted as CSS masks, which care about the alpha and not
about how it was drawn.

Do not reintroduce the angle rule. It describes a set that no longer exists, and
the conformance auditor that enforced it has been removed rather than left to
fail.

### 6.2 Grid

| Property | Value |
|---|---|
| Canvas | 24 x 24 |
| Padding | at least 1 unit, per Lucide's own guide |
| Stroke weight | 2 |
| Stroke cap | round |
| Stroke join | round |
| Corner radius | 2 |
| Fill | none. Stroke-only, with one exception. |

The exception is `cleave`, which is the mark itself and is filled. `theme` used
to be a second exception, a half-filled disc; Lucide's `contrast` draws the same
idea as a stroke, so the set is now stroke-only apart from the mark.

### 6.3 The set

45 icons, enough to build the whole browser chrome.

| Group | Icons |
|---|---|
| Navigation | `back` `forward` `reload` `home` |
| Tabs | `tab` `tab-new` `tab-pin` `tab-audio` `tab-audio-muted` `split` `sidebar` |
| Address bar | `lock` `shield` `shield-off` `info` `warning` `private` |
| Toolbar | `download` `upload` `bookmark` `history` `extensions` `menu` `more` `settings` `search` `zoom-in` `zoom-out` `print` |
| Window | `fullscreen` `fullscreen-exit` `maximize` `restore` `close` `minimize` |
| Content | `copy` `trash` `edit` `external` `check` `chevron-down` `chevron-right` `plus` |
| Theme | `theme` |
| Brand | `cleave` |

There is no separate `stop`: browsers use an X for it, and `close` already is
one. Adding a `stop` would have shipped two byte-identical files.

### 6.4 Optical rules

- Never scale a 24-grid icon to 16px. Lucide's own guidance is the same: below
  24 the 2px stroke stops being 2px and the detail closes up.
- State is expressed with colour, never by adding elements. An active shield is
  a Lamplight shield, not a shield with a tick on it.

### 6.5 Changing an icon

The mapping from Brume's names to Lucide's is in `tools/build-icons.ps1`. Change
the name on the right of the map, or add a line, then:

```
powershell tools/build-icons.ps1
powershell ../tools/sync-brand-assets.ps1
```

The left column is what the app asks for by filename and must not churn.
`tools/icons.py` holds no path data and reads the generated files, so it cannot
produce an icon that differs from the one being shipped.

### 6.6 Examples

Shield.

{block(K.icon("shield"))}

Search. Feather-derived, so MIT rather than ISC; `NOTICE` lists which icons are.

{block(K.icon("search"))}

Cleave, the brand glyph. Not a UI icon: it is the mark itself, filled, mapped
from its 32 canvas onto the 24 icon grid so it sits correctly beside the set in
menus and about dialogs. It uses the small-size redraw because it renders at
icon sizes. Version 2.0 shipped a stroked approximation of the mark here, which
is why it did not look like the logo.

{block(K.icon("cleave"))}

---

## 7. Application Previews

### Browser toolbar and favicon

The logo does **not** appear in Brume's own toolbar. A mark parked permanently in
the window corner is exactly the noise this brand argues against. It appears in
three places only:

- **Tab favicon (16px).** `favicon.svg` with its own colour-scheme query, plus
  PNG fallbacks at 16, 24 and 32.
- **New-tab page.** Primary logomark at 48px, centred, roughly 120px above the
  search field, in Haar at 42% opacity on light or Paper at 55% on dark. It
  should read as a watermark, not a greeting.
- **About dialog and installer.** Stacked lockup at 140px wide, centred, version
  number below in `micro`.

Windows `.exe` and NSIS installer: `assets/ico/brume.ico`, already containing 16
through 256. The 16 and 24 entries use the small redraw, not a downscale.

### GitHub README header

Centred stacked lockup at 180px. No banner image, no hero screenshot above the
fold, no emoji.

```html
<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/svg/generated/lockup-v-paper.svg">
    <img src="assets/svg/generated/lockup-v-ink.svg" width="180" alt="Brume">
  </picture>
</p>

<p align="center"><em>A lightweight, privacy-focused browser. Built by Londev.</em></p>
```

Badges below the tagline, flat style, greyscale only. One screenshot further
down, on an Ink field, with 32px of Ink padding around it.

### One-page landing site

Single column, `max-width: 640px`, centred, Paper background, Ink text. No hero
image, no gradient, no section dividers except one hairline.

- **Header.** Horizontal lockup at 150px, left-aligned, 72px from the top. No
  nav on the first screen.
- **Hero.** `display` headline, two lines maximum. One paragraph at `body` in
  `--fg-muted`, 60 characters per line maximum. Then 40px, then one button.
- **Button.** Ink fill, Paper label, `border-radius: 3px`, padding 12px 24px,
  `ui` size at weight 500. Hover shifts the fill to Haar. Focus ring is 2px
  Lamplight at 2px offset, and that ring is the only Lamplight above the fold.
- **The rest.** Three feature blocks, `h2` plus two lines of `body`, separated
  by 96px. No icons beside them, no cards, no borders. One screenshot at roughly
  50% page depth, full-bleed on an Ink field with 64px vertical padding.
- **Footer.** One 1px hairline, 64px of space, then the logomark at 20px on the
  left and `micro` type in Haar on the right reading Londev and the year.

The test for the page: squint, and you should see four or five dark shapes on a
light field and a lot of nothing. If you see a grid of boxes, start over.

---

## 8. Do and Don't

**Do**

- Keep 3 modules of clear space, which is 0.375 times the mark's height.
- Use `currentColor` and let CSS set the colour. One file, every theme.
- Redraw below 32px rather than scaling down.
- Keep every hex code in `tokens.css` and nowhere else.
- Let the mark sit alone. It is strongest with nothing near it.

**Don't**

- **Don't close the cut, widen it, or change its step.** The cut is the mark. A
  solid square is not a simplified Cleave, it is a different logo.
- **Don't rotate it.** The two pieces are congruent under 180 degrees, so a
  rotated mark is indistinguishable from the original and simply reads as a
  mistake at 90 and 270.
- **Don't stretch or skew.** The cut only stays one module wide under uniform
  scale.
- **Don't fill the two pieces in different colours.** It is one mark that
  happens to be two shapes, not two shapes that make a mark.
- **Don't add effects.** No drop shadows, glows, gradients, blur, outer stroke,
  or frosted-glass backing. Every one of them breaks SVG portability.
- **Don't recolour outside the palette.** The mark is Ink, Paper or Haar. Never
  Lamplight, never sampled from a background photo.
- **Don't place it on photography or any busy field.** If it must go over an
  image, put it on a flat Ink or Paper block first.
- **Don't set the wordmark in a font at display size.** Place the outlined SVG.
  Retyping it in Archivo without the tracking gives a different logo that happens
  to say the same word.
- **Don't use the app tile where the bare mark belongs.** The rounded container
  is for OS icon slots only.

---

## 9. Build

Five sources, everything else generated.

```
assets/svg/          mark.svg  mark-sm.svg  wordmark.svg  lockup-h.svg  lockup-v.svg
assets/svg/generated colourways, tiles, adaptive favicon
assets/icons/        45 icons, incl. cleave.svg (the mark, filled)
assets/png/          45 rasters at every size the platforms ask for
assets/ico/          favicon.ico  brume.ico   (16 24 32 48 64 128 256 each)
assets/css/          tokens.css
tools/               kit.py  raster.py  preview.py  docs.py  wordmark.json
```

```
powershell tools/build-icons.ps1   # assets/icons/, from Lucide. Needs npm install.
python3 tools/kit.py       # the mark, from the geometry constants
python3 tools/raster.py    # PNG + ICO          (needs cairosvg, Pillow)
python3 tools/preview.py   # preview.html
python3 tools/docs.py      # this file
```

`kit.py` is the only file to edit. The wordmark travels with it as baked
outlines in `wordmark.json`, so the kit rebuilds with no font files and no
network access. Run everything through SVGO once with `removeViewBox: false`
before committing, and do not let it merge the two Cleave subpaths: they are
separate so you can address them independently later.
"""

if __name__ == "__main__":
    open(f"{K.ROOT}/BRAND-KIT.md", "w").write(DOC)
    print(f"BRAND-KIT.md: {len(DOC):,} bytes, {len(DOC.splitlines())} lines")
