# Shapeshifter Solver

A browser solver for [Shapeshifter](https://www.neopets.com/medieval/shapeshifter.phtml), the Neopets placement puzzle.

[Open the browser solver](https://shapeshifter.pages.dev/) to paste a saved game
page and view a step-by-step solution. Search uses all browser-reported CPU cores
when the host enables cross-origin isolation.

## Tools and documentation

Use `generate --help` or `bench --help` for command-line options.

| Tool | Purpose |
| --- | --- |
| `generate` | Generate reproducible puzzles from the 100 level specifications. |
| `bench` | Measure batches, compare native builds, and summarize results. |
| `measure` | Native worker for `bench run` and `bench compare`, reading puzzle JSON from stdin. |

- [Solver design](docs/search-algorithms.md): game rules, search schedule, and pruning invariants.
- [Browser development and hosting](docs/browser.md): WASM builds, Cloudflare Pages, and browser tests.
- [Benchmarking](docs/benchmarking.md): the two-minute target, hard-puzzle corpus, and comparisons.

## Development

Native development uses the Rust toolchain pinned in [`rust-toolchain`](rust-toolchain).
Browser development also uses Node 24 (see
[`.nvmrc`](.nvmrc)); npm dependencies are pinned in `package-lock.json`.

Run the native checks locally:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --release --locked --all-targets --all-features
```

[CI](.github/workflows/ci.yml) has three independent jobs:

| Job | Coverage |
| --- | --- |
| Native | Formatting, strict Clippy, and release tests for all targets and features, including the native tools. Tests compile the code, so there is no separate build job. |
| WebAssembly | Rebuild both committed WASM packages and compare their checksums. |
| Browser | Chromium and Firefox integration tests, including parallel search, single-worker fallback, cancellation, and solution replay. |

CI runs on pull requests and pushes to `main`, skipping changes limited to
Markdown or `docs/`. New commits cancel superseded runs.
The full performance corpus is run manually on the target machine; CI checks
correctness and browser behavior without asserting machine-dependent timings.

## Repository layout

| Path | Contents |
| --- | --- |
| `src/core/` | SIMD bitboard, board, and piece representations. |
| `src/solver/` | Search algorithms, preparation, and pruning bounds. |
| `src/bin/` | Puzzle generator, benchmark CLI, and native measurement driver. |
| `web/` | Static site: HTML, JavaScript, WASM packages, images, and hosting headers. |
| `scripts/` | Build, packaging, serving, and browser benchmark tools. |
| `tests/` | Rust integration tests and browser tests with local fixtures. |
| `benchmarks/` | Hard-puzzle corpus, captured puzzle history, and reference results. |
| `data/` | Level specifications used by the generator. |
| `docs/` | Current solver, browser, and benchmarking guides. |
