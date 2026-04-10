#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HTML_FILE="$SCRIPT_DIR/data/ShapeShifter.html"
# Relative from data/solution.html to web/assets/
ASSETS_DIR="../web/assets"
JSON_FILE="$SCRIPT_DIR/data/puzzle.json"
HISTORY_FILE="$SCRIPT_DIR/data/puzzle_history.jsonl"
SOLUTION_FILE="$SCRIPT_DIR/data/solution.html"

if [ ! -f "$HTML_FILE" ]; then
    echo "Error: $HTML_FILE not found"
    echo "Save the Neopets Shapeshifter page source as 'data/ShapeShifter.html'."
    echo "(Right-click → Save As, or Ctrl+S → HTML Only)"
    exit 1
fi

# Build binaries if needed
cargo build --release --bin parse --bin solve --manifest-path "$SCRIPT_DIR/Cargo.toml" 2>/dev/null

# Parse HTML to JSON and append to history if new game
echo "Parsing..."
"$SCRIPT_DIR/target/release/parse" "$HTML_FILE" -o "$JSON_FILE" --history "$HISTORY_FILE"

# Solve and generate visual guide
echo ""
"$SCRIPT_DIR/target/release/solve" "$JSON_FILE" --parallel --assets-dir "$ASSETS_DIR" -o "$SOLUTION_FILE"
