#!/usr/bin/env python3
"""Rasterise the Brume SVGs and assemble the two .ico containers.

Requires cairosvg and Pillow. Everything it reads is produced by kit.py.
"""
import io, os, struct, sys
import cairosvg
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GEN, ICN = f"{ROOT}/assets/svg/generated", f"{ROOT}/assets/icons"
PNG, ICO = f"{ROOT}/assets/png", f"{ROOT}/assets/ico"


def render(src, out=None, w=None):
    img = Image.open(io.BytesIO(cairosvg.svg2png(url=src, output_width=w))).convert("RGBA")
    if out:
        img.save(out, "PNG", optimize=True)
    return img


JOBS = [
    (f"{GEN}/mark-ink.svg",        "mark-ink",        [32, 64, 128, 256, 512]),
    (f"{GEN}/mark-paper.svg",      "mark-paper",      [32, 64, 128, 256, 512]),
    (f"{GEN}/mark-haar.svg",       "mark-haar",       [64, 256]),
    (f"{GEN}/mark-sm-ink.svg",     "favicon-ink",     [16, 24, 32]),
    (f"{GEN}/mark-sm-paper.svg",   "favicon-paper",   [16, 24, 32]),
    (f"{GEN}/wordmark-ink.svg",    "wordmark-ink",    [528, 1056]),
    (f"{GEN}/wordmark-paper.svg",  "wordmark-paper",  [528, 1056]),
    (f"{GEN}/lockup-h-ink.svg",    "lockup-h-ink",    [365, 730, 1460]),
    (f"{GEN}/lockup-h-paper.svg",  "lockup-h-paper",  [365, 730, 1460]),
    (f"{GEN}/lockup-v-ink.svg",    "lockup-v-ink",    [264, 528, 1056]),
    (f"{GEN}/lockup-v-paper.svg",  "lockup-v-paper",  [264, 528, 1056]),
    (f"{GEN}/tile-dark.svg",       "tile-dark",       [64, 128, 256, 512]),
    (f"{GEN}/tile-light.svg",      "tile-light",      [256]),
]


def bmp_entry(img):
    """32-bit BGRA DIB with AND mask, the shape an .ico wants for small sizes."""
    w, h = img.size
    px = img.load()
    xor = bytearray()
    for y in range(h - 1, -1, -1):                    # DIBs run bottom-up
        for x in range(w):
            r, g, b, a = px[x, y]
            xor += bytes((b, g, r, a))
    row = ((w + 31) // 32) * 4                        # 1bpp mask, 4-byte aligned
    mask = bytearray(row * h)
    hdr = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, len(xor) + len(mask), 0, 0, 0, 0)
    return bytes(hdr + xor + mask)


def png_entry(img):
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def write_ico(path, images):
    """<=64px go in as BMP for shell compatibility, 96 and up as PNG for size."""
    blobs = [(im, bmp_entry(im) if im.size[0] <= 64 else png_entry(im)) for im in images]
    off = 6 + 16 * len(blobs)
    out = bytearray(struct.pack("<HHH", 0, 1, len(blobs)))
    for im, blob in blobs:
        w, h = im.size
        out += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(blob), off)
        off += len(blob)
    for _, blob in blobs:
        out += blob
    open(path, "wb").write(bytes(out))


def main():
    os.makedirs(PNG, exist_ok=True)
    os.makedirs(ICO, exist_ok=True)
    n = 0
    for src, stem, widths in JOBS:
        for w in widths:
            render(src, f"{PNG}/{stem}-{w}.png", w)
            n += 1
    for name, w in (("apple-touch-icon", 180), ("icon-192", 192), ("icon-512", 512)):
        render(f"{GEN}/tile-dark.svg", f"{PNG}/{name}.png", w)
        n += 1
    os.makedirs(f"{PNG}/icons", exist_ok=True)
    for f in sorted(os.listdir(ICN)):
        k = f[:-4]
        tmp = f"/tmp/{k}.svg"
        open(tmp, "w").write(open(f"{ICN}/{f}").read().replace("currentColor", "#101418"))
        render(tmp, f"{PNG}/icons/{k}-48.png", 48)
        n += 1

    # Small sizes take the small redraw; 32 and up take the primary geometry.
    #
    # favicon.ico is for browser tabs, where only 16/32/48 are ever asked for,
    # so it keeps the shorter ladder rather than carrying shell sizes it will
    # never be rendered at.
    write_ico(f"{ICO}/favicon.ico", [
        render(f"{GEN}/mark-sm-haar.svg", w=16), render(f"{GEN}/mark-sm-haar.svg", w=24),
        *[render(f"{GEN}/mark-haar.svg", w=s) for s in (32, 48, 64, 128, 256)]])

    # brume.ico is the Windows application icon, so it carries the full ladder
    # the shell actually requests: 16, 20, 24, 32, 40, 48, 64, 96, 128, 256.
    #
    # The three easy ones to omit are 20, 40 and 96, because no size picker
    # names them - they are what Explorer, the Start menu and the taskbar ask
    # for at 125%, 250% and Extra Large respectively. Leave them out and Windows
    # downscales the neighbour, which is why an icon can look soft at one
    # scaling factor and crisp at the next.
    write_ico(f"{ICO}/brume.ico", [
        *[render(f"{GEN}/tile-dark-sm.svg", w=s) for s in (16, 20, 24)],
        *[render(f"{GEN}/tile-dark.svg", w=s) for s in (32, 40, 48, 64, 96, 128, 256)]])

    print(f"{n} PNGs written")
    for f in sorted(os.listdir(ICO)):
        p = f"{ICO}/{f}"
        print(f"  {f:<13}{os.path.getsize(p):>8,} bytes  {sorted(Image.open(p).ico.sizes())}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
