#!/usr/bin/env python3
"""Derive the Windows tray's other two icon states from the brand mark — no deps.

The notification-area icon has three states, and `app/windows/HiAgentWindows/Ui/
TrayIcon.cs` is where each is chosen:

  HiAgent.ico            the mark, in colour       the agent is here
  HiAgentGrey.ico        drained of colour         it is not answering
  HiAgentListening.ico   knocked out of coral      it has its ear open

**That is one mark in three states, not three icons.** Both derived files are
computed from the first, so changing the logo and re-running this cannot leave
the states showing different marks — which is the failure a hand-drawn set
reaches about a year in, when nobody remembers there were three.

Neither derivation invents artwork. Grey is the mark desaturated and lifted;
listening is the *same silhouette* knocked out of the brand coral — a filled tile
where the other two are white. That is the one difference that survives 16px,
where a badge or an outline does not, and it is the shape a pressed toggle has
everywhere else on the platform.

An .ico built by `make-ico.py` is a directory header followed by verbatim PNGs,
so this reads those back out, transforms each, and re-packs — every size and
the alpha the rounded corners need, preserved by construction.

Usage: make-tray-icos.py [<in.ico>]
Defaults to the Windows shell's mark, writing both states beside it.
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

# The coral the "h" is drawn in, read back off the mark rather than typed from a
# brand doc — so a re-coloured logo carries its own new tile with it.
CORAL = (253, 96, 94)

# Saturation at which a pixel counts as fully "glyph" for the knockout. The mark's
# two inks sit at 0.63 and 0.78 by this measure; normalising at 0.6 makes both come
# out solid white rather than one pink and one nearly so, while anti-aliased edges
# below it still ramp smoothly.
GLYPH_AT = 0.6

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


def knockout(rgba: bytearray) -> bytearray:
    """The mark's silhouette in white on a filled coral tile.

    Coverage is how far a pixel is from white, so it reads the glyph without
    needing to know which ink drew it — the "h" and the "i" are different colours
    and both come out solid. Alpha is untouched for the same reason as above: the
    rounded corners are the silhouette, and this state must be the same shape.
    """
    out = bytearray(rgba)
    for i in range(len(rgba) // 4):
        r, g, b = rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]
        coverage = min(1.0, (1.0 - min(r, g, b) / 255.0) / GLYPH_AT)
        for c, background in enumerate(CORAL):
            out[i * 4 + c] = int(background + (255 - background) * coverage)
    return out


def derive(ico: bytes, transform, src: str) -> bytes:
    """Re-pack one .ico with `transform` applied to every size it carries."""
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
        entries.append((w, h, palette, planes, bpp, write_png(width, height, transform(rgba), colour)))

    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    directory, blobs = b"", b""
    for w, h, palette, planes, bpp, blob in entries:
        directory += struct.pack(
            "<BBBBHHII", w, h, palette, 0, planes, bpp, len(blob), offset
        )
        blobs += blob
        offset += len(blob)
    return header + directory + blobs


# Each derived state, and the file it is written to beside the mark.
STATES = (("Grey", desaturate), ("Listening", knockout))


def main() -> None:
    src = sys.argv[1] if len(sys.argv) > 1 else "app/windows/HiAgentWindows/Assets/HiAgent.ico"
    stem = src[: -len(".ico")] if src.endswith(".ico") else src

    with open(src, "rb") as f:
        ico = f.read()

    for suffix, transform in STATES:
        dst = f"{stem}{suffix}.ico"
        with open(dst, "wb") as f:
            f.write(derive(ico, transform, src))
        print(f"wrote {dst}")


if __name__ == "__main__":
    main()
