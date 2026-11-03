# /// script
# requires-python = ">=3.10"
# dependencies = ["truststore==0.10.5"]
# ///
"""Discord Notify: an example AutoScene plugin that talks to the internet.

It posts an export's name and clips to a Discord channel through the
channel's webhook. The webhook's URL is a plain text setting: secret
settings need a newer AutoScene. The script never writes the URL to its
log or its messages. docs/PLUGINS.md describes the protocol.
"""

import http.client
import json
import re
import ssl
import sys
import threading
import urllib.error
import urllib.request
from urllib.parse import urlsplit

import truststore

# Where a webhook may lead. plugin.json names the same hosts, but AutoScene
# doesn't enforce them: this script does, so a mistyped URL posts nowhere.
HOSTS = {"discord.com", "ptb.discord.com", "canary.discord.com", "discordapp.com"}
# A post holds at most 2000 characters.
MAX_POST = 1900
USER_AGENT = "AutoScene-Discord-Notify/1.0 (+https://github.com/tcmmi/AutoScene)"

cancelled = threading.Event()


class Refused(Exception):
    """Why the run can't post: its message for the person, and the detail for the log."""

    def __init__(self, message: str, detail: str | None = None):
        super().__init__(message)
        self.message, self.detail = message, detail


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
    action, settings = run["action"], run["settings"]
    try:
        url = webhook(settings)
        if action["kind"] == "command" and action["id"] == "test":
            post(url, settings, "AutoScene can post here.")
            send({"type": "result", "message": "Posted a test message to Discord."})
        elif action["kind"] == "exportAction" and action["id"] == "post":
            post(url, settings, describe(run["export"], settings.get("listClips", True)))
            send({"type": "result", "message": "Posted the export to Discord."})
        else:
            raise Refused(f"Discord Notify has no action {action['id']}.")
    except Refused as e:
        send({"type": "error", "message": e.message, "detail": e.detail})
        return 1
    return 0


def webhook(settings: dict) -> str:
    """The webhook URL in the settings, once it is one of Discord's."""
    url = (settings.get("webhookUrl") or "").strip()
    if not url:
        raise Refused("Set the webhook URL on Discord Notify's page in Settings first.")
    try:
        parts = urlsplit(url)
        port = parts.port
    except ValueError:
        parts, port = None, None
    if (
        parts is None
        # A space, a line break or a letter outside ASCII: urllib would fail
        # on it with the URL in its error.
        or any(not "!" <= c <= "~" for c in url)
        or parts.scheme != "https"
        or parts.hostname not in HOSTS
        or parts.username is not None
        or port not in (None, 443)
        # /api/webhooks/, or a version's /api/v10/webhooks/.
        or not re.match(r"/api/(v\d+/)?webhooks/", parts.path)
    ):
        raise Refused("That webhook URL isn't a Discord webhook. Copy it again from the channel's settings.")
    return url


def describe(export: dict, list_clips: bool) -> str:
    """The post for an export: its name, how many clips it made, and which."""
    made = [c for c in export["clips"] if c["status"] == "done"]
    missing = len(export["clips"]) - len(made)
    count = "1 clip" if len(made) == 1 else f"{len(made)} clips"
    head = f"**{plain(export['label'] or 'An export')}** is done: {count} with {plain(export['preset'])}."
    if missing:
        head += " 1 wasn't made." if missing == 1 else f" {missing} weren't made."
    lines = [head]
    if list_clips:
        for i, clip in enumerate(made):
            line = f"- {plain(clip['name'])} · {plain(clip['episode'])}"
            if sum(len(l) + 1 for l in lines) + len(line) > MAX_POST:
                lines.append(f"…and {len(made) - i} more.")
                break
            lines.append(line)
    return "\n".join(lines)


def plain(text: str) -> str:
    """Text as Discord shows it, without its Markdown: names may hold * or _."""
    return re.sub(r"([\\*_~`|>])", r"\\\1", text)


def post(url: str, settings: dict, content: str) -> None:
    """Posts `content` to the webhook. Mentions in it never ping anyone."""
    if cancelled.is_set():
        sys.exit(1)
    send({"type": "progress", "stage": "Posting to Discord"})
    body = {"content": content, "allowed_mentions": {"parse": []}}
    if settings.get("username"):
        body["username"] = settings["username"]
    request = urllib.request.Request(
        url,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "User-Agent": USER_AGENT},
        method="POST",
    )
    # The system's certificates, as a browser checks them.
    context = truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    try:
        with urllib.request.urlopen(request, timeout=20, context=context):
            pass
    except urllib.error.HTTPError as e:
        if e.code in (401, 404):
            raise Refused("Discord doesn't know that webhook. Copy it again from the channel's settings.", str(e))
        if e.code == 429:
            wait = e.headers.get("Retry-After", "a few")
            raise Refused(f"Discord asks to wait {wait} seconds. Try again then.", str(e))
        raise Refused("Discord refused the post.", f"{e}: {e.read(500).decode(errors='replace')}")
    except (urllib.error.URLError, OSError) as e:
        raise Refused("Couldn't reach Discord. Check the internet connection.", str(e))
    except (http.client.HTTPException, ValueError) as e:
        # A broken answer, say. Its text may hold the URL: only its kind is logged.
        raise Refused("Couldn't post to Discord.", type(e).__name__)


if __name__ == "__main__":
    sys.exit(main())
