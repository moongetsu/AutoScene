#!/usr/bin/env sh
set -e

# AutoScene Desktop Notification & Sound Extension
# Plays a sound and shows a native macOS notification dialog when run.

TITLE="AutoScene Extension"
MESSAGE="Export completed! Extension executed successfully."

# 1. Native macOS Notification Banner
if command -v osascript >/dev/null 2>&1; then
    osascript -e "display notification \"$MESSAGE\" with title \"$TITLE\" sound name \"Glass\""
fi

# 2. Touch a proof file in the current folder
echo "Extension ran at $(date)" > "extension_test_success.txt"

echo "Extension execution complete!"
exit 0
