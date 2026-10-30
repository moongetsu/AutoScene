"""Turn adjudicated keyframe labels into `<ep>.keyframes.json` (FORMATS.md).

    uv run python -I to_keyframes.py <kf_root> <out_dir>

Reads `labels/<series>_<batch>_{A,B,final}.json` and each episode's
`cands.json`. A head is marked `"adjudicated": true` when the two labellers
gave it different classes or codes (or only one of them listed a missed
head): such labels are less sure and go to the human audit first.
"""
import json
import sys
from collections import defaultdict
from pathlib import Path

HEADS = {"H", "Hm", "Hk", "Hb", "Hn"}
UNNAMED = {"?e", "?u", "x"}


def ignore_unnamed_creature(h: dict) -> dict:
    """An unnamed creature head (swarms, mobs of monsters) becomes an ignore
    region `I`: detecting it is neither a find nor a false alarm (DECISIONS #4)."""
    if h["class"] == "Hn" and h.get("code", "x") in UNNAMED:
        h = {**h, "class": "I"}
        h.pop("code", None)
    return h


def iou(a: list[float], b: list[float]) -> float:
    w = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    h = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    u = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - w * h
    return w * h / u if u > 0 else 0.0


def main() -> None:
    root, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    frames: dict[str, dict[str, dict]] = defaultdict(dict)
    stats = defaultdict(int)
    for final in sorted((root / "labels").glob("*_final.json")):
        series_batch = final.name[: -len("_final.json")]
        a = json.load(open(root / "labels" / f"{series_batch}_A.json", encoding="utf-8"))
        b = json.load(open(root / "labels" / f"{series_batch}_B.json", encoding="utf-8"))
        f = json.load(open(final, encoding="utf-8"))
        for key, lab in f.items():
            ep, frame = key.split("/")
            cands = {c["id"]: c["box"] for c in json.load(open(root / ep / "cands.json"))["frames"][frame]}
            heads = []
            for cid, c in lab.get("cands", {}).items():
                ca, cb = a.get(key, {}).get("cands", {}).get(cid, {}), b.get(key, {}).get("cands", {}).get(cid, {})
                differ = (ca.get("class"), ca.get("code")) != (cb.get("class"), cb.get("code"))
                box = cands[int(cid)]
                if box[2] <= box[0] or box[3] <= box[1]:
                    # A detector box squeezed to nothing at the frame edge: no head can be there.
                    assert c["class"] not in HEADS, (key, cid, c)
                    stats["empty_boxes"] += 1
                    continue
                h = {"box": box, "class": c["class"], "flags": c.get("flags", []), "cand": int(cid)}
                if c["class"] in HEADS:
                    h["code"] = c.get("code", "x")
                if differ:
                    h["adjudicated"] = True
                    stats["adjudicated"] += 1
                heads.append(ignore_unnamed_creature(h))
                stats["cands"] += 1
            for m in lab.get("missed", []):
                seen = [
                    any(iou(m["box"], o["box"]) >= 0.3 and o.get("code") == m.get("code") for o in src.get(key, {}).get("missed", []))
                    for src in (a, b)
                ]
                h = {"box": m["box"], "class": m["class"], "code": m.get("code", "x"), "flags": m.get("flags", []), "missed": True}
                if not all(seen):
                    h["adjudicated"] = True
                    stats["adjudicated_missed"] += 1
                heads.append(ignore_unnamed_creature(h))
                stats["missed"] += 1
            frames[ep][frame] = {"heads": heads}
    for ep, fr in frames.items():
        json.dump({"frames": fr}, open(out / f"{ep}.keyframes.json", "w", encoding="utf-8"), indent=0)
        print(ep, len(fr), "frames", sum(len(v["heads"]) for v in fr.values()), "boxes")
    print(dict(stats))


if __name__ == "__main__":
    main()
