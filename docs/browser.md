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

The saved Cloudflare build command forwards to
[`scripts/package-site.sh`](../scripts/package-site.sh), also available as
`npm run build:site`. It packages the static site from `web/` into a clean `dist/`,
including the committed WASM packages, JavaScript, images, favicon, and
[`_headers`](../web/_headers). Cloudflare does not need Rust to deploy.
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
npm run build:wasm
npm run serve
```

Open `http://127.0.0.1:8000/`. The local server serves `web/` and supplies the
isolation headers. A plain static server exercises the single-worker fallback.

[`scripts/build-wasm.sh`](../scripts/build-wasm.sh) uses the pinned Rust toolchain and
[`wasm-pack` version](../scripts/.wasm-pack-version), installing wasm-pack and Rust
sources when needed. Node 24 normalizes generated JavaScript line endings;
this build step uses only Node built-ins.
Commit both `web/pkg/` and `web/pkg-threaded/` after changes to Rust source;
CI rebuilds them and verifies matching checksums.

The threaded package uses `wasm-bindgen-rayon`, an atomics-enabled Rust standard
library, and WASM SIMD. A coordinator worker owns the puzzle and pool; workers
share prepared tables, the search queue, and cancellation state. Cancellation
sets an atomic flag. The single-worker fallback cancels by terminating and
recreating its worker. Shared WASM memory is limited to 2 GiB.

## Browser tests

Use Node 24 (or run `nvm use`), then install the pinned JavaScript Playwright
client and its matching browsers:

```bash
npm ci
npx playwright install --with-deps chromium firefox
npm test
npm run test:browser
```

`npm test` checks the local server, solution replay, seed handling, and generated
file normalization. Browser tests run Chromium and Firefox sequentially; select
one with `npm run test:browser -- --browser firefox`.

To check an isolated deployment, pass `--url`:

```bash
npm run test:browser -- --url https://your-preview.shapeshifter.pages.dev
```

Deployment checks exercise the shared-memory client and public UI. Local checks
also test the single-worker mode and injected startup failure.

Tests and fixtures live in `tests/browser/`; fixtures are read locally and are
not deployed. Tests serve the app with and without isolation headers. They
replay solutions, check invalid-input recovery, enforce a short search budget,
cancel and reuse the solver, check UI responsiveness and image loading, and
force parallel startup failure.
Tests and benchmarks log the actual browser version.

For performance measurements, see [benchmarking](benchmarking.md).
