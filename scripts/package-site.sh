#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
rm -rf dist
mkdir dist
cp web/*.html web/_headers web/favicon.ico web/*.js dist/
for directory in pkg pkg-threaded; do
  cp -R "web/$directory" dist/
done
echo "Static site ready in dist/ (uses the committed WASM packages)."
