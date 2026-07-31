from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from autoscene.api import app
from autoscene.config import settings
from autoscene.gpu import detect_gpu
from autoscene.pipeline import run_scan
from autoscene.project import ProjectStore


def main() -> None:
    parser = argparse.ArgumentParser(prog="autoscene")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("gpu")

    p_serve = sub.add_parser("serve")
    p_serve.add_argument("--host", default=settings.host)
    p_serve.add_argument("--port", type=int, default=settings.port)
    p_serve.add_argument(
        "--api-only",
        action="store_true",
        help="API only (no static UI). Use with Tauri desktop shell.",
    )

    p_scan = sub.add_parser("scan")
    p_scan.add_argument("video")
    p_scan.add_argument("--name", default=None)
    p_scan.add_argument("--backend", default=settings.default_backend)
    p_scan.add_argument("--model", default=settings.default_model)

    args = parser.parse_args()
    if args.cmd == "gpu":
        print(json.dumps(detect_gpu().__dict__, indent=2))
        return
    if args.cmd == "serve":
        import uvicorn

        from autoscene.api import create_app

        host = str(args.host)
        if host not in ("127.0.0.1", "localhost", "::1") and not os.environ.get(
            "AUTOSCENE_ALLOW_LAN"
        ):
            raise SystemExit(
                "Refusing non-loopback bind without AUTOSCENE_ALLOW_LAN=1 "
                "(API has local auth but LAN exposure is still dangerous)."
            )

        create_app(serve_ui=not args.api_only)
        uvicorn.run(app, host=args.host, port=args.port)
        return
    if args.cmd == "scan":
        store = ProjectStore()
        video = Path(args.video)
        project = store.create(
            args.name or video.stem,
            video,
            args.backend,
            args.model,
        )
        run_scan(store, project, on_progress=lambda p, s: print(f"{p:.0%} {s}"))
        print(json.dumps(project.to_dict(), indent=2))
        return


if __name__ == "__main__":
    main()
