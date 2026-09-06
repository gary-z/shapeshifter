# Shapeshifter Solver

A web and command-line solver for [Shapeshifter](https://www.neopets.com/medieval/shapeshifter.phtml), the Neopets placement puzzle.

- [Open the browser solver](https://gary-z.github.io/shapeshifter/) (single-threaded WebAssembly)
- Use the native CLI for parallel search and HTML solution guides

## Quick start

The repository selects the Rust nightly toolchain through [`rust-toolchain`](rust-toolchain). To solve a live puzzle from the command line:

1. Save the Shapeshifter page as `data/ShapeShifter.html` using “HTML only.” Save it before completing the puzzle.
2. Run:

   ```bash
   ./solve.sh
   ```

The script builds the parser and solver in release mode, records new complete puzzles in `data/puzzle_history.jsonl`, and writes a step-by-step guide to `data/solution.html`.

To run the two stages separately:

```bash
cargo build --release --bin parse --bin solve
target/release/parse data/ShapeShifter.html -o data/puzzle.json
target/release/solve data/puzzle.json \
  --parallel \
  --assets-dir ../web/assets \
  --output data/solution.html
```

Both binaries accept `--help`. The solver reads one JSON object from a file or standard input, and it also accepts JSON Lines on standard input.

## Included tools

| Binary | Purpose |
| --- | --- |
| `parse` | Convert saved Neopets HTML into the solver’s JSON format. |
| `solve` | Solve JSON input and generate an HTML guide. |
| `generate` | Generate reproducible puzzles from the 100 embedded level specifications. |
| `bench` | Run generated or historical puzzle batches through the release solver. |

For example, this generates five level-10 puzzles as JSON Lines:

```bash
cargo run --release --bin generate -- 10 --count 5
```

To measure the two-minute search target across all levels:

```bash
cargo build --release --bin bench --bin solve
target/release/bench simulated 1 100 --parallel \
  --games-per 10 --seed-offset 100 --timeout 120
```

Parallel benchmarks run one game at a time, giving that game all available CPU
cores. The timeout starts after preparation; the table reports preparation and
search separately, and the summary lists levels with less than 50% success in
the sample. Generated puzzle seeds are `level * 1000 + Game`, where the `Game`
column starts at `--seed-offset`. Use a new offset for a fresh sample.

The [uniform search benchmark report](docs/benchmarks/2026-09-06-uniform/README.md)
records 494/500 solves, with at least 3/5 at every level, plus 20/20 fresh puzzles
at levels 50 and 80. Every case has a 120-second search cap, excluding
preparation. It includes matched Monte Carlo controls, the accepted regression
at one level-68 seed, raw timings, and reproduction details.

## Game model

- Boards are 3–14 rows by 3–14 columns.
- Each cell stores a value from `0` through `M - 1`, where `M` is 2–5. Zero is the solved value.
- A puzzle has 2–36 fixed-orientation pieces, each at most 5×5 cells.
- Placing a piece decrements every covered cell modulo `M`; pieces may overlap.
- Every piece must be placed exactly once, and the final board must contain only zeroes.

## Solver design

Native parallel, non-exhaustive solves use the same schedule for every board:
five seconds of the existing backtracker, 25 seconds of adaptive backtracking,
and 25 seconds of regional inference, followed by fallback search. Each phase uses all available
workers on the same game and stops as soon as a solution is found.

Adaptive placement domains shrink as pieces are fixed, and exact modular
coverage bounds force or forbid cells in each remaining piece. Single-cell
probability distributions guide piece and placement
ordering. Workers vary those choices and restart with growing node budgets.

Regional inference chooses tile shapes under a budget of 1,024 joint states per
region. It uses disjoint 2x2, 2x3, and 3x2 regions when six cells fit that budget;
otherwise it uses 2x2, 1x4, and 4x1 regions. Regions retain the joint
effect of each piece's placement on neighboring cells. Fourier convolution computes
messages between regions and pieces; those messages guide successive placement choices.
Workers cover different region partitions and damping levels, then try randomized
restarts. Impossible partial assignments are rejected by the exact remaining-area
bound. Both searches check every reported solution against all board cells and
return placements in the original piece order.

See [the algorithm notes](docs/search-algorithms.md) for the exact filtering
invariants and the inference model.

Backtracking orders pieces by placement count and shape constraints.
Native backtracking distributes branches through a shared work queue; the
WebAssembly build uses the same serial search as the CLI's default mode.

Monte Carlo trajectory sampling, per-cell hit counters, and percentile retries
have been removed. Backtracking uses exact bounds throughout.

Native parallel fallback also builds a 200,000-state guided frontier through the
first eight pieces when its 3x3 windows fit a budget of 32,768 states per window
(`M^9`). This limit applies equally to every board size and piece count.
Exact suffix distributions on small overlapping regions
rank spatially coherent partial boards. Workers expand each layer in parallel
and merge their candidates into the same global frontier. The guided phase,
including frontier construction and DFS, has a 30-second budget. If it misses,
backtracking restarts from the root with the exact bounds.

The backtracker applies these deterministic checks:

- remaining piece cells must cover the total deficit;
- checkerboard, row, column, and diagonal partition totals must remain reachable;
- remaining piece perimeter must be able to smooth adjacent-cell differences;
- cell-set reachability and small-component bounds reject impossible residual boards;
- equivalent effects from consecutive placement pairs are searched once;
- a zero-cell budget discards placements that would exceed the remaining deficit capacity;
- a suffix of single-cell pieces is solved directly.

`--exhaustive` uses backtracking with exact pruning and continues after finding a
solution. Probability models and restarts guide the bounded search phases;
their work counts as search time, including failed attempts and guided frontier
construction. Adaptive domain filtering uses exact bounds; probabilities only
change branch ordering.

Library callers can separate timing with `solver::prepare(game, parallel,
exhaustive)` followed by `PreparedSearch::solve()`. The benchmark worker protocol
prints `READY preparation_ms` before search, then `nodes search_ms solved` when
search finishes.

## Repository layout

| Path | Contents |
| --- | --- |
| `src/core/` | SIMD bitboard, board, and piece representations. |
| `src/solver/` | Serial and parallel search, preparation, and pruning bounds. |
| `src/bin/` | Parser, solver, generator, and benchmark CLIs. |
| `web/` | Browser parser, shared board renderer, and WebAssembly integration. |
| `data/levels.json` | The 100 level specifications used by the generator. |
| `data/puzzle_history.jsonl` | Captured puzzles used for regression and benchmark input. |

## Development

Run the CI checks locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --release --all-targets
```

To rebuild the browser package, install the WebAssembly target and run:

```bash
rustup target add wasm32-unknown-unknown
./web/build.sh
```

The repository pins Rust in `rust-toolchain` and `wasm-pack` in `web/.wasm-pack-version`. `web/build.sh` installs the pinned `wasm-pack` version when needed and writes the generated package to `web/pkg/`. Serve the repository root with a static HTTP server to test `index.html` locally.
