# /// script
# requires-python = ">=3.12"
# dependencies = ["pillow==13.0.0"]
# ///
"""Renders the installer art from the mark (app/public/autoscene-mark.svg) and
the colours in docs/DESIGN.md. See README.md beside this file for the command.

Outputs (committed, so a build needs none of this):
  windows/sidebar.bmp       NSIS welcome and finish pages, 328x628 (2x of 164x314)
  windows/header.bmp        NSIS page header, installer and uninstaller, 300x114 (2x of 150x57)
  macos/dmg-background.png  DMG window, 1320x800 px at 144 dpi = 660x400 pt
  icons/macos/icon.icns     macOS app icon on Apple's grid (824 px tile in 1024 px)
"""

import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

TAURI = Path(__file__).resolve().parent.parent
FONT = TAURI.parent / "node_modules/@fontsource/instrument-serif/files/instrument-serif-latin-400-normal.woff"

# docs/DESIGN.md
CANVAS = (0x09, 0x09, 0x0B)  # --bg-canvas
FG_1 = (0xF5, 0xF3, 0xEF)  # --fg-1, warm white
FG_2 = (0xB4, 0xB1, 0xAC)  # --fg-2
# app/public/autoscene-mark.svg
TILE = (0x1C, 0x1A, 0x17)
GOLD = (0xC4, 0xA3, 0x5A)
EDGE = (242, 237, 230, round(0.12 * 255))
# Under the Finder's icon labels: black (light mode) and white (dark mode)
# both read on it at 4.6:1.
LABEL_PLATE = (0x78, 0x75, 0x70)

SS = 8  # supersampling: Pillow draws shapes without anti-aliasing


def mark_layer(size: tuple[int, int], u: float, ox: float, oy: float) -> Image.Image:
    """The mark as an RGBA layer at final size. Its 48-unit viewBox is placed
    at (ox, oy) with u pixels per unit, the geometry exactly as in the SVG."""
    w, h = size
    s = u * SS

    def p(x: float, y: float) -> tuple[float, float]:
        return (ox * SS + x * s, oy * SS + y * s)

    big = Image.new("RGBA", (w * SS, h * SS), (0, 0, 0, 0))
    d = ImageDraw.Draw(big)
    # <rect x=1 y=1 width=46 height=46 rx=10 fill=TILE>
    d.rounded_rectangle([p(1, 1), p(47, 47)], radius=10 * s, fill=TILE + (255,))
    # Its stroke: 1 unit wide, centred on the rect's edge.
    edge = Image.new("RGBA", big.size, (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle(
        [p(0.5, 0.5), p(47.5, 47.5)], radius=10.5 * s, outline=EDGE, width=max(1, round(s))
    )
    big = Image.alpha_composite(big, edge)
    d = ImageDraw.Draw(big)
    # <path d="M14 32 L24 12 L34 32" stroke-width=2.5 round caps and joins>
    pts = [p(14, 32), p(24, 12), p(34, 32)]
    r = 1.25 * s
    d.line(pts, fill=GOLD + (255,), width=round(2.5 * s), joint="curve")
    for x, y in (pts[0], pts[2]):
        d.ellipse([x - r, y - r, x + r, y + r], fill=GOLD + (255,))
    # <circle cx=24 cy=28 r=3>
    cx, cy = p(24, 28)
    d.ellipse([cx - 3 * s, cy - 3 * s, cx + 3 * s, cy + 3 * s], fill=GOLD + (255,))
    return big.resize(size, Image.Resampling.LANCZOS)


def tile_silhouette(size: tuple[int, int], u: float, ox: float, oy: float) -> Image.Image:
    """The tile's shape as an L mask at final size (for glows and shadows)."""
    w, h = size
    big = Image.new("L", (w * SS, h * SS), 0)
    s = u * SS
    ImageDraw.Draw(big).rounded_rectangle(
        [(ox * SS + s, oy * SS + s), (ox * SS + 47 * s, oy * SS + 47 * s)], radius=10 * s, fill=255
    )
    return big.resize(size, Image.Resampling.LANCZOS)


def glow(size, mask: Image.Image, colour, opacity: float, blur: float, dy: float = 0) -> Image.Image:
    """A soft coloured light from a mask: blurred, shifted down by dy."""
    shifted = Image.new("L", size, 0)
    shifted.paste(mask, (0, round(dy)))  # shifts in empty space (ImageChops.offset wraps around)
    m = shifted.filter(ImageFilter.GaussianBlur(blur))
    m = m.point(lambda v: round(v * opacity))
    layer = Image.new("RGBA", size, colour + (0,))
    layer.putalpha(m)
    return layer


def radial(size, centre, radii, colour, opacity: float) -> Image.Image:
    """A wide, soft elliptical light (a blurred ellipse)."""
    w, h = size
    m = Image.new("L", size, 0)
    cx, cy = centre
    rx, ry = radii
    ImageDraw.Draw(m).ellipse([cx - rx, cy - ry, cx + rx, cy + ry], fill=round(255 * opacity))
    m = m.filter(ImageFilter.GaussianBlur(min(rx, ry) * 0.6))
    layer = Image.new("RGBA", size, colour + (0,))
    layer.putalpha(m)
    return layer


def text(img: Image.Image, xy, s: str, px: int, colour, anchor="ms") -> None:
    font = ImageFont.truetype(str(FONT), px)
    ImageDraw.Draw(img).text(xy, s, font=font, fill=colour + (255,), anchor=anchor)


def mark_centred(cx, cy, edge_px):
    """Placement of the mark so that its 48-unit viewBox is edge_px wide, centred on (cx, cy)."""
    u = edge_px / 48
    return u, cx - 24 * u, cy - 24 * u


def sidebar() -> Image.Image:
    size = (328, 628)
    img = Image.new("RGBA", size, CANVAS + (255,))
    u, ox, oy = mark_centred(164, 250, 144)
    img = Image.alpha_composite(img, glow(size, tile_silhouette(size, u, ox, oy), GOLD, 0.45, 26, dy=10))
    img = Image.alpha_composite(img, mark_layer(size, u, ox, oy))
    text(img, (164, 400), "AutoScene", 60, FG_1)
    return img.convert("RGB")


def header() -> Image.Image:
    size = (300, 114)
    img = Image.new("RGBA", size, (255, 255, 255, 255))
    u, ox, oy = mark_centred(150, 57, 64)
    img = Image.alpha_composite(img, mark_layer(size, u, ox, oy))
    return img.convert("RGB")


def dmg_background() -> Image.Image:
    # The window and the icon centres (in points) come from the DMG config;
    # icons are 128 pt with 16 pt labels (the bundler's sizes). Everything
    # here is drawn at 2x.
    dmg = json.loads((TAURI / "tauri.macos.conf.json").read_text(encoding="utf-8"))["bundle"]["macOS"]["dmg"]
    win_w, win_h = dmg["windowSize"]["width"], dmg["windowSize"]["height"]
    app_x, icon_y = dmg["appPosition"]["x"], dmg["appPosition"]["y"]
    apps_x = dmg["applicationFolderPosition"]["x"]
    assert dmg["applicationFolderPosition"]["y"] == icon_y, "the arrow assumes both icons on one line"
    size = (win_w * 2, win_h * 2)
    img = Image.new("RGBA", size, CANVAS + (255,))
    img = Image.alpha_composite(img, radial(size, (win_w, icon_y * 2 - 40), (520, 210), GOLD, 0.12))

    # Plates behind the two labels: Finder sets a 16 pt label's text at about
    # 247-264 pt below a 128 pt icon centred at 170 (measured in the CI shot).
    plates = Image.new("RGBA", (size[0] * 2, size[1] * 2), (0, 0, 0, 0))
    d = ImageDraw.Draw(plates)
    for cx in (app_x, apps_x):
        x0, x1 = (cx - 66) * 4, (cx + 66) * 4
        y0, y1 = (icon_y + 71) * 4, (icon_y + 97) * 4
        d.rounded_rectangle([x0, y0, x1, y1], radius=13 * 4, fill=LABEL_PLATE + (255,))
    img = Image.alpha_composite(img, plates.resize(size, Image.Resampling.LANCZOS))

    # A thin gold arrow from the app to Applications, in the mark's stroke style.
    arrow = Image.new("RGBA", (size[0] * 4, size[1] * 4), (0, 0, 0, 0))
    d = ImageDraw.Draw(arrow)
    k = 8  # pt -> supersampled px
    y = icon_y * k
    x0, x1 = (app_x + 92) * k, (apps_x - 92) * k
    w = round(2 * k)
    r = w / 2
    d.line([(x0, y), (x1, y)], fill=GOLD + (255,), width=w)
    head = [(x1 - 12 * k, y - 10 * k), (x1, y), (x1 - 12 * k, y + 10 * k)]
    d.line(head, fill=GOLD + (255,), width=w, joint="curve")
    for x, yy in (head[0], head[2], (x0, y)):
        d.ellipse([x - r, yy - r, x + r, yy + r], fill=GOLD + (255,))
    img = Image.alpha_composite(img, arrow.resize(size, Image.Resampling.LANCZOS))

    text(img, (win_w, win_h * 2 - 132), "Drag AutoScene to Applications", 44, FG_2)
    return img.convert("RGB")


def icon_1024() -> Image.Image:
    # Apple's macOS grid: an 824 px tile centred in 1024 px, with the
    # template's shadow (black 30 %, 10 px down, 10 px blur).
    size = (1024, 1024)
    u = 824 / 46
    ox = oy = 100 - u  # the rect starts at unit 1
    img = Image.new("RGBA", size, (0, 0, 0, 0))
    img = Image.alpha_composite(img, glow(size, tile_silhouette(size, u, ox, oy), (0, 0, 0), 0.30, 5, dy=10))
    img = Image.alpha_composite(img, mark_layer(size, u, ox, oy))
    return img


# The PNG entries macOS reads as PNG. Not icp4/icp5 (16/32 px 1x): Finder
# showed PNG data there as noise (the DMG window's title icon); it scales
# ic11/ic12 down instead.
ICNS_TYPES = [
    (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024),
    (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512),
]


def write_icns(path: Path, master: Image.Image) -> None:
    import io
    import struct

    body = b""
    for kind, px in ICNS_TYPES:
        im = master if px == master.width else master.resize((px, px), Image.Resampling.LANCZOS)
        buf = io.BytesIO()
        im.save(buf, format="PNG", optimize=True)
        data = buf.getvalue()
        body += kind + struct.pack(">I", 8 + len(data)) + data
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def main() -> None:
    out = {
        TAURI / "windows/sidebar.bmp": sidebar(),
        TAURI / "windows/header.bmp": header(),
    }
    for path, im in out.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        im.save(path, format="BMP")
    bg = TAURI / "macos/dmg-background.png"
    bg.parent.mkdir(parents=True, exist_ok=True)
    dmg_background().save(bg, format="PNG", dpi=(144, 144), optimize=True)
    icns = TAURI / "icons/macos/icon.icns"
    icns.parent.mkdir(parents=True, exist_ok=True)
    write_icns(icns, icon_1024())
    for p in [*out, bg, icns]:
        print(p.relative_to(TAURI.parent.parent))


if __name__ == "__main__":
    main()
