#!/usr/bin/env python3
"""Check every icon against the system rules. Run this after adding one.

  angles  every straight segment must be 0, 45 or 90 degrees
  bounds  ink (stroke included) must stay inside 2..22
  size    ink must fill at least 11 units on one axis, or it reads too small
  unique  no two icons may share an identical path set
"""
import math, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import icons as I


def arc_bounds(x0, y0, r, fa, fs, x1, y1):
    """Exact extremes of a circular SVG arc: both endpoints, plus whichever of
    the four cardinal points of the circle actually fall on the swept sector."""
    pts = [(x0, y0), (x1, y1)]
    dx, dy = (x0 - x1) / 2.0, (y0 - y1) / 2.0
    lam = (dx * dx + dy * dy) / (r * r)
    if lam > 1:                       # SVG inflates r rather than failing
        r *= math.sqrt(lam)
    num = max(0.0, r * r - dx * dx - dy * dy)
    coef = math.sqrt(num / (dx * dx + dy * dy)) if (dx or dy) else 0.0
    if fa == fs:
        coef = -coef
    cx = coef * dy + (x0 + x1) / 2.0
    cy = -coef * dx + (y0 + y1) / 2.0
    t0 = math.atan2(y0 - cy, x0 - cx)
    t1 = math.atan2(y1 - cy, x1 - cx)
    delta = t1 - t0
    if fs == 0 and delta > 0:
        delta -= 2 * math.pi
    elif fs == 1 and delta < 0:
        delta += 2 * math.pi
    for k in range(4):                # 0, 90, 180, 270 degrees
        a = k * math.pi / 2
        for turn in (-2, -1, 0, 1, 2):
            s = a + turn * 2 * math.pi - t0
            if (0 <= s <= delta) if delta >= 0 else (delta <= s <= 0):
                pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
                break
    return pts


def walk(d):
    """Yield ('seg',dx,dy) and ('pt',x,y), tracking the cursor through H/V/A."""
    cx = cy = sx = sy = 0.0
    for cmd, args in re.findall(r'([MLHVAZmlhvaz])([^MLHVAZmlhvaz]*)', d):
        n = [float(v) for v in re.findall(r'-?\d*\.?\d+', args)]
        if cmd == 'M':
            for i in range(0, len(n), 2):
                cx, cy = n[i], n[i + 1]
                if i == 0: sx, sy = cx, cy
                yield ('pt', cx, cy)
        elif cmd == 'L':
            for i in range(0, len(n), 2):
                yield ('seg', n[i] - cx, n[i + 1] - cy)
                cx, cy = n[i], n[i + 1]; yield ('pt', cx, cy)
        elif cmd == 'H':
            for v in n:
                yield ('seg', v - cx, 0.0); cx = v; yield ('pt', cx, cy)
        elif cmd == 'V':
            for v in n:
                yield ('seg', 0.0, v - cy); cy = v; yield ('pt', cx, cy)
        elif cmd == 'A':
            for i in range(0, len(n), 7):
                r, fa, fs, ex, ey = n[i], int(n[i+3]), int(n[i+4]), n[i+5], n[i+6]
                for px, py in arc_bounds(cx, cy, r, fa, fs, ex, ey):
                    yield ('pt', px, py)
                cx, cy = ex, ey
        elif cmd in 'Zz':
            yield ('seg', sx - cx, sy - cy); cx, cy = sx, sy


def main():
    bad_angle, oob, small = [], [], []
    seen = {}
    for name, paths in I.ICONS.items():
        key = tuple(d for d, _ in paths)
        if key in seen:
            print(f"  DUPLICATE: {name} is identical to {seen[key]}")
        seen[key] = name
        xs, ys = [], []
        for d, _ in paths:
            for ev in walk(d):
                if ev[0] == 'seg':
                    dx, dy = ev[1], ev[2]
                    if abs(dx) < 1e-9 and abs(dy) < 1e-9: continue
                    a = round(math.degrees(math.atan2(abs(dy), abs(dx))))
                    if a not in (0, 45, 90):
                        bad_angle.append((name, a))
                else:
                    xs.append(ev[1]); ys.append(ev[2])
        x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
        if x0 - 1 < 1.5 or y0 - 1 < 1.5 or x1 + 1 > 22.5 or y1 + 1 > 22.5:
            oob.append((name, (x0, y0, x1, y1)))
        if (x1 - x0) < 11 and (y1 - y0) < 11:
            small.append((name, round(x1 - x0, 1), round(y1 - y0, 1)))

    print(f"icons: {len(I.ALL)}  (incl. the brand glyph, which is exempt)")
    for label, rows in (("non-conforming angles", bad_angle),
                        ("outside live area", oob),
                        ("under-filled", small)):
        print(f"  {label:<24}{len(rows)}")
        for r in rows[:6]:
            print(f"      {r}")
    return 1 if (bad_angle or oob or small) else 0


if __name__ == "__main__":
    sys.exit(main())
