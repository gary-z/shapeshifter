# Benchmarking

The optimization target is the probability of solving a randomly generated hard
puzzle within **120 seconds total wall time on the native machine, using all
available CPU cores**. Preparation, search, and the driver's solution replay
count toward that deadline. Run one puzzle at a time on an otherwise idle
machine. Improvements qualify at roughly 10% faster solves or 10% fewer failures.

## Native measurement

Build the driver with the repository's pinned toolchain and release profile:

```bash
cargo build --release --locked --bin bench --example measure
target/release/bench run --binary target/release/examples/measure \
  --puzzles benchmarks/puzzles.jsonl --output /tmp/native-results.jsonl \
  --variant current
target/release/bench summarize /tmp/native-results.jsonl
```

The Rust runner uses all available CPUs through `RAYON_NUM_THREADS` and kills
the native worker at the deadline. The driver reports preparation, search,
visited nodes, and placements; the runner independently replays every solution.
Records include puzzle and binary SHA-256 hashes. Puzzle hashes use sorted,
compact JSON with exact integer seeds. Failures and late solutions
count as 120 seconds in the capped mean. `--timeout` changes the total budget.

The checked-in [corpus](../benchmarks/puzzles.jsonl) contains 32 puzzles from
16 difficult levels, with fixed seeds. The [reference results](../benchmarks/reference.jsonl)
for solver revision [5faef85](https://github.com/gary-z/shapeshifter/commit/5faef85)
were measured on 2026-09-07 on an Intel Core i9-13900KF with 32 logical CPUs:
**32/32 solved, 12.01 seconds mean total time**.

```bash
target/release/bench summarize benchmarks/reference.jsonl
```

Use this corpus for repeatable regression checks. For acceptance measurements,
generate a fresh sample from the difficult levels with unused seeds:

```bash
cargo build --release --locked --bin generate
target/release/generate 80 --seed 80900 --count 10 > /tmp/fresh-puzzles.jsonl
```

The generator uses consecutive seeds starting at `--seed`. Keep the JSONL input
and seed command with the results. A small sample's success rate does not
establish performance across all generated puzzles.

## Comparing changes

Build and copy the measurement executable from each revision before running:

```bash
target/release/bench compare --baseline /tmp/measure-baseline \
  --candidate /tmp/measure-candidate --puzzles benchmarks/puzzles.jsonl \
  --output /tmp/paired
target/release/bench summarize /tmp/paired/results.jsonl
```

The runner alternates baseline/candidate order by puzzle and runs them
sequentially. Repeating the command resumes completed cases after checking
binary hash, puzzle hash, and timeout. Use a new output directory for a different
comparison. Summaries report success counts and capped mean time, with paired
time reduction when both variants are present. Keep CPU availability, compiler,
release settings, and timeout the same between builds.

## Batch and browser tools

The `bench simulated` and `bench historical` modes survey generated levels or
captured puzzle history. Their timeout starts **after preparation**, using a different deadline from the
total-time measurement above:

```bash
cargo build --release --locked --bin bench --bin solve
target/release/bench simulated 1 100 --parallel \
  --games-per 10 --seed-offset 100 --timeout 120
```

With `--parallel`, each game uses all cores and games run sequentially. Seeds are
`level * 1000 + Game`, where `Game` starts at `--seed-offset`. Preparation and
search are reported separately.

After [installing browser dependencies](browser.md#browser-tests), measure the
same JSONL puzzles in the browser:

```bash
npm run bench:browser -- benchmarks/puzzles.jsonl \
  --output /tmp/browser-results.jsonl
```

Browser measurements also use a search-only timeout and replay every solution.
They default to all browser-reported CPUs; `--threads 1` uses one worker in the
shared-memory build. For Firefox, add `--browser firefox`. Browser scheduling,
CPU reporting, and WASM code generation can affect results independently of
native performance.
