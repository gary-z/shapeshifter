#!/bin/bash
set -e
cd "$(dirname "$0")/.."

which wasm-pack >/dev/null 2>&1 || cargo install wasm-pack

# --no-opt: skip wasm-opt (bundled version may be too old for bulk memory ops).
wasm-pack build --target web --out-dir web/pkg --release --no-opt -- --features wasm

echo "Build complete. Serve index.html from project root."
