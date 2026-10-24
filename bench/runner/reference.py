#!/usr/bin/env python3
"""A fixture's exact frame for a time stamp, as RGB bytes: `quality` as bench/fixtures/quality.html draws it, and
`text-scroll` as bench/fixtures/text-scroll.html draws text-scroll.png."""

import pathlib
import struct
import zlib

WIDTH, HEIGHT, BLOCK, BITS = 1280, 800, 16, 40


def fill(buf, x, y, w, h, rgb):
    x0, y0, x1, y1 = max(x, 0), max(y, 0), min(x + w, WIDTH), min(y + h, HEIGHT)
    if x1 <= x0 or y1 <= y0:
        return
    row = bytes(rgb) * (x1 - x0)
    for yy in range(y0, y1):
        start = (yy * WIDTH + x0) * 3
        buf[start:start + len(row)] = row


def stamp(buf, t):
    bits = [1, 0] + [(t >> b) & 1 for b in range(BITS - 1, -1, -1)] + [0, 1]
    for n, bit in enumerate(bits):
        fill(buf, n * BLOCK, 0, BLOCK, BLOCK, (255, 255, 255) if bit else (0, 0, 0))


def read_png(path):
    """Width, height, and RGB bytes of an 8-bit RGB or RGBA PNG (alpha dropped)."""
    data = pathlib.Path(path).read_bytes()
    width, height, depth, color = struct.unpack(">IIBB", data[16:26])
    if depth != 8 or color not in (2, 6):
        raise ValueError(f"{path}: only 8-bit RGB or RGBA images are supported")
    pos, idat = 8, b""
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        if kind == b"IDAT":
            idat += data[pos + 8:pos + 8 + length]
        pos += 12 + length
    raw = zlib.decompress(idat)
    bpp = 3 if color == 2 else 4
    stride = width * bpp
    out, prev = bytearray(), bytearray(stride)
    for y in range(height):
        kind, row = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = row[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if kind == 1:
                row[i] = (row[i] + a) & 255
            elif kind == 2:
                row[i] = (row[i] + b) & 255
            elif kind == 3:
                row[i] = (row[i] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                row[i] = (row[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += row if bpp == 3 else b"".join(row[i:i + 3] for i in range(0, stride, 4))
        prev = row
    return width, height, bytes(out)


_TEXT = None


def text_scroll(t):
    global _TEXT
    if _TEXT is None:
        _TEXT = read_png(pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "text-scroll.png")
    width, height, rgb = _TEXT
    y = ((t // 16) * 4) % (height - HEIGHT)
    buf = bytearray(rgb[y * width * 3:(y + HEIGHT) * width * 3])
    stamp(buf, t)
    return bytes(buf)


def frame(t):
    buf = bytearray(WIDTH * HEIGHT * 3)
    k = t // 16
    for j in range(13):
        for i in range(20):
            fill(buf, i * 64, j * 64, 64, 64, ((i * 37 + k * 3) & 255, (j * 53 + k * 5) & 255, ((i + j) * 29 + k * 7) & 255))
    for n in range(8):
        x, y = (k * 8 + n * 150) % 1280, (k * 5 + n * 90) % 800
        fill(buf, x, y, 96, 64, ((n * 31) & 255, 255 - ((n * 47) & 255), (n * 97) & 255))
    stamp(buf, t)
    return bytes(buf)


FRAMES = {"quality": frame, "text-scroll": text_scroll}


def png(rgb, path, width=WIDTH, height=HEIGHT):
    raw = b"".join(b"\0" + rgb[y * width * 3:(y + 1) * width * 3] for y in range(height))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    import sys
    # reference.py FIXTURE STAMP OUT.png   writes a fixture's exact frame for a stamp
    # reference.py normalize IN.png OUT.png  rewrites any 8-bit RGB(A) PNG as the plain RGB PNG fixtures load
    if sys.argv[1] == "normalize":
        w, h, rgb = read_png(sys.argv[2])
        png(rgb, sys.argv[3], w, h)
    else:
        png(FRAMES[sys.argv[1]](int(sys.argv[2])), sys.argv[3])
