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

rustup component add rust-src
rust_source_dir="$(rustc --print sysroot)/lib/rustlib/src/rust"
rust_commit="$(rustc -vV | sed -n 's/^commit-hash: //p')"
# Match the prebuilt standard library's paths, including monomorphized code
# whose source locations are resolved through the locally installed rust-src.
printf -v single_flags '%s\x1f' "$wasm_rustflags" \
  "--remap-path-prefix=$rust_source_dir=/rustc/$rust_commit"

# wasm-opt bundled with wasm-pack may not support the generated bulk-memory operations.
CARGO_ENCODED_RUSTFLAGS="${single_flags%$'\x1f'}" \
  wasm-pack build --target web --out-dir web/pkg --release --no-opt -- --locked --features wasm

# Shared-memory Rust needs an atomics-enabled standard library. Keep its flags
# local to this build so native and single-worker builds retain their targets.
thread_flags=(
  "$wasm_rustflags"
  "--remap-path-prefix=$rust_source_dir=/rust-src"
  -C target-feature=+atomics,+bulk-memory,+simd128
  -C link-arg=--shared-memory
  -C link-arg=--max-memory=2147483648
  -C link-arg=--import-memory
  -C link-arg=--export=__wasm_init_tls
  -C link-arg=--export=__tls_size
  -C link-arg=--export=__tls_align
  -C link-arg=--export=__tls_base
)
printf -v encoded_thread_flags '%s\x1f' "${thread_flags[@]}"
CARGO_ENCODED_RUSTFLAGS="${encoded_thread_flags%$'\x1f'}" \
  wasm-pack build --target web --out-dir web/pkg-threaded --release --no-opt -- \
  --locked --features wasm-threads -Z build-std=panic_abort,std

# These packages are served directly and checked into the repository.
rm -f web/pkg/.gitignore web/pkg-threaded/.gitignore

# Copied dependency helpers can contain CRLF. Match .gitattributes so a checkout
# and a fresh build have identical bytes for CI's generated-package check.
python3 - <<'PY'
from pathlib import Path
for package in ('web/pkg', 'web/pkg-threaded'):
    for path in Path(package).rglob('*.js'):
        path.write_bytes(path.read_bytes().replace(b'\r\n', b'\n'))
PY

echo "Build complete. Serve index.html from project root."
