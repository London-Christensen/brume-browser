# Brume Brand Identity System

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

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" fill="currentColor" role="img" aria-label="Brume">
  <path d="M4 4H8.5V17.5H20.5V28H4Z M11.5 4H28V28H23.5V14.5H11.5Z"/>
</svg>
```

### 2.3 Small-Size Redraw

**Use for:** 16px and 24px only. Browser tabs, bookmark bars, system tray,
Windows taskbar at small scale.

Below 32px the 3-unit cut starts closing under antialiasing, so the cut opens to
4 units. Every other edge is unchanged. This is a different drawing, not a
downscale.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" fill="currentColor" role="img" aria-label="Brume">
  <path d="M4 4H8V18H20V28H4Z M12 4H28V28H24V14H12Z"/>
</svg>
```

### 2.4 Wordmark

**Use for:** website header, landing page footer, documentation masthead,
anywhere the name appears as a graphic rather than as running text.

Set in **Archivo Medium** (weight 500), lowercase, tracking -0.03em, converted
to outlines. Archivo is SIL OFL 1.1, free for commercial use, and available as a
single variable file. Because the wordmark ships as outlines there is no font
dependency at all: it cannot re-render differently on someone else's machine.

Lowercase belongs to the logotype only. In running prose the name is always
written **Brume**, capitalised.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="6.9 -72.3 264.0 73.5" fill="currentColor" role="img" aria-label="Brume">
  <path d="M31.90 1.20Q26.60 1.20 22.40 -0.95Q18.20 -3.10 15.50 -7.50H14.80L13.90 0.00H6.90V-72.30H15.70V-46.00H16.30Q18.10 -48.80 20.50 -50.50Q22.90 -52.20 25.90 -53.00Q28.90 -53.80 32.30 -53.80Q38.40 -53.80 43.00 -50.90Q47.60 -48.00 50.15 -42.00Q52.70 -36.00 52.70 -26.70Q52.70 -16.90 50.20 -10.75Q47.70 -4.60 43.10 -1.70Q38.50 1.20 31.90 1.20ZM29.80 -6.50Q34.30 -6.50 37.35 -8.30Q40.40 -10.10 42.00 -14.25Q43.60 -18.40 43.60 -25.20V-27.30Q43.60 -33.90 42.10 -38.05Q40.60 -42.20 37.50 -44.15Q34.40 -46.10 29.50 -46.10Q26.70 -46.10 24.20 -45.20Q21.70 -44.30 19.75 -42.10Q17.80 -39.90 16.75 -36.20Q15.70 -32.50 15.70 -27.10V-25.50Q15.70 -19.30 17.15 -15.10Q18.60 -10.90 21.75 -8.70Q24.90 -6.50 29.80 -6.50Z M60.60 0.00V-52.60H67.70L68.50 -43.90H69.20Q70.00 -46.40 71.40 -48.65Q72.80 -50.90 75.20 -52.35Q77.60 -53.80 81.10 -53.80Q82.60 -53.80 83.85 -53.55Q85.10 -53.30 85.70 -53.00V-44.90H82.40Q79.00 -44.90 76.55 -43.75Q74.10 -42.60 72.50 -40.50Q70.90 -38.40 70.15 -35.50Q69.40 -32.60 69.40 -29.30V0.00Z M107.10 1.20Q99.50 1.20 94.90 -2.70Q90.30 -6.60 90.30 -16.30V-52.60H99.10V-17.50Q99.10 -14.20 99.90 -12.10Q100.70 -10.00 102.20 -8.80Q103.70 -7.60 105.75 -7.10Q107.80 -6.60 110.20 -6.60Q114.00 -6.60 117.20 -8.40Q120.40 -10.20 122.45 -13.65Q124.50 -17.10 124.50 -21.70V-52.60H133.30V0.00H126.20L125.40 -7.90H124.70Q122.60 -4.70 119.95 -2.70Q117.30 -0.70 114.05 0.25Q110.80 1.20 107.10 1.20Z M144.00 0.00V-52.60H151.10L151.90 -44.70H152.60Q154.60 -47.90 157.10 -49.90Q159.60 -51.90 162.60 -52.85Q165.60 -53.80 169.10 -53.80Q174.20 -53.80 177.90 -51.75Q181.60 -49.70 183.40 -44.70H184.00Q185.90 -47.90 188.40 -49.85Q190.90 -51.80 194.00 -52.80Q197.10 -53.80 200.60 -53.80Q205.40 -53.80 209.00 -52.15Q212.60 -50.50 214.65 -46.75Q216.70 -43.00 216.70 -36.70V0.00H207.90V-35.30Q207.90 -38.50 207.15 -40.60Q206.40 -42.70 205.00 -43.85Q203.60 -45.00 201.75 -45.50Q199.90 -46.00 197.80 -46.00Q194.30 -46.00 191.35 -44.20Q188.40 -42.40 186.60 -39.00Q184.80 -35.60 184.80 -30.90V0.00H176.00V-35.30Q176.00 -38.50 175.20 -40.60Q174.40 -42.70 173.10 -43.85Q171.80 -45.00 169.95 -45.50Q168.10 -46.00 166.10 -46.00Q162.50 -46.00 159.45 -44.20Q156.40 -42.40 154.60 -39.00Q152.80 -35.60 152.80 -30.90V0.00Z M248.00 1.20Q240.20 1.20 234.85 -1.75Q229.50 -4.70 226.75 -10.80Q224.00 -16.90 224.00 -26.30Q224.00 -35.80 226.75 -41.85Q229.50 -47.90 234.90 -50.85Q240.30 -53.80 248.40 -53.80Q255.80 -53.80 260.80 -50.95Q265.80 -48.10 268.35 -42.45Q270.90 -36.80 270.90 -28.30V-24.10H233.10Q233.30 -17.80 234.95 -13.75Q236.60 -9.70 239.90 -7.85Q243.20 -6.00 248.20 -6.00Q251.60 -6.00 254.15 -6.85Q256.70 -7.70 258.45 -9.30Q260.20 -10.90 261.10 -13.10Q262.00 -15.30 262.10 -17.90H270.70Q270.60 -13.70 269.10 -10.15Q267.60 -6.60 264.70 -4.10Q261.80 -1.60 257.60 -0.20Q253.40 1.20 248.00 1.20ZM233.30 -30.70H261.80Q261.80 -35.10 260.80 -38.10Q259.80 -41.10 257.95 -43.00Q256.10 -44.90 253.65 -45.75Q251.20 -46.60 248.10 -46.60Q243.50 -46.60 240.30 -44.90Q237.10 -43.20 235.40 -39.70Q233.70 -36.20 233.30 -30.70Z"/>
</svg>
```

### 2.5 Combination Mark, Horizontal

**Use for:** website header, GitHub social preview, presentation title, email
signature, anywhere wide and short.

Mark height equals the wordmark's ascender height. Gap is 3 modules at mark
scale.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 365.06 73.50" fill="currentColor" role="img" aria-label="Brume">
  <g transform="translate(-12.2500 -12.2500) scale(3.06250)"><path d="M4 4H8.5V17.5H20.5V28H4Z M11.5 4H28V28H23.5V14.5H11.5Z"/></g>
  <g transform="translate(94.16 72.30)"><path d="M31.90 1.20Q26.60 1.20 22.40 -0.95Q18.20 -3.10 15.50 -7.50H14.80L13.90 0.00H6.90V-72.30H15.70V-46.00H16.30Q18.10 -48.80 20.50 -50.50Q22.90 -52.20 25.90 -53.00Q28.90 -53.80 32.30 -53.80Q38.40 -53.80 43.00 -50.90Q47.60 -48.00 50.15 -42.00Q52.70 -36.00 52.70 -26.70Q52.70 -16.90 50.20 -10.75Q47.70 -4.60 43.10 -1.70Q38.50 1.20 31.90 1.20ZM29.80 -6.50Q34.30 -6.50 37.35 -8.30Q40.40 -10.10 42.00 -14.25Q43.60 -18.40 43.60 -25.20V-27.30Q43.60 -33.90 42.10 -38.05Q40.60 -42.20 37.50 -44.15Q34.40 -46.10 29.50 -46.10Q26.70 -46.10 24.20 -45.20Q21.70 -44.30 19.75 -42.10Q17.80 -39.90 16.75 -36.20Q15.70 -32.50 15.70 -27.10V-25.50Q15.70 -19.30 17.15 -15.10Q18.60 -10.90 21.75 -8.70Q24.90 -6.50 29.80 -6.50Z M60.60 0.00V-52.60H67.70L68.50 -43.90H69.20Q70.00 -46.40 71.40 -48.65Q72.80 -50.90 75.20 -52.35Q77.60 -53.80 81.10 -53.80Q82.60 -53.80 83.85 -53.55Q85.10 -53.30 85.70 -53.00V-44.90H82.40Q79.00 -44.90 76.55 -43.75Q74.10 -42.60 72.50 -40.50Q70.90 -38.40 70.15 -35.50Q69.40 -32.60 69.40 -29.30V0.00Z M107.10 1.20Q99.50 1.20 94.90 -2.70Q90.30 -6.60 90.30 -16.30V-52.60H99.10V-17.50Q99.10 -14.20 99.90 -12.10Q100.70 -10.00 102.20 -8.80Q103.70 -7.60 105.75 -7.10Q107.80 -6.60 110.20 -6.60Q114.00 -6.60 117.20 -8.40Q120.40 -10.20 122.45 -13.65Q124.50 -17.10 124.50 -21.70V-52.60H133.30V0.00H126.20L125.40 -7.90H124.70Q122.60 -4.70 119.95 -2.70Q117.30 -0.70 114.05 0.25Q110.80 1.20 107.10 1.20Z M144.00 0.00V-52.60H151.10L151.90 -44.70H152.60Q154.60 -47.90 157.10 -49.90Q159.60 -51.90 162.60 -52.85Q165.60 -53.80 169.10 -53.80Q174.20 -53.80 177.90 -51.75Q181.60 -49.70 183.40 -44.70H184.00Q185.90 -47.90 188.40 -49.85Q190.90 -51.80 194.00 -52.80Q197.10 -53.80 200.60 -53.80Q205.40 -53.80 209.00 -52.15Q212.60 -50.50 214.65 -46.75Q216.70 -43.00 216.70 -36.70V0.00H207.90V-35.30Q207.90 -38.50 207.15 -40.60Q206.40 -42.70 205.00 -43.85Q203.60 -45.00 201.75 -45.50Q199.90 -46.00 197.80 -46.00Q194.30 -46.00 191.35 -44.20Q188.40 -42.40 186.60 -39.00Q184.80 -35.60 184.80 -30.90V0.00H176.00V-35.30Q176.00 -38.50 175.20 -40.60Q174.40 -42.70 173.10 -43.85Q171.80 -45.00 169.95 -45.50Q168.10 -46.00 166.10 -46.00Q162.50 -46.00 159.45 -44.20Q156.40 -42.40 154.60 -39.00Q152.80 -35.60 152.80 -30.90V0.00Z M248.00 1.20Q240.20 1.20 234.85 -1.75Q229.50 -4.70 226.75 -10.80Q224.00 -16.90 224.00 -26.30Q224.00 -35.80 226.75 -41.85Q229.50 -47.90 234.90 -50.85Q240.30 -53.80 248.40 -53.80Q255.80 -53.80 260.80 -50.95Q265.80 -48.10 268.35 -42.45Q270.90 -36.80 270.90 -28.30V-24.10H233.10Q233.30 -17.80 234.95 -13.75Q236.60 -9.70 239.90 -7.85Q243.20 -6.00 248.20 -6.00Q251.60 -6.00 254.15 -6.85Q256.70 -7.70 258.45 -9.30Q260.20 -10.90 261.10 -13.10Q262.00 -15.30 262.10 -17.90H270.70Q270.60 -13.70 269.10 -10.15Q267.60 -6.60 264.70 -4.10Q261.80 -1.60 257.60 -0.20Q253.40 1.20 248.00 1.20ZM233.30 -30.70H261.80Q261.80 -35.10 260.80 -38.10Q259.80 -41.10 257.95 -43.00Q256.10 -44.90 253.65 -45.75Q251.20 -46.60 248.10 -46.60Q243.50 -46.60 240.30 -44.90Q237.10 -43.20 235.40 -39.70Q233.70 -36.20 233.30 -30.70Z"/></g>
</svg>
```

### 2.6 Combination Mark, Stacked

**Use for:** README header, splash or about dialog, square social avatar,
print collateral.

The mark goes to 1.5x here so it holds its own against the wordmark's width.
Vertical gap is again 3 modules at that scale.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 264.00 225.09" fill="currentColor" role="img" aria-label="Brume">
  <g transform="translate(58.5000 -18.3750) scale(4.59375)"><path d="M4 4H8.5V17.5H20.5V28H4Z M11.5 4H28V28H23.5V14.5H11.5Z"/></g>
  <g transform="translate(-6.90 223.89)"><path d="M31.90 1.20Q26.60 1.20 22.40 -0.95Q18.20 -3.10 15.50 -7.50H14.80L13.90 0.00H6.90V-72.30H15.70V-46.00H16.30Q18.10 -48.80 20.50 -50.50Q22.90 -52.20 25.90 -53.00Q28.90 -53.80 32.30 -53.80Q38.40 -53.80 43.00 -50.90Q47.60 -48.00 50.15 -42.00Q52.70 -36.00 52.70 -26.70Q52.70 -16.90 50.20 -10.75Q47.70 -4.60 43.10 -1.70Q38.50 1.20 31.90 1.20ZM29.80 -6.50Q34.30 -6.50 37.35 -8.30Q40.40 -10.10 42.00 -14.25Q43.60 -18.40 43.60 -25.20V-27.30Q43.60 -33.90 42.10 -38.05Q40.60 -42.20 37.50 -44.15Q34.40 -46.10 29.50 -46.10Q26.70 -46.10 24.20 -45.20Q21.70 -44.30 19.75 -42.10Q17.80 -39.90 16.75 -36.20Q15.70 -32.50 15.70 -27.10V-25.50Q15.70 -19.30 17.15 -15.10Q18.60 -10.90 21.75 -8.70Q24.90 -6.50 29.80 -6.50Z M60.60 0.00V-52.60H67.70L68.50 -43.90H69.20Q70.00 -46.40 71.40 -48.65Q72.80 -50.90 75.20 -52.35Q77.60 -53.80 81.10 -53.80Q82.60 -53.80 83.85 -53.55Q85.10 -53.30 85.70 -53.00V-44.90H82.40Q79.00 -44.90 76.55 -43.75Q74.10 -42.60 72.50 -40.50Q70.90 -38.40 70.15 -35.50Q69.40 -32.60 69.40 -29.30V0.00Z M107.10 1.20Q99.50 1.20 94.90 -2.70Q90.30 -6.60 90.30 -16.30V-52.60H99.10V-17.50Q99.10 -14.20 99.90 -12.10Q100.70 -10.00 102.20 -8.80Q103.70 -7.60 105.75 -7.10Q107.80 -6.60 110.20 -6.60Q114.00 -6.60 117.20 -8.40Q120.40 -10.20 122.45 -13.65Q124.50 -17.10 124.50 -21.70V-52.60H133.30V0.00H126.20L125.40 -7.90H124.70Q122.60 -4.70 119.95 -2.70Q117.30 -0.70 114.05 0.25Q110.80 1.20 107.10 1.20Z M144.00 0.00V-52.60H151.10L151.90 -44.70H152.60Q154.60 -47.90 157.10 -49.90Q159.60 -51.90 162.60 -52.85Q165.60 -53.80 169.10 -53.80Q174.20 -53.80 177.90 -51.75Q181.60 -49.70 183.40 -44.70H184.00Q185.90 -47.90 188.40 -49.85Q190.90 -51.80 194.00 -52.80Q197.10 -53.80 200.60 -53.80Q205.40 -53.80 209.00 -52.15Q212.60 -50.50 214.65 -46.75Q216.70 -43.00 216.70 -36.70V0.00H207.90V-35.30Q207.90 -38.50 207.15 -40.60Q206.40 -42.70 205.00 -43.85Q203.60 -45.00 201.75 -45.50Q199.90 -46.00 197.80 -46.00Q194.30 -46.00 191.35 -44.20Q188.40 -42.40 186.60 -39.00Q184.80 -35.60 184.80 -30.90V0.00H176.00V-35.30Q176.00 -38.50 175.20 -40.60Q174.40 -42.70 173.10 -43.85Q171.80 -45.00 169.95 -45.50Q168.10 -46.00 166.10 -46.00Q162.50 -46.00 159.45 -44.20Q156.40 -42.40 154.60 -39.00Q152.80 -35.60 152.80 -30.90V0.00Z M248.00 1.20Q240.20 1.20 234.85 -1.75Q229.50 -4.70 226.75 -10.80Q224.00 -16.90 224.00 -26.30Q224.00 -35.80 226.75 -41.85Q229.50 -47.90 234.90 -50.85Q240.30 -53.80 248.40 -53.80Q255.80 -53.80 260.80 -50.95Q265.80 -48.10 268.35 -42.45Q270.90 -36.80 270.90 -28.30V-24.10H233.10Q233.30 -17.80 234.95 -13.75Q236.60 -9.70 239.90 -7.85Q243.20 -6.00 248.20 -6.00Q251.60 -6.00 254.15 -6.85Q256.70 -7.70 258.45 -9.30Q260.20 -10.90 261.10 -13.10Q262.00 -15.30 262.10 -17.90H270.70Q270.60 -13.70 269.10 -10.15Q267.60 -6.60 264.70 -4.10Q261.80 -1.60 257.60 -0.20Q253.40 1.20 248.00 1.20ZM233.30 -30.70H261.80Q261.80 -35.10 260.80 -38.10Q259.80 -41.10 257.95 -43.00Q256.10 -44.90 253.65 -45.75Q251.20 -46.60 248.10 -46.60Q243.50 -46.60 240.30 -44.90Q237.10 -43.20 235.40 -39.70Q233.70 -36.20 233.30 -30.70Z"/></g>
</svg>
```

### 2.7 Monochrome Black (light backgrounds)

**Use for:** print, light-mode README, documentation footer, anywhere on Paper
or white. For true single-ink print, substitute `#000000`.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 365.06 73.50" fill="#101418" role="img" aria-label="Brume">
  <g transform="translate(-12.2500 -12.2500) scale(3.06250)"><path d="M4 4H8.5V17.5H20.5V28H4Z M11.5 4H28V28H23.5V14.5H11.5Z"/></g>
  <g transform="translate(94.16 72.30)"><path d="M31.90 1.20Q26.60 1.20 22.40 -0.95Q18.20 -3.10 15.50 -7.50H14.80L13.90 0.00H6.90V-72.30H15.70V-46.00H16.30Q18.10 -48.80 20.50 -50.50Q22.90 -52.20 25.90 -53.00Q28.90 -53.80 32.30 -53.80Q38.40 -53.80 43.00 -50.90Q47.60 -48.00 50.15 -42.00Q52.70 -36.00 52.70 -26.70Q52.70 -16.90 50.20 -10.75Q47.70 -4.60 43.10 -1.70Q38.50 1.20 31.90 1.20ZM29.80 -6.50Q34.30 -6.50 37.35 -8.30Q40.40 -10.10 42.00 -14.25Q43.60 -18.40 43.60 -25.20V-27.30Q43.60 -33.90 42.10 -38.05Q40.60 -42.20 37.50 -44.15Q34.40 -46.10 29.50 -46.10Q26.70 -46.10 24.20 -45.20Q21.70 -44.30 19.75 -42.10Q17.80 -39.90 16.75 -36.20Q15.70 -32.50 15.70 -27.10V-25.50Q15.70 -19.30 17.15 -15.10Q18.60 -10.90 21.75 -8.70Q24.90 -6.50 29.80 -6.50Z M60.60 0.00V-52.60H67.70L68.50 -43.90H69.20Q70.00 -46.40 71.40 -48.65Q72.80 -50.90 75.20 -52.35Q77.60 -53.80 81.10 -53.80Q82.60 -53.80 83.85 -53.55Q85.10 -53.30 85.70 -53.00V-44.90H82.40Q79.00 -44.90 76.55 -43.75Q74.10 -42.60 72.50 -40.50Q70.90 -38.40 70.15 -35.50Q69.40 -32.60 69.40 -29.30V0.00Z M107.10 1.20Q99.50 1.20 94.90 -2.70Q90.30 -6.60 90.30 -16.30V-52.60H99.10V-17.50Q99.10 -14.20 99.90 -12.10Q100.70 -10.00 102.20 -8.80Q103.70 -7.60 105.75 -7.10Q107.80 -6.60 110.20 -6.60Q114.00 -6.60 117.20 -8.40Q120.40 -10.20 122.45 -13.65Q124.50 -17.10 124.50 -21.70V-52.60H133.30V0.00H126.20L125.40 -7.90H124.70Q122.60 -4.70 119.95 -2.70Q117.30 -0.70 114.05 0.25Q110.80 1.20 107.10 1.20Z M144.00 0.00V-52.60H151.10L151.90 -44.70H152.60Q154.60 -47.90 157.10 -49.90Q159.60 -51.90 162.60 -52.85Q165.60 -53.80 169.10 -53.80Q174.20 -53.80 177.90 -51.75Q181.60 -49.70 183.40 -44.70H184.00Q185.90 -47.90 188.40 -49.85Q190.90 -51.80 194.00 -52.80Q197.10 -53.80 200.60 -53.80Q205.40 -53.80 209.00 -52.15Q212.60 -50.50 214.65 -46.75Q216.70 -43.00 216.70 -36.70V0.00H207.90V-35.30Q207.90 -38.50 207.15 -40.60Q206.40 -42.70 205.00 -43.85Q203.60 -45.00 201.75 -45.50Q199.90 -46.00 197.80 -46.00Q194.30 -46.00 191.35 -44.20Q188.40 -42.40 186.60 -39.00Q184.80 -35.60 184.80 -30.90V0.00H176.00V-35.30Q176.00 -38.50 175.20 -40.60Q174.40 -42.70 173.10 -43.85Q171.80 -45.00 169.95 -45.50Q168.10 -46.00 166.10 -46.00Q162.50 -46.00 159.45 -44.20Q156.40 -42.40 154.60 -39.00Q152.80 -35.60 152.80 -30.90V0.00Z M248.00 1.20Q240.20 1.20 234.85 -1.75Q229.50 -4.70 226.75 -10.80Q224.00 -16.90 224.00 -26.30Q224.00 -35.80 226.75 -41.85Q229.50 -47.90 234.90 -50.85Q240.30 -53.80 248.40 -53.80Q255.80 -53.80 260.80 -50.95Q265.80 -48.10 268.35 -42.45Q270.90 -36.80 270.90 -28.30V-24.10H233.10Q233.30 -17.80 234.95 -13.75Q236.60 -9.70 239.90 -7.85Q243.20 -6.00 248.20 -6.00Q251.60 -6.00 254.15 -6.85Q256.70 -7.70 258.45 -9.30Q260.20 -10.90 261.10 -13.10Q262.00 -15.30 262.10 -17.90H270.70Q270.60 -13.70 269.10 -10.15Q267.60 -6.60 264.70 -4.10Q261.80 -1.60 257.60 -0.20Q253.40 1.20 248.00 1.20ZM233.30 -30.70H261.80Q261.80 -35.10 260.80 -38.10Q259.80 -41.10 257.95 -43.00Q256.10 -44.90 253.65 -45.75Q251.20 -46.60 248.10 -46.60Q243.50 -46.60 240.30 -44.90Q237.10 -43.20 235.40 -39.70Q233.70 -36.20 233.30 -30.70Z"/></g>
</svg>
```

### 2.8 Monochrome White (dark backgrounds)

**Use for:** Brume's own dark chrome, dark-mode README, dark landing header,
stickers on dark stock, video end cards.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 365.06 73.50" fill="#F3F4F5" role="img" aria-label="Brume">
  <g transform="translate(-12.2500 -12.2500) scale(3.06250)"><path d="M4 4H8.5V17.5H20.5V28H4Z M11.5 4H28V28H23.5V14.5H11.5Z"/></g>
  <g transform="translate(94.16 72.30)"><path d="M31.90 1.20Q26.60 1.20 22.40 -0.95Q18.20 -3.10 15.50 -7.50H14.80L13.90 0.00H6.90V-72.30H15.70V-46.00H16.30Q18.10 -48.80 20.50 -50.50Q22.90 -52.20 25.90 -53.00Q28.90 -53.80 32.30 -53.80Q38.40 -53.80 43.00 -50.90Q47.60 -48.00 50.15 -42.00Q52.70 -36.00 52.70 -26.70Q52.70 -16.90 50.20 -10.75Q47.70 -4.60 43.10 -1.70Q38.50 1.20 31.90 1.20ZM29.80 -6.50Q34.30 -6.50 37.35 -8.30Q40.40 -10.10 42.00 -14.25Q43.60 -18.40 43.60 -25.20V-27.30Q43.60 -33.90 42.10 -38.05Q40.60 -42.20 37.50 -44.15Q34.40 -46.10 29.50 -46.10Q26.70 -46.10 24.20 -45.20Q21.70 -44.30 19.75 -42.10Q17.80 -39.90 16.75 -36.20Q15.70 -32.50 15.70 -27.10V-25.50Q15.70 -19.30 17.15 -15.10Q18.60 -10.90 21.75 -8.70Q24.90 -6.50 29.80 -6.50Z M60.60 0.00V-52.60H67.70L68.50 -43.90H69.20Q70.00 -46.40 71.40 -48.65Q72.80 -50.90 75.20 -52.35Q77.60 -53.80 81.10 -53.80Q82.60 -53.80 83.85 -53.55Q85.10 -53.30 85.70 -53.00V-44.90H82.40Q79.00 -44.90 76.55 -43.75Q74.10 -42.60 72.50 -40.50Q70.90 -38.40 70.15 -35.50Q69.40 -32.60 69.40 -29.30V0.00Z M107.10 1.20Q99.50 1.20 94.90 -2.70Q90.30 -6.60 90.30 -16.30V-52.60H99.10V-17.50Q99.10 -14.20 99.90 -12.10Q100.70 -10.00 102.20 -8.80Q103.70 -7.60 105.75 -7.10Q107.80 -6.60 110.20 -6.60Q114.00 -6.60 117.20 -8.40Q120.40 -10.20 122.45 -13.65Q124.50 -17.10 124.50 -21.70V-52.60H133.30V0.00H126.20L125.40 -7.90H124.70Q122.60 -4.70 119.95 -2.70Q117.30 -0.70 114.05 0.25Q110.80 1.20 107.10 1.20Z M144.00 0.00V-52.60H151.10L151.90 -44.70H152.60Q154.60 -47.90 157.10 -49.90Q159.60 -51.90 162.60 -52.85Q165.60 -53.80 169.10 -53.80Q174.20 -53.80 177.90 -51.75Q181.60 -49.70 183.40 -44.70H184.00Q185.90 -47.90 188.40 -49.85Q190.90 -51.80 194.00 -52.80Q197.10 -53.80 200.60 -53.80Q205.40 -53.80 209.00 -52.15Q212.60 -50.50 214.65 -46.75Q216.70 -43.00 216.70 -36.70V0.00H207.90V-35.30Q207.90 -38.50 207.15 -40.60Q206.40 -42.70 205.00 -43.85Q203.60 -45.00 201.75 -45.50Q199.90 -46.00 197.80 -46.00Q194.30 -46.00 191.35 -44.20Q188.40 -42.40 186.60 -39.00Q184.80 -35.60 184.80 -30.90V0.00H176.00V-35.30Q176.00 -38.50 175.20 -40.60Q174.40 -42.70 173.10 -43.85Q171.80 -45.00 169.95 -45.50Q168.10 -46.00 166.10 -46.00Q162.50 -46.00 159.45 -44.20Q156.40 -42.40 154.60 -39.00Q152.80 -35.60 152.80 -30.90V0.00Z M248.00 1.20Q240.20 1.20 234.85 -1.75Q229.50 -4.70 226.75 -10.80Q224.00 -16.90 224.00 -26.30Q224.00 -35.80 226.75 -41.85Q229.50 -47.90 234.90 -50.85Q240.30 -53.80 248.40 -53.80Q255.80 -53.80 260.80 -50.95Q265.80 -48.10 268.35 -42.45Q270.90 -36.80 270.90 -28.30V-24.10H233.10Q233.30 -17.80 234.95 -13.75Q236.60 -9.70 239.90 -7.85Q243.20 -6.00 248.20 -6.00Q251.60 -6.00 254.15 -6.85Q256.70 -7.70 258.45 -9.30Q260.20 -10.90 261.10 -13.10Q262.00 -15.30 262.10 -17.90H270.70Q270.60 -13.70 269.10 -10.15Q267.60 -6.60 264.70 -4.10Q261.80 -1.60 257.60 -0.20Q253.40 1.20 248.00 1.20ZM233.30 -30.70H261.80Q261.80 -35.10 260.80 -38.10Q259.80 -41.10 257.95 -43.00Q256.10 -44.90 253.65 -45.75Q251.20 -46.60 248.10 -46.60Q243.50 -46.60 240.30 -44.90Q237.10 -43.20 235.40 -39.70Q233.70 -36.20 233.30 -30.70Z"/></g>
</svg>
```

### 2.9 App Tile

**Use for:** macOS dock, Windows `.exe` and installer, Android adaptive icon,
apple-touch-icon, PWA manifest at 192 and 512.

Corner radius is 21.9% of the canvas, close enough to the platform squircle
masks everywhere. Glyph fills 62.5% of the width.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none" role="img" aria-label="Brume">
  <rect width="64" height="64" rx="14" fill="#101418"/>
  <g transform="translate(5.333 5.333) scale(1.66667)" fill="#F3F4F5"><path d="M4 4H8.5V17.5H20.5V28H4Z M11.5 4H28V28H23.5V14.5H11.5Z"/></g>
</svg>
```

### 2.10 Adaptive Favicon

**Use for:** `favicon.svg`. Carries its own colour-scheme query, so it flips
between Ink and Paper without you shipping two files.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="Brume">
  <style>
    path { fill: #101418; }
    @media (prefers-color-scheme: dark) { path { fill: #F3F4F5; } }
  </style>
  <path d="M4 4H8V18H20V28H4Z M12 4H28V28H24V14H12Z"/>
</svg>
```

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

### 6.1 The angle rule

Version 2.0 said the icon set was entirely orthogonal, because Cleave is. That
rule does not survive a real browser: back and forward have to carry direction,
and a search lens has to be round. The rule as it actually stands:

| Angle | Permitted |
|---|---|
| 90 degrees | Always. The default for anything box-like. |
| 45 degrees | Only where the object carries direction: arrows, chevrons, the check, the pencil, the bookmark notch, the shield point. |
| Arcs | Only where the object genuinely is round: the lens, the clock, the lock shackle, info, theme. |
| Anything else | Never. No 30s, no 60s, no arbitrary slopes. |

Where a glyph is conventionally round but does not need to be, it is squared off
instead. Settings is sliders, not a gear. Extensions is a module grid, not a
puzzle piece. Private is a mask, not a hat. Warning is a diamond, not a triangle,
because a triangle with 45-degree sides cannot fit the grid. That substitution is
where the set gets its character, and it is the thing that keeps it looking
related to the mark.

### 6.2 Grid

| Property | Value |
|---|---|
| Canvas | 24 x 24 |
| Live area | 20 x 20 (2 units padding all sides) |
| Stroke weight | 2 |
| Stroke cap | butt |
| Stroke join | miter |
| Corner radius | 0. Square corners. |
| Fill | none. Stroke-only, with two documented exceptions. |
| Arc radii | 7 for the lens, 9 for full discs (the system radius on this grid) |
| Coordinate grid | 0.5 units; put 2-unit centrelines on integers so they land on pixel boundaries at 24 and 48 |

Two icons break the stroke-only rule, both deliberately: `theme` fills a half
disc, because a light/dark toggle has to show contrast rather than describe it,
and `cleave` is the mark itself.

### 6.3 The set

44 icons, enough to build the whole browser chrome.

| Group | Icons |
|---|---|
| Navigation | `back` `forward` `reload` `home` |
| Tabs | `tab` `tab-new` `tab-pin` `tab-audio` `split` `sidebar` |
| Address bar | `lock` `shield` `shield-off` `info` `warning` `private` |
| Toolbar | `download` `upload` `bookmark` `history` `extensions` `menu` `more` `settings` `search` `zoom-in` `zoom-out` `print` |
| Window | `fullscreen` `fullscreen-exit` `maximize` `restore` `close` `minimize` |
| Content | `copy` `trash` `edit` `external` `check` `chevron-down` `chevron-right` `plus` |
| Theme | `theme` |
| Brand | `cleave` |

There is no separate `stop`: browsers use an X for it, and `close` already is
one. Adding a `stop` would have shipped two byte-identical files.

### 6.4 Optical rules

- Never scale a 24-grid icon to 16px. Redraw it: stroke to 1.5, detail out, live
  area 14 x 14. Same rule the logomark follows.
- State is expressed with colour, never by adding elements. An active shield is
  a Lamplight shield, not a shield with a tick on it.
- Two elements per icon wherever possible. Four is the hard ceiling, and only
  `extensions` and `settings` reach it.

### 6.5 Adding an icon

Add it to `ICONS` in `tools/icons.py`, then run:

```
python3 tools/audit.py
```

It checks every straight segment against the angle rule, computes exact arc
bounding boxes to confirm the ink stays inside the live area, flags anything too
small to read, and catches two icons that have ended up with identical paths.
It exits non-zero on any failure, so it drops straight into a pre-commit hook.

### 6.6 Examples

Shield. Flat top, vertical sides, 45-degree point.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="butt" stroke-linejoin="miter" role="img" aria-label="shield">
  <path d="M3 4H21V12L12 21L3 12Z"/>
</svg>
```

Search. One of the five icons allowed an arc, struck at the lens radius of 7.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="butt" stroke-linejoin="miter" role="img" aria-label="search">
  <path d="M11 4A7 7 0 1 1 11 18A7 7 0 1 1 11 4Z"/>
  <path d="M16 16L21 21"/>
</svg>
```

Cleave, the brand glyph. Not a UI icon: it is the mark itself, filled, mapped
from its 32 canvas onto the 24 icon grid so it sits correctly beside the set in
menus and about dialogs. It uses the small-size redraw because it renders at
icon sizes. Version 2.0 shipped a stroked approximation of the mark here, which
is why it did not look like the logo.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="butt" stroke-linejoin="miter" role="img" aria-label="cleave">
  <g transform="translate(-1.3333 -1.3333) scale(0.83333)" fill="currentColor" stroke="none"><path d="M4 4H8V18H20V28H4Z M12 4H28V28H24V14H12Z"/></g>
</svg>
```

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
assets/icons/        44 icons, incl. cleave.svg (the mark, filled)
assets/png/          45 rasters at every size the platforms ask for
assets/ico/          favicon.ico  brume.ico   (16 24 32 48 64 128 256 each)
assets/css/          tokens.css
tools/               kit.py  raster.py  preview.py  docs.py  wordmark.json
```

```
python3 tools/kit.py       # all SVG, from the geometry constants
python3 tools/raster.py    # PNG + ICO          (needs cairosvg, Pillow)
python3 tools/preview.py   # preview.html
python3 tools/docs.py      # this file
python3 tools/audit.py     # icon system conformance, exits non-zero on failure
```

`kit.py` holds the mark; `icons.py` holds the icon set. Those are the two files to edit. The wordmark travels with it as baked
outlines in `wordmark.json`, so the kit rebuilds with no font files and no
network access. Run everything through SVGO once with `removeViewBox: false`
before committing, and do not let it merge the two Cleave subpaths: they are
separate so you can address them independently later.
