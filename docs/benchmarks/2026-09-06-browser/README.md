# Shared-memory browser search

The browser solved **45/50** fixed-sample puzzles using 32 search workers and
an overall 120-second search cap, excluding preparation. The corresponding
merged-main native sample solved **45/50**. Every browser solution was replayed
independently in original piece order. There were no worker errors.

| Level | Merged native | Browser | Longest successful browser search (s) |
| --- | ---: | ---: | ---: |
| 10 | 5/5 | 5/5 | 0.008 |
| 40 | 5/5 | 5/5 | 2.018 |
| 50 | 5/5 | 5/5 | 64.514 |
| 51 | 5/5 | 5/5 | 34.499 |
| 65 | 5/5 | 5/5 | 0.020 |
| 68 | 4/5 | 4/5 | 3.215 |
| 75 | 4/5 | 4/5 | 69.562 |
| 80 | 3/5 | 3/5 | 50.537 |
| 85 | 5/5 | 5/5 | 62.447 |
| 100 | 4/5 | 4/5 | 64.828 |

These are five seeds per selected level, `level * 1000 + 100` through `+104`,
covering all supported moduli and the backtracking, adaptive, regional, and
guided paths. They are the same fixed seeds used in the earlier native study,
not a fresh holdout or a new all-100-level qualification. Browser-only solves:
none. Native-only solves: none.
The observations are finite samples, not guarantees about future success rates.

## Matched native timing check

A fresh native run on this branch and an initial browser pilot each solved all
ten selected seed-100 cases. Both used all 32 logical CPUs, one game at a time.
The browser pilot took **231.664 seconds** of
search in total, versus **112.483 seconds**
native (2.06x).

| Level | Fresh native search (s) | Browser pilot search (s) |
| --- | ---: | ---: |
| 10 | 0.201 | 0.008 |
| 40 | 0.202 | 1.985 |
| 50 | 15.881 | 56.428 |
| 51 | 0.411 | 34.947 |
| 65 | 0.201 | 0.004 |
| 68 | 1.823 | 2.648 |
| 75 | 30.263 | 32.670 |
| 80 | 0.202 | 0.585 |
| 85 | 30.816 | 60.398 |
| 100 | 32.483 | 41.991 |

Identical phase budgets do not imply identical per-puzzle times. Slower browser
execution can miss an early-phase solution and enter a later phase, creating a
larger wall-time difference than an instruction-throughput comparison suggests.
Worker scheduling, floating-point code generation, and architecture-dependent
random streams can also change search order. Native's progress-monitor join
adds roughly 200 ms to very short searches; the browser has no such monitor.
The counts of pruned candidates are not a cross-algorithm throughput metric.

## Worker scaling

A separate four-puzzle check used the same shared-memory WASM build with one
worker and the same 120-second cap. One worker solved **3/4**; the 32-worker
sample solved **4/4** for these seeds.

| Seed | One worker search (s) | 32 workers search (s) |
| --- | ---: | ---: |
| 50100 | 17.111 | 56.415 |
| 51100 | 9.442 | 34.499 |
| 80100 | 30.332 | 0.102 |
| 85100 | timeout | 51.571 |

Scaling is uneven: one worker won on the selected level-50 and level-51 cases;
32 workers won on level 80 and solved level 85 within the cap. These are separate
configuration trials with different search paths and warm-up histories. They
do not establish a linear throughput ratio or an optimal pool size.

The pinned [Rust WASM allocator](https://raw.githubusercontent.com/rust-lang/rust/fd7ed57df/library/std/src/sys/alloc/wasm.rs)
serializes heap operations with a global spin lock. Adaptive and regional search
allocate temporary vectors frequently. Allocation contention is a plausible
contributor to the slower multiworker cases; this measurement does not isolate
its cost from search-order differences. High aggregate CPU usage can include
spinning, so it is not a measure of useful search throughput. Hosting headers
cannot eliminate this runtime limitation.

[One-worker results](chromium-one-worker.jsonl) and their
[input puzzles](scaling-puzzles.jsonl) are included. Reproduce them with:

```bash
/tmp/shapeshifter-browser/bin/python web/bench_browser.py \
  docs/benchmarks/2026-09-06-browser/scaling-puzzles.jsonl \
  --threads 1 --output /tmp/browser-one-worker.jsonl
```

## What changed

The previous browser entry point only ran serial backtracking. Browser search
now compiles the native adaptive, regional, and guided methods and uses their
same thresholds and 5/25/25/30-second phase schedule. A coordinator Web Worker
prepares the puzzle and owns a reusable Rayon pool backed by shared WASM memory.
Prepared tables and the work queue are shared rather than duplicated per worker.
The UI remains responsive, enforces one active puzzle, and can cancel via an
atomic flag. Without isolation headers, one worker runs the same search policy.

Cloudflare Pages can supply the required COOP/COEP headers using the committed
`_headers` file. The packaging command is `bash web/package-site.sh`, output
`dist`. Search remains on the user's device. See [hosting and browser setup](../../browser.md).

## Measurement and limits

- Intel Core i9-13900KF, 32 logical CPUs, about 15 GiB RAM, no CPU quota.
- Headless Chromium 151.0.7922.34 on Linux, controlled through Playwright.
  The browser reported 32 CPUs. During the pilot, the renderer used about 30
  cores' worth of CPU; the pool runs real concurrent search.
- One reusable pool and one puzzle at a time. No Rust/WASM builds, browser test suites, or other
  solver searches overlapped measurements. Browser defaults from Playwright
  disable background throttling; this is a foreground-use comparison.
- Browser preparation median: 1.291s;
  maximum: 2.013s. Initial module and
  pool startup are also outside the search budget.
- Branching, inference, restarts, and guided-frontier construction count as
  search. Rust checks cooperative deadlines; raw records retain any slight
  deadline overrun. A solve is counted only if it finishes within 120 seconds.
- The expanded sample uses the final WASM build. The earlier ten-case pilot
  preceded a build-only change that remaps Rust standard-library source paths
  for reproducible output. It is kept separate and never substituted for a
  slower expanded-sample result. No search choices changed between these builds.
- Native-main controls are the selected rows of the [merged native study](../2026-09-06-uniform/README.md).
The fresh native timing check uses this branch, with the same generator inputs.
- Chromium and Firefox pass functional browser tests, including fallback,
  cancellation during startup and search, solution replay, deadlines, worker
  reuse, and the actual paste/solve/cancel UI. Firefox performance and Safari
  compatibility are not established by this Chromium sample.
- Browsers may report fewer CPUs, constrain memory, or throttle inactive tabs.
  The threaded build allows at most 2 GiB of WASM linear memory. Hosting headers
  enable parallelism; they cannot guarantee native speed on every device.

## Reproduction

Validation passed: 165 native release tests with all features, strict Clippy,
formatting, Chromium and Firefox browser tests, and an end-to-end solve from
the packaged `dist` site. All five generated JS/WASM files reproduced byte for
byte in an independent source directory. The build normalizes standard-library
source paths and copied JavaScript line endings for this check. The threaded
binary remains identical to the one used for all expanded and one-worker trials.

Based on merged main `1a988b6`. Rust is pinned to `nightly-2026-08-30`, wasm-pack
to `0.15.0`. Both WASM builds use release LTO; the threaded build enables atomics,
bulk memory, and 128-bit SIMD, and rebuilds the standard library with atomics.
Native uses `target-cpu=native`. The exact binaries are identified in
[sha256.json](sha256.json).

```bash
./web/build.sh
python3 -m venv /tmp/shapeshifter-browser
/tmp/shapeshifter-browser/bin/pip install -r web/requirements.txt
/tmp/shapeshifter-browser/bin/playwright install --with-deps chromium firefox
/tmp/shapeshifter-browser/bin/python web/test_browser.py
/tmp/shapeshifter-browser/bin/python web/test_browser.py --browser firefox
/tmp/shapeshifter-browser/bin/python web/bench_browser.py \
  docs/benchmarks/2026-09-06-browser/puzzles.jsonl \
  --output /tmp/browser-results.jsonl
```

To reproduce the fresh native timing check, run these sequentially:

```bash
cargo build --release --locked --bin bench --bin solve
for level in 10 40 50 51 65 68 75 80 85 100; do
  target/release/bench simulated "$level" "$level" --parallel \
    --games-per 1 --seed-offset 100 --timeout 120
done
```

[Browser records](chromium.jsonl), [input puzzles](puzzles.jsonl),
[native-main controls](native-main.csv), [fresh native results](native-current.jsonl),
and [separate browser pilot](pilot.jsonl) retain every accepted case.
