# Search validation — September 6, 2026

The frozen final build solved **478/500 puzzles**, with **at least 3/5
successes at every level from 1 through 100**, within 120 seconds of search.
The distribution was 86 levels at 5/5, 6 levels at 4/5, and 8 levels at 3/5.
Fresh holdouts at levels 50 and 80 scored **5/10** and **10/10**, respectively
(15/20 combined).
These are empirical rates on finite generated samples, not a proof of the
success probability on every future puzzle distribution.

## Measurement

One puzzle ran at a time, using all 32 available logical CPUs on an Intel Core
i9-13900KF (about 15 GiB RAM, no CPU quota). No builds, test suites, or competing
solver runs ran during these measurements. Preparation was timed separately;
all branching, inference, decimation, failed attempts, restarts, and guided
frontier construction counted as search. The longest successful search was
104.622 seconds. Preparation had median 0.740 seconds and
maximum 3.329 seconds across the 520 cases.

The fixed sample uses five games per level, with generator seeds
`level * 1000 + 100` through `level * 1000 + 104`. Some of these seeds informed
development. The extra ten-game samples use offsets 200–209 at levels 50 and 80;
those seeds were first measured after the final algorithms were frozen.

Most levels were screened at 40 seconds, with focused checks at 60 seconds.
Three levels already known to need 120 seconds used that budget directly.
The protocol allowed a whole-level rerun at 120 seconds if fewer than half
solved, replacing that level's short sample; no additional reruns were needed
on the final build. Every puzzle contributes one run, and all final rows come
from the same frozen binary. A short-budget timeout is a conservative miss in
the success count, not a claim of failure at 120 seconds. The fixed-sample caps were:

| Search cap | Levels |
| --- | --- |
| 120 seconds | 48, 75, 85 |
| 60 seconds | 54, 60, 62, 63, 64, 68, 70, 71, 79, 84, 94 |
| 40 seconds | All other levels |

Both fresh holdouts used a 40-second search cap. There were 22 fixed-sample
timeouts and 5 holdout timeouts, zero completed unsuccessful searches, and no
worker or preparation errors. See [per-level counts](levels.csv),
[fixed records](fixed.csv), [holdout records](holdouts.csv), and the benchmark
[fixed output](fixed.log) and [holdout output](holdouts.log), with trailing
display padding removed.
`search_ms` is blank for timeouts; `search_budget_ms` preserves their actual cap.
`work_units` combines backtracking states and complete inference assignments
and must not be used for cross-algorithm node-throughput comparisons.

## Algorithm changes and regressions

[Adaptive domain propagation and regional inference](../../search-algorithms.md)
change which assignments are explored. Exact modular bounds shrink adaptive
domains; joint regional constraints guide inference and decimation. Every new
search result is replayed against every board cell before being returned.

An intermediate schedule delayed quick backtracking solutions by putting a
30-second heuristic phase first. The final build gives backtracking five seconds
first and removes the unhelpful adaptive phase on 8x8 M=3 boards. This corrects
the early-level regression observed during development. Most early levels were
already fast on main; the work also targets hard seeds that main misses.

Matched examples (all times exclude preparation):

| Generator seed | Comparison search | Previous search | Final search |
| --- | --- | ---: | ---: |
| 50100 | main search on 56-cell M3 | >120s | 16.275s |
| 80104 | main search on M4 | >120s | 6.233s |
| 54100 | intermediate adaptive phase on 8x8 M3 | 59.820s | 22.419s |
| 62100 | intermediate regional-first schedule | 30.612s | 2.603s |
| 68102 | main search on M4; retained latency regression | 24.817s | >60s |

The main-search controls above use experimental binaries whose search path for
the stated board classes is unchanged from main; their other board classes
include intermediate algorithms. [Control output and binary hashes](comparisons.log)
identify them. These matched examples are not an aggregate main-versus-final
benchmark. Thread scheduling also makes individual runtimes variable.

The level-68 case shows a remaining tradeoff: when the additional phase misses,
fallback backtracking restarts, delaying some puzzles that previously needed
more than five seconds. The per-level target is met on the measured sample;
this change does not improve every puzzle's latency. The new searches apply to
native parallel mode. Serial, exhaustive, and browser searches retain the
existing backtracker.

## Reproduction and checks

Based on main commit `c01a22da6d3ea4bd5078083f63b35b2256fa27e4` on branch
`improve/spatial-search`. The repository pins `nightly-2026-08-30`;
the measured compiler was `rustc 1.100.0-nightly (fd7ed57df 2026-08-29)`. Release builds use
`target-cpu=native`, LTO, and one codegen unit.
[The manifest](manifest.json) records source and binary SHA256 hashes.

Run the full fixed sample at the maximum search budget, then the fresh samples:

```bash
cargo build --release --locked --bin bench --bin solve
target/release/bench simulated 1 100 --parallel --games-per 5 --seed-offset 100 --timeout 120
target/release/bench simulated 50 50 --parallel --games-per 10 --seed-offset 200 --timeout 120
target/release/bench simulated 80 80 --parallel --games-per 10 --seed-offset 200 --timeout 120
```

To reproduce an individual accepted level sample's cap, set its start and end
level to that level and use the `search_budget_ms` value in `levels.csv` divided
by 1000. Fresh samples can use a different seed offset. Do not run benchmark
commands concurrently. Hardware, compiler, and thread scheduling affect timing.

The final Rust source passed 163 release tests, strict all-target/all-feature
Clippy, formatting, and the native release build. The WebAssembly package was
regenerated with the repository's pinned wasm-pack 0.15.0. Tests cover modular
convolution against direct enumeration, partition coverage, propagation that
preserves every brute-force solution on small boards, deadline cancellation,
and legal solutions in original piece order.
