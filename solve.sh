#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HTML_FILE="$SCRIPT_DIR/data/ShapeShifter.html"
ASSETS_DIR="https://images.neopets.com/medieval/shapeshifter"
JSON_FILE="$SCRIPT_DIR/data/puzzle.json"
HISTORY_FILE="$SCRIPT_DIR/data/puzzle_history.jsonl"
SOLUTION_FILE="$SCRIPT_DIR/data/solution.html"

if [ ! -f "$HTML_FILE" ]; then
    echo "Error: $HTML_FILE not found"
    echo "Save the Neopets Shapeshifter page source as 'data/ShapeShifter.html'."
    echo "(Right-click → Save As, or Ctrl+S → HTML Only)"
    exit 1
fi

# Parse HTML to JSON
echo "Parsing..."
python3 "$SCRIPT_DIR/tools/parse_html.py" "$HTML_FILE" "$JSON_FILE"

# Append to history
cat "$JSON_FILE" >> "$HISTORY_FILE"
echo "" >> "$HISTORY_FILE"

# Build solver if needed
cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml" 2>/dev/null

# Solve and generate visual guide
echo ""
"$SCRIPT_DIR/target/release/shapeshifter" "$JSON_FILE" --assets-dir "$ASSETS_DIR"

echo ""
echo "Solution: $SOLUTION_FILE"
