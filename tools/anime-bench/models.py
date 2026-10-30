"""Fetch the pinned bench models to the layout the harness expects, checking
every byte against `bench/anime/models.tsv` (the same SHA-256 the shipping
manifest will carry).

    uv run python -I models.py <models_tsv> <models_dir> [--check] [--only=<substr>]

Each row lands at `<models_dir>/<repo base>/<variant folder>/<file>`, e.g.
`<models_dir>/anime_head_detection/head_detect_v2.0_s/model.onnx`. Downloads
go through `.part` files; an interrupted run resumes them with a Range
request. `--check` verifies what is on disk without fetching.
"""
import hashlib
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

HOST = "https://huggingface.co"
CHUNK = 1 << 20


def rows(tsv: Path) -> list[dict]:
    out = []
    for line in tsv.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#") or line.startswith("repo"):
            continue
        repo, rev, path, size, sha, note = (line.split("\t") + [""] * 6)[:6]
        out.append({"repo": repo, "rev": rev, "path": path,
                    "size": int(size), "sha256": sha, "note": note})
    return out


def dest_of(root: Path, row: dict) -> Path:
    base = row["repo"].split("/")[-1]
    p = Path(row["path"])
    return root / base / p.parent / p.name


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while b := f.read(CHUNK):
            h.update(b)
    return h.hexdigest()


def verify(path: Path, row: dict) -> str | None:
    """None when the file is good, else the reason it is not."""
    if not path.exists():
        return "missing"
    if path.stat().st_size != row["size"]:
        return f"size {path.stat().st_size} != {row['size']}"
    if digest(path) != row["sha256"]:
        return "sha256 mismatch"
    return None


def fetch(url: str, dest: Path, size: int) -> None:
    part = dest.with_suffix(dest.suffix + ".part")
    part.parent.mkdir(parents=True, exist_ok=True)
    have = part.stat().st_size if part.exists() else 0
    req = urllib.request.Request(url, headers={"User-Agent": "autoscene-anime-bench"})
    if have:
        req.add_header("Range", f"bytes={have}-")
    try:
        with urllib.request.urlopen(req, timeout=60) as r, open(part, "ab") as f:
            got = have
            while b := r.read(CHUNK):
                f.write(b)
                got += len(b)
    except urllib.error.HTTPError as e:
        if have and e.code == 416:  # range unsatisfiable: start over
            part.unlink()
            return fetch(url, dest, size)
        raise
    if part.stat().st_size != size:
        raise OSError(f"short read: {part.stat().st_size} != {size}")
    part.rename(dest)


def main() -> None:
    tsv, root = Path(sys.argv[1]), Path(sys.argv[2])
    check, only = "--check" in sys.argv, next((a.split("=", 1)[1] for a in sys.argv[3:] if a.startswith("--only=")), None)
    todo = [r for r in rows(tsv) if only is None or only in f"{r['repo']}/{r['path']}"]
    bad = []
    for row in todo:
        dest, name = dest_of(root, row), f"{row['repo'].split('/')[-1]}/{row['path']}"
        if (why := verify(dest, row)) is None:
            print(f"ok   {name}", flush=True)
            continue
        if check:
            bad.append(name)
            print(f"BAD  {name}: {why}", flush=True)
            continue
        for attempt in range(3):
            try:
                fetch(f"{HOST}/{row['repo']}/resolve/{row['rev']}/{row['path']}", dest, row["size"])
                break
            except (urllib.error.URLError, OSError) as e:
                print(f"     {name}: {e} (attempt {attempt + 1})", flush=True)
        if (why := verify(dest, row)) is None:
            print(f"got  {name}", flush=True)
        else:
            bad.append(name)
            print(f"BAD  {name}: {why}", flush=True)
    if bad:
        print(json.dumps({"failed": bad}))
        sys.exit(1)


if __name__ == "__main__":
    main()
