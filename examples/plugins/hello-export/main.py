# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Hello Export: an example AutoScene plugin.

AutoScene starts this script for each run with
`uv run --script --locked main.py`, in the plugin's data folder. The first
line on stdin says what to do (the run line); a cancel line may follow.
The script answers on stdout, one JSON object per line: progress and log
lines, then one result or error line. Anything else it prints goes to the
plugin's log. docs/PLUGINS.md describes it all.
"""

import json
import sys
import threading
from datetime import datetime
from pathlib import Path

cancelled = threading.Event()


def send(message: dict) -> None:
    """Writes one line to AutoScene."""
    print(json.dumps(message), flush=True)


def watch_for_cancel() -> None:
    """Reads the lines after the run line: a cancel line asks the run to stop."""
    for line in sys.stdin:
        try:
            if json.loads(line).get("type") == "cancel":
                cancelled.set()
        except (ValueError, AttributeError):
            pass


def main() -> int:
    # AutoScene sets PYTHONUTF8, but a run started by hand may not have it.
    sys.stdin.reconfigure(encoding="utf-8")
    run = json.loads(sys.stdin.readline())
    threading.Thread(target=watch_for_cancel, daemon=True).start()
    action = run["action"]
    if action["kind"] == "command" and action["id"] == "hello":
        return hello(run)
    if action["kind"] == "exportAction" and action["id"] == "clip-list":
        return clip_list(run)
    send({"type": "error", "message": f"Hello Export has no action {action['id']}."})
    return 1


def hello(run: dict) -> int:
    """The palette command: says what it runs with."""
    greeting = run["settings"].get("greeting") or "Hello"
    app, tools = run["app"], run["tools"]
    send({"type": "log", "message": f"FFmpeg is {tools['ffmpeg']}"})
    where = f" in {run['project']['title']}" if run["project"] else ""
    runs_with = f"AutoScene {app['version']} on {app['platform']}{where}, with FFmpeg {tools['ffmpegVersion']}"
    send({"type": "result", "message": f"{greeting} from {runs_with}."})
    return 0


def clip_list(run: dict) -> int:
    """The export action: lists the export's clips in a file in its folder."""
    export, settings = run["export"], run["settings"]
    markdown = settings.get("format") == "markdown"
    with_frames = settings.get("withFrames", True)
    clips = export["clips"]
    created = datetime.fromtimestamp(export["createdAt"] / 1000).strftime("%Y-%m-%d %H:%M")
    title = export["label"] or "Export"
    lines = [f"# {title}" if markdown else title, f"Exported {created} with {export['preset']}."]
    if export["mergedFile"]:
        lines.append(f"Joined into {Path(export['mergedFile']).name}.")
    lines.append("")
    if markdown:
        lines += ["| Clip | Episode | From | To |", "| --- | --- | --- | --- |"]
    for i, clip in enumerate(clips):
        if cancelled.is_set():
            # Any exit will do: AutoScene says the run was cancelled.
            return 1
        send({"type": "progress", "fraction": i / len(clips), "stage": f"Listing {clip['name']}"})
        start, end = when(clip, clip["start"], with_frames), when(clip, clip["end"] - 1, with_frames)
        name = clip["name"] if clip["status"] == "done" else f"{clip['name']} (not made)"
        if markdown:
            cells = [name, clip["episode"], start, end]
            lines.append("| " + " | ".join(c.replace("|", "\\|") for c in cells) + " |")
        else:
            lines.append(f"{name}: {clip['episode']}, {start} to {end}")

    out = Path(export["outDir"]) / ("clips.md" if markdown else "clips.txt")
    try:
        out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    except OSError as e:
        send({"type": "error", "message": f"Couldn't write {out.name} in the export's folder.", "detail": str(e)})
        return 1
    send({"type": "log", "message": f"Wrote {out}"})
    count = f"{len(clips)} clip" if len(clips) == 1 else f"{len(clips)} clips"
    send({"type": "result", "message": f"Wrote {out.name} with {count}."})
    return 0


def when(clip: dict, frame: int, with_frame: bool) -> str:
    """A frame of the clip's episode as a time (and the frame number)."""
    fps = clip["fps"]
    if not fps:
        return f"frame {frame}"
    num, den = (int(n) for n in fps.split("/"))
    seconds = frame * den / num
    time = f"{int(seconds // 3600):02}:{int(seconds // 60 % 60):02}:{seconds % 60:06.3f}"
    return f"{time} (frame {frame})" if with_frame else time


if __name__ == "__main__":
    sys.exit(main())
