# Browser search and hosting

The browser runs the same adaptive, regional, and guided search algorithms as
native parallel search, with the same phase budgets and thresholds. One game
gets the entire worker pool. The pool size defaults to
`navigator.hardwareConcurrency`, which the browser can report below the machine's
logical CPU count. The UI remains responsive and supports cancellation.
See [`hardwareConcurrency`](https://developer.mozilla.org/en-US/docs/Web/API/WorkerNavigator/hardwareConcurrency)
for the browser's CPU-reporting contract.

The search limit is 120 seconds. Puzzle preparation and initial WASM/pool startup
are measured separately and do not consume that limit. Rust checks deadlines
inside search; the UI also has a recovery watchdog for an unresponsive worker.
As on native, small cooperative deadline overruns are possible while completing
a search operation or joining workers.

The [measured browser sample](benchmarks/2026-09-06-browser/README.md) solved
45/50 puzzles with 32 workers, matching the native outcome for every seed. The
longest successful browser search was 69.6 seconds. This covers ten selected
levels, with five seeds each; it does not establish a new all-level success rate.

## Cloudflare Pages

Cloudflare Pages can serve the app as a static site, with no server-side solver:

1. Connect this repository to Pages.
2. Set the production branch to `main`.
3. Set the build command to `bash web/package-site.sh`.
4. Set the build output directory to `dist`.

This packages the committed WASM builds, JavaScript, and local image assets. It
does not require Rust in Cloudflare's build environment. Rebuild and commit the
WASM packages when changing Rust source.

The root [`_headers`](../_headers) file is copied to `dist` and sets:

```text
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Cloudflare [applies `_headers` to static Pages responses](https://developers.cloudflare.com/pages/configuration/headers/).
These headers, together with HTTPS (or localhost for development), allow
[`SharedArrayBuffer` across browser workers](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer).
Keep the WASM files, worker scripts, and images on the same origin. Verify that
the page says “Ready · N search workers” after deployment. No DNS or hosting
changes are made by the packaging script.

Other static hosts work if they can set these response headers. GitHub Pages
does not provide a custom response-header configuration, so its ordinary
deployment uses one worker. The app also falls back to one worker if the
shared-memory pool fails to initialize. Both modes run the same search policy;
the fallback naturally has less compute available.

## Build and test

The build script requires Python 3 to normalize generated JavaScript line endings.

```bash
rustup target add wasm32-unknown-unknown
./web/build.sh
python3 web/serve.py
```

Open `http://127.0.0.1:8000/`. The threaded build uses
[`wasm-bindgen-rayon`](https://github.com/RReverser/wasm-bindgen-rayon), an
atomics-enabled Rust standard library, and WASM SIMD. A coordinator worker owns
the puzzle and the pool; search workers share prepared tables, the search queue,
and cancellation state. Cancellation writes an atomic flag from the UI. The
single-worker fallback terminates its worker on cancellation and recreates it
for the next puzzle.

Install browser test dependencies in a virtual environment:

```bash
python3 -m venv /tmp/shapeshifter-browser
/tmp/shapeshifter-browser/bin/pip install -r web/requirements.txt
/tmp/shapeshifter-browser/bin/playwright install --with-deps chromium
/tmp/shapeshifter-browser/bin/python web/test_browser.py
```

For Firefox, use the official browser from Playwright 1.63.0. The Python package
is currently pinned to 1.62.0, whose bundled Firefox debugger disables the
optimizing WASM compiler. Ordinary Firefox JIT preferences do not override this.
Playwright 1.63.0 sets `allowUnobservedWasm` on its debuggers in
[`Runtime.js`](https://github.com/microsoft/playwright/blob/v1.63.0/browser_patches/firefox/juggler/content/Runtime.js)
and [`WorkerMain.js`](https://github.com/microsoft/playwright/blob/v1.63.0/browser_patches/firefox/juggler/content/WorkerMain.js).

With Node.js 24 and npm installed, install that browser into a separate cache:

```bash
npm install --prefix /tmp/shapeshifter-firefox --no-package-lock --ignore-scripts --no-audit --no-fund playwright-core@1.63.0
PLAYWRIGHT_BROWSERS_PATH=/tmp/shapeshifter-firefox/browsers \
  node /tmp/shapeshifter-firefox/node_modules/playwright-core/cli.js install --with-deps firefox
FIREFOX_EXECUTABLE_PATH=$(PLAYWRIGHT_BROWSERS_PATH=/tmp/shapeshifter-firefox/browsers \
  node -e "console.log(require('/tmp/shapeshifter-firefox/node_modules/playwright-core').firefox.executablePath())")
/tmp/shapeshifter-browser/bin/python web/test_browser.py --browser firefox \
  --executable-path "$FIREFOX_EXECUTABLE_PATH"
```

CI uses this same executable override. It keeps the Python 1.62 test client and
validates it against the official Firefox 155.0 build from Playwright 1.63;
the browser files and JIT preferences are unchanged. Once Python Playwright
1.63 is available, update the Python pin and remove the separate Node installer.
The scripts log the actual browser version so results identify the browser used.

Tests start local servers with and without isolation headers. They replay
solutions in original piece order, check invalid-input recovery, enforce a short
search budget, cancel and reuse the solver, check UI responsiveness, and force
parallel startup failure to exercise fallback.

Benchmark reproducible puzzles with all browser-reported CPUs, one at a time:

```bash
target/release/generate 50 --seed 50100 --count 5 > /tmp/puzzles.jsonl
/tmp/shapeshifter-browser/bin/python web/bench_browser.py /tmp/puzzles.jsonl \
  --output /tmp/browser-results.jsonl
```

The JSONL records preparation and search independently and validates every
reported solution. `--threads 1` measures the same shared-memory build with one
worker. To benchmark the fixed Firefox build, pass `--browser firefox` and
`--executable-path "$FIREFOX_EXECUTABLE_PATH"` using the path obtained above.
Use a foreground browser on an otherwise idle machine when assessing
performance. Browser CPU reporting, scheduling, memory limits, and WASM code
generation can differ from native; worker support alone does not guarantee the
same solve rate within two minutes. The shared WASM build permits up to 2 GiB of
linear memory, and a memory exhaustion or worker failure is reported by the UI.
