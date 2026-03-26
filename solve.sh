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

# Append to history if this is a fresh game (not already in progress).
# Compare piece count in the parsed puzzle against the level spec.
COMPACT=$(python3 -c "import json,sys;print(json.dumps(json.load(open(sys.argv[1])),separators=(',',':')))" "$JSON_FILE")
IN_PROGRESS=$(python3 -c "
import json, sys
puzzle = json.load(open(sys.argv[1]))
levels = json.load(open(sys.argv[2]))
spec = next((l for l in levels if l['level'] == puzzle['level']), None)
if spec and len(puzzle['pieces']) < spec['shapes']:
    print('yes')
else:
    print('no')
" "$JSON_FILE" "$SCRIPT_DIR/data/levels.json")
touch "$HISTORY_FILE"
if [ "$IN_PROGRESS" = "yes" ]; then
    echo "Game already in progress (fewer pieces than expected). Skipping history."
elif ! grep -qFx "$COMPACT" "$HISTORY_FILE"; then
    echo "$COMPACT" >> "$HISTORY_FILE"
fi

# Build solver if needed
cargo build --release --bin solve --manifest-path "$SCRIPT_DIR/Cargo.toml" 2>/dev/null

# Solve and generate visual guide
echo ""
"$SCRIPT_DIR/target/release/solve" "$JSON_FILE" --parallel --assets-dir "$ASSETS_DIR" -o "$SOLUTION_FILE"
