#!/usr/bin/env sh
set -e

# AutoScene Export Summary Logger
# Receives the export metadata JSON via the AUTOSCENE_META environment variable or argument.

META_FILE="${AUTOSCENE_META:-$1}"

echo "========================================"
echo " [AutoScene Extension] Export Logger"
echo "========================================"

if [ -f "$META_FILE" ]; then
    echo "Metadata payload located at: $META_FILE"
    echo "Contents:"
    cat "$META_FILE"
    echo ""
    # Write a summary log in the output directory
    echo "Export completed successfully at $(date)" > "export_summary.log"
    echo "Logged to export_summary.log"
else
    echo "No metadata file provided, executing standalone test."
    echo "Current working directory: $(pwd)"
fi

echo "Extension finished successfully."
exit 0

