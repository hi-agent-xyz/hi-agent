#!/usr/bin/env python3
"""Derive the grey Windows tray icon from the colour one — no deps.

The Windows shell shows the brand mark in colour while the agent is answering
and drained of colour while it is not (`app/windows/HiAgentWindows/Ui/
TrayIcon.cs`). That is one mark in two states, not two icons, so the grey one is
*derived* rather than drawn: change the logo and re-run this, and the pair cannot
end up showing different marks.

An .ico built by `make-ico.py` is a directory header followed by verbatim PNGs,
so this reads those back out, desaturates each, and re-packs — every size and
the alpha the rounded corners need, preserved by construction.

Usage: make-grey-ico.py [<in.ico> [<out.ico>]]
Defaults to the Windows shell's pair.
"""
import struct
import sys
import zlib

# How far to pull each pixel toward its own luminance (1.0 = no colour left),
# and how far that luminance is then lifted toward a soft mid grey. Full
# desaturation alone reads as a photo printed in black and white; the lift is
# what makes it read as "off".
DESATURATE = 1.0
LIFT_TO = 150
LIFT = 0.35

CHANNELS = {0: 1, 2: 3, 4: 2, 6: 4}  # PNG colour type -> samples per pixel


def read_png(data: bytes):
    """Decode a non-interlaced 8-bit PNG to (width, height, RGBA, colour type)."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    pos, idat, width, height, colour = 8, b"", 0, 0, 6
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            width, height, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", chunk)
            if depth != 8 or interlace:
                raise ValueError(f"unsupported PNG: depth={depth} interlace={interlace}")
        elif kind == b"IDAT":
            idat += chunk
        elif kind == b"IEND":
            break

    raw = zlib.decompress(idat)
    step = CHANNELS[colour]
    stride = width * step
    pixels = bytearray(height * stride)
    previous = bytearray(stride)
    at = 0
    for y in range(height):
        method = raw[at]
        at += 1
        line = bytearray(raw[at : at + stride])
        at += stride
        for x in range(stride):
            left = line[x - step] if x >= step else 0
            up = previous[x]
            upleft = previous[x - step] if x >= step else 0
            if method == 0:
                value = line[x]
            elif method == 1:
                value = line[x] + left
            elif method == 2:
                value = line[x] + up
            elif method == 3:
                value = line[x] + ((left + up) >> 1)
            elif method == 4:
                estimate = left + up - upleft
                dl, du, dul = abs(estimate - left), abs(estimate - up), abs(estimate - upleft)
                nearest = left if (dl <= du and dl <= dul) else (up if du <= dul else upleft)
                value = line[x] + nearest
            else:
                raise ValueError(f"unknown filter {method}")
            line[x] = value & 0xFF
        pixels[y * stride : (y + 1) * stride] = line
        previous = line

    rgba = bytearray(width * height * 4)
    for i in range(width * height):
        sample = pixels[i * step : i * step + step]
        if step == 4:
            rgba[i * 4 : i * 4 + 4] = sample
        elif step == 3:
            rgba[i * 4 : i * 4 + 4] = bytes(sample) + b"\xff"
        elif step == 2:
            rgba[i * 4 : i * 4 + 4] = bytes((sample[0], sample[0], sample[0], sample[1]))
        else:
            rgba[i * 4 : i * 4 + 4] = bytes((sample[0], sample[0], sample[0], 255))
    return width, height, rgba, colour


def write_png(width: int, height: int, rgba: bytes, colour: int) -> bytes:
    """Re-encode RGBA as a PNG of the same colour type it came in as."""
    step = CHANNELS[colour]
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter: none — these are tiny, and it keeps this honest
        for x in range(width):
            pixel = rgba[(y * width + x) * 4 : (y * width + x) * 4 + 4]
            if step == 4:
                raw += pixel
            elif step == 3:
                raw += pixel[:3]
            elif step == 2:
                raw += bytes((pixel[0], pixel[3]))
            else:
                raw += bytes((pixel[0],))

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (
            struct.pack(">I", len(body))
            + kind
            + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", width, height, 8, colour, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def desaturate(rgba: bytearray) -> bytearray:
    out = bytearray(rgba)
    for i in range(len(rgba) // 4):
        r, g, b = rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]
        grey = 0.299 * r + 0.587 * g + 0.114 * b
        grey += (LIFT_TO - grey) * LIFT
        for c, value in enumerate((r, g, b)):
            out[i * 4 + c] = int(value + (grey - value) * DESATURATE)
        # Alpha is untouched: the rounded corners are transparent, and a mark
        # that lost its silhouette would read as a different icon, not a
        # quieter one.
    return out


def main() -> None:
    src = sys.argv[1] if len(sys.argv) > 1 else "app/windows/HiAgentWindows/Assets/HiAgent.ico"
    dst = sys.argv[2] if len(sys.argv) > 2 else "app/windows/HiAgentWindows/Assets/HiAgentGrey.ico"

    with open(src, "rb") as f:
        ico = f.read()
    reserved, kind, count = struct.unpack("<HHH", ico[:6])
    if reserved or kind != 1 or count == 0:
        sys.exit(f"{src}: not an icon file")

    entries = []
    for i in range(count):
        at = 6 + 16 * i
        w, h, palette, _, planes, bpp, length, offset = struct.unpack("<BBBBHHII", ico[at : at + 16])
        blob = ico[offset : offset + length]
        if blob[:8] != b"\x89PNG\r\n\x1a\n":
            sys.exit(f"{src}: entry {i} is a BMP; only the PNG form make-ico.py writes is handled")
        width, height, rgba, colour = read_png(blob)
        entries.append((w, h, palette, planes, bpp, write_png(width, height, desaturate(rgba), colour)))

    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    directory, blobs = b"", b""
    for w, h, palette, planes, bpp, blob in entries:
        directory += struct.pack(
            "<BBBBHHII", w, h, palette, 0, planes, bpp, len(blob), offset
        )
        blobs += blob
        offset += len(blob)

    with open(dst, "wb") as f:
        f.write(header + directory + blobs)
    print(f"wrote {dst}: {len(entries)} sizes {[w or 256 for w, *_ in entries]}")


if __name__ == "__main__":
    main()
