#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v wasm-pack >/dev/null 2>&1; then
  cargo install wasm-pack
fi

# wasm-opt bundled with wasm-pack may not support the generated bulk-memory operations.
wasm-pack build --target web --out-dir web/pkg --release --no-opt -- --features wasm

echo "Build complete. Serve index.html from project root."
