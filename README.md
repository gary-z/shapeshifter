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

## Game model

- Boards are 3–14 rows by 3–14 columns.
- Each cell stores a value from `0` through `M - 1`, where `M` is 2–5. Zero is the solved value.
- A puzzle has 2–36 fixed-orientation pieces, each at most 5×5 cells.
- Placing a piece decrements every covered cell modulo `M`; pieces may overlap.
- Every piece must be placed exactly once, and the final board must contain only zeroes.

## Solver design

The solver reorders pieces by placement count and shape constraints, then runs depth-first backtracking. Native parallel solves distribute branches through a shared work queue; the WebAssembly build uses the same serial search as the CLI’s default mode.

Before search, 100,000 random placement trajectories are sampled in each direction. The solver first tries bounds derived from the 50th, 75th, 90th, and 95th percentile sample sets, skipping duplicate levels, and finishes with its widest sampled fallback. These levels bound:

- maximum hits to any cell at each depth;
- total board deficit in forward and reverse search;
- horizontal and vertical board jaggedness.

The sampled bounds are combined with deterministic checks:

- remaining piece cells must cover the total deficit;
- checkerboard, row, column, and diagonal partition totals must remain reachable;
- remaining piece perimeter must be able to smooth adjacent-cell differences;
- equivalent effects from consecutive placement pairs are searched once;
- a zero-cell budget discards placements that would exceed the remaining deficit capacity;
- a suffix of single-cell pieces is solved directly.

`--exhaustive` continues after finding a solution and traverses the entire bounded search tree. It does not disable pruning or the sampled bounds.

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
cargo test --release --all-targets
```

To rebuild the browser package, install the WebAssembly target and run:

```bash
rustup target add wasm32-unknown-unknown
./web/build.sh
```

`web/build.sh` installs `wasm-pack` when it is missing and writes the generated package to `web/pkg/`. Serve the repository root with a static HTTP server to test `index.html` locally.
