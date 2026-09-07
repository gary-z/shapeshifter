# Native optimization on difficult levels

The frozen candidate solved **32/32** independent validation puzzles within the
120-second total deadline, matching **32/32** on main. Mean total time fell from
**27.804 to 12.009 seconds (56.8% lower)**. The ten puzzles from the five levels
with historical failures also solved **10/10** on both builds; their mean fell
from **30.019 to 24.168 seconds (19.5% lower)**. This qualifies through faster
solves; the sample does not demonstrate a reduction in failures.

| Validation metric | Main | Candidate |
| --- | ---: | ---: |
| Solved within 120 seconds | 32/32 | 32/32 |
| Mean total time | 27.804 s | 12.009 s |
| Median total time | 32.187 s | 5.852 s |
| Mean preparation | 1.158 s | 0.526 s |
| Longest total time | 61.712 s | 62.483 s |
| Historical-timeout subgroup mean (10 puzzles) | 30.019 s | 24.168 s |

The paired geometric mean time reduction is 62.6%. The largest individual
regression was 1.036 seconds (level 48, seed 484373384649827468). A level-75
validation puzzle improved from 59.785 to 6.276 seconds; the other remained about
one minute (60.928 to 61.163 seconds). Performance varies by puzzle, and two
samples per level do not establish future per-level success probabilities.

The [paired timings](validation.csv), [complete validation records](validation.jsonl),
[summary](summary.json), and [build manifest](manifest.json) preserve the measured
results. The [initial wide-beam development run](development-wide.jsonl) and
[final narrow-beam diagnostics](diagnostic.jsonl) are recorded separately.

This study measures native parallel solving on the 32 logical CPUs available on
an Intel Core i9-13900KF, with about 15 GiB RAM and no CPU quota. Each puzzle runs
alone, with all 32 workers. Builds, browser tests, and other solver runs do not
overlap timed measurements.

The deadline is **120 seconds of total process wall time**, including startup,
preparation, search, independent solution replay, and process exit. This differs
from the older `bench` command, which starts its timeout after preparation.
A timeout contributes 120 seconds to the capped arithmetic mean. A completed
unsuccessful search would also contribute 120 seconds. Each returned placement
sequence is replayed in original piece order by the Rust driver and independently
by the Python harness.

## Change

After the initial five seconds of root backtracking, eligible puzzles receive a
two-second guided attempt retaining 20,000 prefixes at each of eight depths.
A narrow beam lets larger trees reach backtracking during this short attempt.
The existing 25-second adaptive phase, 25-second regional phase, full 200,000-state
30-second guided fallback, and root fallback remain available. Guided eligibility
continues to require `M^9 <= 32,768`, so this phase is available for M=2 and M=3.
An unsuccessful early attempt can add about two seconds to a later solve.

Preparation also avoids repeated work: each subtraction-table state is decoded
once, nonzero effect probabilities are collected outside the state loop, and
base-two/base-four cell-set decrements use bit operations. These changes preserve
the table values and floating-point accumulation order. Tests compare modular
arithmetic and complete small-window distributions against independent enumeration.

## Sample plan

The [plan](hard-plan.json) and puzzle files were fixed before their results were
measured. Levels were selected from the prior
[500-puzzle study](../2026-09-06-uniform/fixed.csv): at least two of five searches
took 30 seconds or longer, or at least one failed within 120 seconds. This selects
levels 48, 49, 50, 53, 54, 60, 64, 68, 73, 74, 75, 80, 83, 84, 85, and 100.
The historical-timeout subgroup is levels 68, 73, 75, 80, and 100.

The development sample contains one newly generated puzzle per selected level.
It tested an initial early attempt with the full 200,000-state beam. The final
20,000-state beam was checked on three historical puzzles and three observed
slow development cases. These checks informed development and are not
independent validation. The final validation sample contains two new puzzles
per selected level (32 total) and was untouched until the final candidate was
frozen. These are samples from the selected hard levels, not a uniform estimate
across levels 1–100.

Validation runs each puzzle on both frozen binaries, alternating baseline-first
and candidate-first order. Every accepted result is retained; there is no
best-of selection or replacement of slow results. Parallel scheduling can change
search order, so individual times can vary even when the search policy is unchanged.

## Reproduction

The baseline is main commit `715ff4f23dffb3370fffe7e3c0dd852bb06da475`.
The repository pins `nightly-2026-08-30`; native builds use `target-cpu=native`,
release LTO, one codegen unit, and the checkout's exact dependency versions.
Python 3.11 or newer is needed for the build helper. Run commands sequentially.

From the candidate checkout, create a baseline worktree and fetch the locked
Rust dependencies in each checkout. The measurement helper creates a temporary
crate containing [measure.rs](measure.rs), builds against the requested checkout,
and verifies every resolved dependency has the version recorded in that
checkout's lockfile. Unused optional browser dependencies may be absent.

```bash
git worktree add --detach ../shapeshifter-baseline 715ff4f23dffb3370fffe7e3c0dd852bb06da475
cargo fetch --locked
cargo fetch --locked --manifest-path ../shapeshifter-baseline/Cargo.toml
python3 docs/benchmarks/2026-09-07-hard/build_measure.py ../shapeshifter-baseline /tmp/measure-baseline
python3 docs/benchmarks/2026-09-07-hard/build_measure.py . /tmp/measure-candidate
python3 docs/benchmarks/2026-09-07-hard/paired_sample.py \
  --baseline /tmp/measure-baseline --candidate /tmp/measure-candidate \
  --puzzles docs/benchmarks/2026-09-07-hard/hard-holdout-puzzles.jsonl \
  --output /tmp/shapeshifter-hard-validation
python3 docs/benchmarks/2026-09-07-hard/summarize.py /tmp/shapeshifter-hard-validation
```

The harness writes each result immediately and supports resuming the same input
and binaries. Each record includes the puzzle and binary hashes, timings, outcome,
phase log, and returned placements. The original generated puzzle inputs are
committed, avoiding dependency on a later generator version.

## Verification

The final Rust source passed all 168 release tests with all features, strict
all-target/all-feature Clippy, formatting, and native debug and release builds.
Both WebAssembly packages were regenerated. Chromium 151 and Firefox 155 passed
solution replay, worker reuse, validation, deadlines, cancellation, responsiveness,
UI checks, and fallback after failed parallel startup.

The clean measurement build resolves the same dependency versions and solver
sources as the frozen candidate. Its binary hash differs: cached dependencies
embed standard-library paths under `/rustc/...`, while a clean build with
`rust-src` installed embeds local toolchain paths. The manifest records both
binaries. Hashes identify the measured executable; a portable reproduction need
not be byte-identical because toolchain source paths also affect the binary.

The clean executable also replayed two selected validation puzzles in 6.283 and
6.545 seconds, compared with 6.276 and 6.592 seconds in the frozen run. These
[clean-build checks](clean-build-check.jsonl) are additional diagnostics; they do
not replace or enter the 32-puzzle validation results.
