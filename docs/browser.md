# Browser development and hosting

The [browser solver](https://shapeshifter.pages.dev/) uses the
[same search schedule](search-algorithms.md) as native parallel search. One game
gets the entire worker pool, defaulting to `navigator.hardwareConcurrency`.
The browser may report fewer workers than the machine's logical CPU count.

The search limit is 120 seconds; puzzle preparation and WASM/pool startup are
measured separately. Rust checks deadlines inside search, and the UI has a
recovery watchdog. The UI stays responsive and supports cancellation.

## Hosting

Cloudflare Pages serves the app as a static site:

| Setting | Value |
| --- | --- |
| Production branch | `main` |
| Build command | `bash web/package-site.sh` |
| Output directory | `dist` |

The packaging script copies the committed WASM packages, JavaScript, local
assets, and [`_headers`](../_headers). Cloudflare does not need Rust to deploy.
The headers enable shared memory on HTTPS or localhost:

```text
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Keep WASM files, worker scripts, and images on the same origin. Verify that the
page reports “Ready · N search workers.” Other static hosts need the same
headers for parallel search. Without cross-origin isolation, or if pool startup
fails, the app uses one worker with the same search policy.

## Build and serve

```bash
rustup target add wasm32-unknown-unknown
./web/build.sh
python3 web/serve.py
```

Open `http://127.0.0.1:8000/`. The local server supplies the isolation headers.
A plain static server exercises the single-worker fallback.

`web/build.sh` uses the pinned Rust toolchain and
[`wasm-pack` version](../web/.wasm-pack-version), installing wasm-pack and Rust
sources when needed. Python 3 normalizes generated JavaScript line endings.
Commit both `web/pkg/` and `web/pkg-threaded/` after changes to Rust source;
CI rebuilds them and verifies matching checksums.

The threaded package uses `wasm-bindgen-rayon`, an atomics-enabled Rust standard
library, and WASM SIMD. A coordinator worker owns the puzzle and pool; workers
share prepared tables, the search queue, and cancellation state. Cancellation
sets an atomic flag. The single-worker fallback cancels by terminating and
recreating its worker. Shared WASM memory is limited to 2 GiB.

## Browser tests

Install the Python test client and Chromium:

```bash
python3 -m venv /tmp/shapeshifter-browser
/tmp/shapeshifter-browser/bin/pip install -r web/requirements.txt
/tmp/shapeshifter-browser/bin/playwright install --with-deps chromium
/tmp/shapeshifter-browser/bin/python web/test_browser.py
```

Firefox tests use the official Playwright 1.63 browser with the pinned Python
1.62 client, because that browser keeps optimizing WASM compilation enabled
under automation. With Node.js 24 and npm installed:

```bash
npm install --prefix /tmp/shapeshifter-firefox --no-package-lock --ignore-scripts --no-audit --no-fund playwright-core@1.63.0
PLAYWRIGHT_BROWSERS_PATH=/tmp/shapeshifter-firefox/browsers \
  node /tmp/shapeshifter-firefox/node_modules/playwright-core/cli.js install --with-deps firefox
FIREFOX_EXECUTABLE_PATH=$(PLAYWRIGHT_BROWSERS_PATH=/tmp/shapeshifter-firefox/browsers \
  node -e "console.log(require('/tmp/shapeshifter-firefox/node_modules/playwright-core').firefox.executablePath())")
/tmp/shapeshifter-browser/bin/python web/test_browser.py --browser firefox \
  --executable-path "$FIREFOX_EXECUTABLE_PATH"
```

Tests serve the app with and without isolation headers. They replay solutions,
check invalid-input recovery, enforce a short search budget, cancel and reuse
the solver, check UI responsiveness, and force parallel startup failure. Both
scripts log the actual browser version.

For performance measurements, see [benchmarking](benchmarking.md).
