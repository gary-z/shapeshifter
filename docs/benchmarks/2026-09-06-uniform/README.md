# Uniform search and Monte Carlo retirement

The solver uses a common phase schedule instead of board-area and piece-count
thresholds. With Monte Carlo removed, the frozen build solved **494/500** fixed
sample puzzles, with at least **3/5 at every level from 1 through 100**, under a
120-second search cap. The distribution was 95 levels at 5/5, four at 4/5, and
one at 3/5. Levels 68, 73, 75, and 100 scored 4/5; level 80 scored 3/5.
Fresh ten-game samples at levels 50 and 80 both scored **10/10**. Monte Carlo is
retired under this measured budget tradeoff, with its one remaining unique
solve in the paired controls documented below.

## Search policy

Every native parallel, non-exhaustive solve uses the following schedule, stopping
as soon as a solution is found:

1. Five seconds of exact backtracking.
2. Twenty-five seconds of adaptive domain search.
3. Twenty-five seconds of regional inference.
4. Thirty seconds of guided frontier construction and backtracking, when its
   tables fit the state budget.
5. Exact backtracking from the root.

Regional factors have a 1,024-state budget. Six-cell regions fit for M=2 and M=3;
four-cell regions are used for M=4 and M=5. Guided 3x3 windows have a 32,768-state
budget (`M^9`), admitting M=2 and M=3. These decisions do not depend on level,
board area, or piece count. Root fallback is unbounded in the solver; the
benchmark enforces the overall 120-second search limit.

Simply broadening the previous schedule caused a regression: serial construction
of the guided frontier delayed useful fallback search. The frontier now expands
parent states in parallel and merges the global best 200,000 candidates at each
depth. A worker keeps up to the full beam width locally, so its assigned parents
cannot lose a globally competitive candidate to a per-worker quota. Stable
parent and placement ranks preserve tie order. Both construction and subsequent
DFS count against the guided phase's deadline.

Monte Carlo's 100,000 sampled trajectories in each direction, sampled deficit
and jaggedness envelopes, per-cell hit histories, and percentile retries are
removed. The exact jaggedness bound formerly inside the Monte Carlo wrapper is
preserved. The other exact feasibility bounds remain in use. See the
[algorithm notes](../../search-algorithms.md) for the search and pruning details.

## What the Monte Carlo comparison measures

The control has the same common schedule, state budgets, and parallel guided
frontier as the retirement build. It retains Monte Carlo sampling and all its
integration. Their search implementations differ only in the six source files
needed to remove Monte Carlo; their benchmark and generator executables are
identical. The embedded HTML viewer also has different line endings, recorded
in the manifest; that viewer is unused by the benchmark worker.

The planned native comparison uses all five fixed-sample puzzles at levels
40, 50, 51, 75, 80, 85, and 100. A separate five-puzzle diagnostic at level 68 was
added after the retirement run's first timeout, to check that particular loss.
The diagnostic is identified separately in the paired records.

The planned comparison scored **31/35 with and without Monte Carlo**, with
identical solve/timeout outcomes for every seed:

| Level | With Monte Carlo | Retired |
| --- | ---: | ---: |
| 40 | 5/5 | 5/5 |
| 50 | 5/5 | 5/5 |
| 51 | 5/5 | 5/5 |
| 75 | 4/5 | 4/5 |
| 80 | 3/5 | 3/5 |
| 85 | 5/5 | 5/5 |
| 100 | 4/5 | 4/5 |

Individual times vary in both directions. Seed 40104 took 55.989 seconds with
Monte Carlo and 1.001 seconds without it; seed 100101 took 34.863 and 3.804 seconds,
respectively. Both retirement solves fit inside the initial backtracking phase.
Median preparation across the same 35 cases fell from 1.601 to 1.136 seconds.
Retirement does not claim to speed up every search.

The added level-68 diagnostic found a remaining Monte Carlo benefit: **5/5 with
Monte Carlo versus 4/5 after retirement**. Seed 68102 solved in 83.829 seconds
with Monte Carlo and timed out at 120 seconds without it. Including this
diagnostic gives **36/40 versus 35/40**, with one Monte Carlo-only solve and no
retirement-only solves. The retirement build still clears the per-level target
at level 68. Monte Carlo therefore retains some unique value; its retirement
accepts this individual regression in exchange for a simpler search that meets
the stated per-level budget target on the measured samples.

This measures Monte Carlo's incremental value with the newer searches held
fixed. It does not isolate the individual contributions of adaptive, regional,
and guided search, or establish that their decisions are mathematically
equivalent to Monte Carlo pruning. Monte Carlo uses coarse sampled trajectory
statistics to reject branches; the newer probability models use spatial
constraints to guide assignments and prefix selection.

Serial native controls exercise the backtracking path used by WebAssembly.
Both builds solved **9/10** across levels 40 and 51, with the same timeout at
seed 40104. Every solved search was no slower without Monte Carlo, and all ten
preparation times were lower. These one-core controls are separate from the
all-core target and do not measure browser runtime.

## Measurement

The native parallel samples run one puzzle at a time with all 32 available
logical CPUs on an Intel Core i9-13900KF, about 15 GiB RAM, and no CPU quota.
No builds, test suites, or competing solver runs overlap these measurements.
The serial controls are pinned to CPU 0 and also run one puzzle at a time.

Every case has a 120-second search cap. Preparation is timed separately, before
the worker's `READY preparation_ms` message starts the search timer. Branching,
inference, decimation, restarts, failed phases, and guided frontier construction
all count as search. The fixed sample uses seeds `level * 1000 + 100` through
`level * 1000 + 104`; some of these seeds informed development. All fixed-sample
puzzles through level 45 solved in at most 1.001 seconds. The longest successful
fixed-sample search was 85.029 seconds.

Fresh samples use ten games each at levels 50 and 80, with offsets 500–509.
These seeds and sample sizes were selected before their results were measured;
the solver was frozen throughout validation. Both levels scored 10/10, with
longest searches of 57.080 seconds at level 50 and 38.208 seconds at level 80.
Across the 520 native parallel retirement cases, preparation had a median of 0.8125 seconds
and a maximum of 2.306 seconds. There were six fixed-sample timeouts, no holdout
timeouts, no completed unsuccessful searches, and no worker or preparation
errors.

Each puzzle contributes one accepted run from its build. The full level-85
sample was measured before the sweep and reused intact from the same frozen
executable. No per-case best-of selection is used. Interrupted or competing
prototype measurements are excluded. The success rates describe finite
generated samples, not a proof of future per-level success probabilities.

The [merged baseline report](../2026-09-06/README.md) recorded 478/500 using
mostly 40- or 60-second caps. That aggregate is not a matched 120-second control:
its shorter-budget timeouts conservatively count puzzles that might solve with
more time. Individual backtracking puzzles can also take longer under the
common schedule because a timed-out root search restarts after the other phases.

The [per-level counts](levels.csv), [fixed sample](fixed.csv),
[fresh samples](holdouts.csv), and [paired native comparison](paired.csv)
include every accepted result. Separate files retain the [planned controls](mc-control.csv),
[level-68 diagnostic](mc-diagnostic.csv), and serial
[Monte Carlo](serial-control.csv) and [retirement](serial-retired.csv) controls.
The six underlying samples have matching `.log` files with the benchmark's
raw output, with trailing display padding removed. `search_ms` is blank for timeouts; every
`search_budget_ms` is 120,000. `work_units` combines backtracking states and
complete inference assignments and must not be used to compare node throughput
across algorithms.

## Reproduction

Based on merged main commit `da8b4d62a28b02562de18b970c8fdf6c347cae5c`.
The repository pins `nightly-2026-08-30`; the compiler is
`rustc 1.100.0-nightly (fd7ed57df 2026-08-29)`. Native release builds use
`target-cpu=native`, LTO, and one codegen unit. Run benchmark commands
sequentially on an otherwise idle machine.

```bash
cargo build --release --locked --bin bench --bin solve --bin generate
target/release/bench simulated 1 100 --parallel --games-per 5 --seed-offset 100 --timeout 120
target/release/bench simulated 50 50 --parallel --games-per 10 --seed-offset 500 --timeout 120
target/release/bench simulated 80 80 --parallel --games-per 10 --seed-offset 500 --timeout 120
```

The [control patch](mc-control.patch) applies to the merged base. Build it in a
separate worktree and target directory, so Cargo cannot reuse a binary from a
different source variant:

```bash
git worktree add --detach ../shapeshifter-mc da8b4d62a28b02562de18b970c8fdf6c347cae5c
git -C ../shapeshifter-mc apply "$PWD/docs/benchmarks/2026-09-06-uniform/mc-control.patch"
cd ../shapeshifter-mc
cargo build --release --locked --bin bench --bin solve --bin generate
target/release/bench simulated 75 75 --parallel --games-per 5 --seed-offset 100 --timeout 120
```

For a native Monte Carlo pair, run the same whole-level command with each
build's executable. To reproduce the
serial controls, omit `--parallel` and prefix each command with `taskset -c 0`
(or another available CPU). Without affinity, serial benchmark mode runs
multiple games concurrently. Compiler, hardware, and thread scheduling affect
timing.

The [manifest](manifest.json) records source, native binary, and WebAssembly
SHA256 hashes, the control build, and the sample plan. The final native rebuild
matches all three measured binaries byte for byte. The frozen control embedded
a CRLF copy of `web/board.js`, whereas the retirement build embeds the tracked
LF copy; normalize that file to CRLF to reproduce the control's exact binary
hash. This only changes the unused HTML viewer payload in benchmark workers.

The final Rust source passed 164 release tests, strict all-target/all-feature
Clippy, formatting, and native debug and release builds. Tests cover preservation
of the global frontier across worker counts, cancellation during construction
and DFS, incremental likelihood scores for M=2 and M=3, exact domain propagation,
regional convolution, and solution replay. The WebAssembly library compile
check passed, and the package was regenerated with pinned wasm-pack 0.15.0.
A runtime smoke test replayed eight WebAssembly solutions in original piece
order and rejected four impossible puzzles across M=2 through M=5.
