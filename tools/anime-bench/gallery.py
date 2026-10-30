"""Render a series' gallery pages from its exemplars, for labellers.

    uv run python -I gallery.py <gallery_dir> <kf_root>

`<gallery_dir>/gallery.tsv` has `code person type note`; `exemplars.json` maps
each code to up to 4 `{"ep": ..., "frame": ..., "cand": id}` picks (candidate
ids of the keyframe sheets) or `{"ep", "frame", "box": [x0, y0, x1, y1]}`.
"""
import csv
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def main() -> None:
    gdir, kf = Path(sys.argv[1]), Path(sys.argv[2])
    rows = list(csv.DictReader(open(gdir / "gallery.tsv", encoding="utf-8"), delimiter="\t"))
    ex = json.load(open(gdir / "exemplars.json", encoding="utf-8"))
    font = ImageFont.load_default(size=17)
    cell, per_page = 180, 12
    for p in range(0, len(rows), per_page):
        page = rows[p : p + per_page]
        im = Image.new("RGB", (220 + 4 * cell, len(page) * (cell + 8)), "black")
        d = ImageDraw.Draw(im)
        for r, row in enumerate(page):
            y = r * (cell + 8)
            d.text((6, y + 6), row["code"], fill="yellow", font=font)
            d.text((6, y + 30), f"{row['person']} ({row['type']})"[:26], fill="white", font=font)
            for i, k, in enumerate(ex.get(row["code"], [])[:4]):
                cands = json.load(open(kf / k["ep"] / "cands.json"))["frames"][str(k["frame"])]
                box = k.get("box") or next(c["box"] for c in cands if c["id"] == k["cand"])
                frame = Image.open(kf / k["ep"] / "frames" / f"{k['frame']}.jpg")
                w, h = frame.size
                cx, cy = (box[0] + box[2]) / 2 * w, (box[1] + box[3]) / 2 * h
                side = max((box[2] - box[0]) * w, (box[3] - box[1]) * h) * 1.6
                crop = frame.crop((int(cx - side / 2), int(cy - side / 2), int(cx + side / 2), int(cy + side / 2)))
                im.paste(crop.resize((cell, cell)), (220 + i * cell, y))
        im.save(gdir / f"gallery_p{p // per_page:02d}.jpg", quality=88)
    with open(gdir / "gallery.md", "w", encoding="utf-8") as f:
        for row in rows:
            f.write(f"- `{row['code']}`: {row['person']} ({row['type']}). {row['note']}\n")


if __name__ == "__main__":
    main()
