# Shapeshifter Solver

A web and command-line solver for [Shapeshifter](https://www.neopets.com/medieval/shapeshifter.phtml), the Neopets placement puzzle.

[Open the browser solver](https://shapeshifter.pages.dev/) to paste a saved game
page and view a step-by-step solution. Search uses all browser-reported CPU cores
when the host enables cross-origin isolation.

## Native quick start

The repository selects its Rust toolchain through [`rust-toolchain`](rust-toolchain).

1. Save an unfinished Shapeshifter page as `data/ShapeShifter.html` using “HTML only.”
2. Run `./solve.sh`.
3. Open `data/solution.html` for the placement guide.

The script builds the parser and solver in release mode and records new complete
puzzles in `data/puzzle_history.jsonl`. To run the stages separately:

```bash
cargo build --release --locked --bin parse --bin solve
target/release/parse data/ShapeShifter.html -o data/puzzle.json
target/release/solve data/puzzle.json --parallel \
  --assets-dir ../web/assets --output data/solution.html
```

## Tools and documentation

Each CLI accepts `--help`.

| Tool | Purpose |
| --- | --- |
| `parse` | Convert saved Neopets HTML into puzzle JSON. |
| `solve` | Solve JSON or JSON Lines and generate an HTML guide. |
| `generate` | Generate reproducible puzzles from the 100 level specifications. |
| `bench` | Measure batches, compare native builds, and summarize results. |
| `examples/measure.rs` | Native driver for `bench run` and `bench compare`. |

- [Solver design](docs/search-algorithms.md): game rules, search schedule, and pruning invariants.
- [Browser development and hosting](docs/browser.md): WASM builds, Cloudflare Pages, and browser tests.
- [Benchmarking](docs/benchmarking.md): the two-minute target, hard-puzzle corpus, and comparisons.

## Development

Native development needs Rust. Browser development also uses Node 24 (see
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
| Native | Formatting, strict Clippy, and release tests for all targets and features, including the measurement example. Tests compile the code, so there is no separate build job. |
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
| `src/bin/` | Parser, solver, generator, and batch benchmark CLIs. |
| `examples/measure.rs` | Native measurement driver with solution replay. |
| `benchmarks/` | Hard-puzzle corpus and reference results. |
| `web/` | Browser app, WASM packages, assets, and browser tooling. |
| `data/` | Level specifications and captured puzzle history. |
| `docs/` | Current solver, browser, and benchmarking guides. |
