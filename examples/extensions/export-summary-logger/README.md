# Export Summary Logger (Sample Extension)

A minimal reference extension demonstrating AutoScene's extension architecture.

## Structure
- `extension.json`: Extension manifest declaring ID, version, entrypoint, and capability permissions.
- `run.sh`: Shell script executed out-of-process when an export finishes.

## How it Works
1. When an export finishes, AutoScene writes a structured metadata payload to `$TEMP/export_meta.json`.
2. AutoScene invokes `run.sh` in an isolated process group.
3. The script reads the payload and writes `export_summary.log` directly into the export destination folder.

## Testing Locally
Copy this folder into your AutoScene extensions directory:
- **macOS**: `~/Library/Application Support/AutoScene/extensions/export-summary-logger`
- **Windows**: `%LOCALAPPDATA%\AutoScene\extensions\export-summary-logger`

Open AutoScene, click **Extensions** in the left navigation rail, and you will see **Export Summary Logger** listed!
