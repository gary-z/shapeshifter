#!/bin/bash
cd "$(dirname "$0")/.."
cargo install wasm-pack 2>/dev/null
wasm-pack build --target web --out-dir web/pkg -- --features wasm
