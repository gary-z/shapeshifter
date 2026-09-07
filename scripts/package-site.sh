#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
rm -rf dist
mkdir dist
cp web/index.html web/_headers web/favicon.ico web/*.js dist/
for directory in assets pkg pkg-threaded; do
  cp -R "web/$directory" dist/
done
echo "Static site ready in dist/ (uses the committed WASM packages)."
