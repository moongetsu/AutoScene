# Desktop Notifier & Sound Alert (Sample Extension)

A lightweight sample extension for AutoScene that plays a sound chime and triggers a desktop notification when an export finishes.

## Files
- `extension.json`: Extension manifest declaring ID, name, kind, entrypoint, and permissions.
- `run.sh`: Executable script triggered out-of-process.

## How it Works
When triggered by AutoScene:
1. Plays the native audio chime.
2. Triggers a macOS / desktop notification banner.
3. Touches `extension_test_success.txt` to confirm execution.
