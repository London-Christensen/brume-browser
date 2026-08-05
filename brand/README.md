# Brume Brand Kit v2.0

Studio: Londev. Mark: **Cleave**.

**Open `preview.html` first.** Every asset at true size, light and dark, with a
three-step construction demo that shows why the mark is built the way it is.
`BRAND-KIT.md` is the full specification.

## The one thing to know

Cleave is a square opened along a stepped cut. The two pieces are **congruent**:
rotate either 180 degrees about the centre and you get the other. That is why
the Don't list says never rotate the mark. A rotated Cleave is identical to an
unrotated one, so rotating it communicates nothing and just looks like a bug.

Three numbers govern the whole system, all measured off Archivo Medium rather
than chosen:

| | Value | Origin |
|---|---|---|
| Module | 3 | stem weight, 0.088em, from the `l`. The cut is one module wide. |
| Radius | 9 | half the bowl, 0.55em, from the `o` |
| Live area | 24 | ascender height, so mark and wordmark match in any lockup |

## What is where

```
BRAND-KIT.md              full specification, all SVG inline
preview.html              visual contact sheet, open this first

assets/svg/               THE SOURCES. Five files, currentColor.
  mark.svg                primary, 32 canvas, 3-unit cut
  mark-sm.svg             redraw for 16 and 24, cut opened to 4
  wordmark.svg            Archivo Medium, outlined
  lockup-h.svg            horizontal
  lockup-v.svg            stacked

assets/svg/generated/     colourways + tiles. Regenerate, don't hand-edit.
  favicon.svg             adaptive: follows prefers-color-scheme on its own
  tile-dark.svg           Ink field, Paper mark, for OS icon slots
assets/icons/             44 icons for the browser chrome. Lucide, ISC.
  cleave.svg              the mark itself, filled. Not a UI icon, not Lucide.
assets/png/               45 rasters, incl. apple-touch-icon / icon-192 / icon-512
assets/ico/               favicon.ico  brume.ico  (16 24 32 48 64 128 256 each)
assets/css/tokens.css     colour, type and geometry tokens
tools/                    kit.py (mark) + build-icons.ps1 (set) + 3 generators
```

## Wiring it up

```html
<link rel="stylesheet" href="assets/css/tokens.css">
<link rel="icon" href="assets/svg/generated/favicon.svg">
<link rel="icon" sizes="any" href="assets/ico/favicon.ico">
<link rel="apple-touch-icon" href="assets/png/apple-touch-icon.png">
```

```css
.logo { color: var(--fg); }   /* the SVG inherits it via currentColor */
```

Tauri:

```json
"bundle": { "icon": ["assets/ico/brume.ico", "assets/png/icon-512.png"] }
```

GitHub README, the one place you need the hard-coded colourways because GitHub
strips CSS from inline SVG:

```html
<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/svg/generated/lockup-v-paper.svg">
    <img src="assets/svg/generated/lockup-v-ink.svg" width="180" alt="Brume">
  </picture>
</p>
```

## Two rules that are easy to get wrong

1. **The favicon is a different drawing, not a downscale.** 16 and 24 use
   `mark-sm.svg`, where the cut opens from 3 units to 4 so it survives
   antialiasing. 32 and above use `mark.svg`. Both `.ico` files already do this
   internally. Keep the crossover at 32 if you export new sizes.

2. **Clear space is 3 modules**, which is `0.375 x` the rendered mark height. A
   32px mark needs 12px of nothing around it. It scales correctly at any size
   because the module scales with the mark.

## Regenerating

```
powershell tools/build-icons.ps1   # assets/icons/, from Lucide. Needs npm install.
python3 tools/kit.py               # the mark, from the geometry constants
python3 tools/raster.py            # PNG + ICO       (needs cairosvg, Pillow)
python3 tools/preview.py           # preview.html
python3 tools/docs.py              # BRAND-KIT.md
```

Run `build-icons.ps1` before `preview.py` or `docs.py`: those read the icons off
disk, so they report whatever the last icon build wrote.

### The icon set is Lucide

`tools/build-icons.ps1` holds the mapping from Brume's icon names to Lucide's and
rewrites `assets/icons/` from `node_modules/lucide-static`. To change an icon,
change the name on the right of that map and re-run it. To add one, add a line.

`icons.py` no longer holds path data; it reads the generated files. That matters
more than it looks: while the paths lived there, `kit.py` rewrote all 44 icons on
every run, so regenerating the mark would quietly have reverted the set.

Lucide is ISC, which permits redistribution as long as the copyright notice
travels with the files. Every generated icon carries it in a comment, and
`NOTICE` records it at the repository level. Some of the icons derive from
Feather and are MIT; `NOTICE` lists which and carries that notice too.

The set used to be drawn by hand here, to a rule of 90 degrees with 45 only for
direction and arcs only for genuinely round objects. There was a conformance
auditor, `tools/audit.py`, that enforced it. Both are gone: the rule does not
describe Lucide, and an auditor that parses only absolute path commands cannot
read it either.

`cleave.svg` is the exception and is still Brume's own, composed by `kit.py` from
the mark geometry.

### The mark

`tools/kit.py` holds it. The wordmark travels with it as baked outlines in
`tools/wordmark.json`, so the kit rebuilds with no font files and no network
access. `BRAND-KIT.md` is generated from the same geometry, so the spec cannot
drift from the assets.
