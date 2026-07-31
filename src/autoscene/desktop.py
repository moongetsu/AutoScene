from __future__ import annotations

import argparse
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import uvicorn

from autoscene.api import app, create_app
from autoscene.config import settings


def _port_open(host: str, port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.4)
        return sock.connect_ex((host, port)) == 0


def _api_alive(url: str) -> bool:
    try:
        with urllib.request.urlopen(f"{url.rstrip('/')}/api/health", timeout=1.5) as res:
            return res.status == 200
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


def _launch_tauri() -> int:
    ui_dir = Path(__file__).resolve().parents[2] / "ui"
    if not ui_dir.exists():
        raise SystemExit(f"UI folder missing: {ui_dir}")
    npm = "npm.cmd" if sys.platform == "win32" else "npm"
    return subprocess.call([npm, "run", "tauri", "dev"], cwd=ui_dir)


def main() -> None:
    parser = argparse.ArgumentParser(description="AutoScene desktop (legacy API launcher)")
    parser.add_argument("--host", default=settings.host)
    parser.add_argument("--port", type=int, default=settings.port)
    parser.add_argument("--api-only", action="store_true", help="Serve API only")
    parser.add_argument(
        "--tauri",
        action="store_true",
        help="Start API then open Tauri shell (preferred desktop entry)",
    )
    args = parser.parse_args()

    url = f"http://{args.host}:{args.port}"

    if args.tauri:
        if not (_port_open(args.host, args.port) and _api_alive(url)):
            create_app(serve_ui=False)

            def run_server() -> None:
                config = uvicorn.Config(
                    app,
                    host=args.host,
                    port=args.port,
                    log_level="info",
                )
                server = uvicorn.Server(config)
                server.run()

            import threading

            thread = threading.Thread(target=run_server, daemon=True)
            thread.start()
            for _ in range(40):
                if _api_alive(url):
                    break
                if not thread.is_alive():
                    raise SystemExit(f"API failed to start on {url}")
                time.sleep(0.15)
            else:
                raise SystemExit(f"API not ready at {url}")
        raise SystemExit(_launch_tauri())

    if _port_open(args.host, args.port) and _api_alive(url):
        print(f"AutoScene API already running at {url}")
        print("Desktop: cd ui && npm run tauri dev")
        return

    create_app(serve_ui=not args.api_only)
    print(f"Starting API at {url}")
    print("Desktop shell: cd ui && npm run tauri dev")
    uvicorn.run(app, host=args.host, port=args.port)


if __name__ == "__main__":
    main()
