#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
wasm_pack_version="$(tr -d '[:space:]' < "$repo_dir/web/.wasm-pack-version")"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
wasm_rustflags="--remap-path-prefix=$cargo_home=/cargo"

installed_wasm_pack_version=""
if command -v wasm-pack >/dev/null 2>&1; then
  installed_wasm_pack_version="$(wasm-pack --version | awk '{print $2}')"
fi

if [[ "$installed_wasm_pack_version" != "$wasm_pack_version" ]]; then
  cargo install wasm-pack --version "$wasm_pack_version" --locked
fi

cd "$repo_dir"

# wasm-opt bundled with wasm-pack may not support the generated bulk-memory operations.
CARGO_ENCODED_RUSTFLAGS="$wasm_rustflags" \
  wasm-pack build --target web --out-dir web/pkg --release --no-opt -- --locked --features wasm

echo "Build complete. Serve index.html from project root."
