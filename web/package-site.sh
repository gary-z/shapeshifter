#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p dist/web
cp index.html _headers dist/
cp web/*.js dist/web/
for directory in assets pkg pkg-threaded; do
  mkdir -p "dist/web/$directory"
  cp -R "web/$directory/." "dist/web/$directory/"
done
echo "Static site ready in dist/ (uses the committed WASM packages)."
