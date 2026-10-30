"""Labelling aids: a grid view of each keyframe (for boxing missed heads by
eye) and contact pages of a series' AniList cast.

    uv run python -I views.py grid <kf_dir>
    uv run python -I views.py cast <cast_dir> [per_page=24]
"""
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def grid(kf: Path) -> None:
    font = ImageFont.load_default(size=16)
    out = kf / "grid"
    out.mkdir(exist_ok=True)
    for f in sorted((kf / "frames").glob("*.jpg")):
        img = Image.open(f).convert("RGB")
        w, h = img.size
        big = img.resize((w * 3 // 2, h * 3 // 2), Image.BICUBIC)
        bw, bh = big.size
        d = ImageDraw.Draw(big)
        for i in range(1, 10):
            x, y = bw * i / 10, bh * i / 10
            d.line([(x, 0), (x, bh)], fill=(255, 255, 0), width=1)
            d.line([(0, y), (bw, y)], fill=(255, 255, 0), width=1)
            d.text((x + 2, 2), f"{i / 10:.1f}", fill="yellow", font=font, stroke_width=2, stroke_fill="black")
            d.text((2, y + 2), f"{i / 10:.1f}", fill="yellow", font=font, stroke_width=2, stroke_fill="black")
        big.save(out / f.name, quality=85)


def cast(root: Path, per_page: int) -> None:
    chars = json.load(open(root / "cast.json", encoding="utf-8"))
    font = ImageFont.load_default(size=15)
    cw, ch = 230, 330
    for p in range(0, len(chars), per_page):
        page = chars[p : p + per_page]
        cols = 6
        rows = (len(page) + cols - 1) // cols
        im = Image.new("RGB", (cols * cw, rows * ch), "black")
        d = ImageDraw.Draw(im)
        for i, c in enumerate(page):
            x, y = (i % cols) * cw, (i // cols) * ch
            try:
                pic = Image.open(c["file"]).convert("RGB")
                pic.thumbnail((cw - 10, ch - 50))
                im.paste(pic, (x + 5, y + 5))
            except Exception:
                pass
            d.text((x + 5, y + ch - 42), f"{c['name']}"[:28], fill="white", font=font)
            d.text((x + 5, y + ch - 22), f"{c['role']} #{c['id']}", fill="gray", font=font)
        im.save(root / f"cast_p{p // per_page:02d}.jpg", quality=85)


if __name__ == "__main__":
    if sys.argv[1] == "grid":
        grid(Path(sys.argv[2]))
    else:
        cast(Path(sys.argv[2]), int(sys.argv[3]) if len(sys.argv) > 3 else 24)
