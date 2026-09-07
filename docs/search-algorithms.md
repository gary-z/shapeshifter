# Solver design

## Game model

Boards have 3–14 rows and columns, with cell values from zero to `M - 1` for
`M = 2–5`. A puzzle has 2–36 fixed-orientation pieces, each at most 5×5 cells.
Placing a piece decrements every covered cell modulo `M`; pieces may overlap.
Every piece must be placed exactly once, leaving every cell zero.

## Search schedule

Native parallel search and both browser packages use this schedule, stopping
as soon as a solution is found. Each phase gets all available workers on one game.

| Phase | Budget | Configuration |
| --- | --- | --- |
| Root backtracking | 5 seconds | Exact pruning bounds. |
| Early guided search | 2 seconds | 20,000-state frontier, eight depths; `M = 2–3`. |
| Adaptive domain search | 25 seconds | Propagation, probability ordering, and restarts. |
| Regional inference | 25 seconds | Joint distributions over neighboring cells. |
| Guided fallback | 30 seconds | 200,000-state frontier, eight depths; `M = 2–3`. |
| Root backtracking | Remaining time | Exact pruning bounds. |

Browser phases are clipped to the remaining search budget. Native
`PreparedSearch::solve()` has no overall deadline; benchmark runners enforce
one externally. Native serial and exhaustive searches use the exact backtracker;
`--exhaustive` continues after finding a solution.

Eligibility depends on table size: regional factors allow at most 1,024 joint
states, and guided 3×3 windows at most 32,768 states (`M^9`). Board area and piece
count do not change the schedule.

## Exact backtracking

Pieces are ordered by placement count and shape constraints. Parallel workers
share a branch queue. Feasibility checks reject states using:

- Total deficit and the area of remaining pieces.
- Reachable checkerboard, row, column, diagonal, and cell-set totals.
- Adjacent-cell differences that the remaining piece perimeters must smooth.
- Small-component reachability.
- Equivalent effects from consecutive placement pairs, searched only once.
- A zero-cell budget for placements and direct completion of single-cell suffixes.

Cell-set arithmetic uses XOR for base two and packed two-bit subtraction for
base four; other moduli use digitwise subtraction.

## Adaptive placement domains

Each piece starts with all legal placements. Fixing a piece shrinks other
pieces' domains, and new singleton domains become fixed placements. Bounds are
recomputed until no domain changes.

For residual cell deficits `d[c]`, remaining piece area `A`, and modulus `M`, a
completion must satisfy:

```text
hits[c] = d[c] + M * wraps[c],  wraps[c] >= 0
Q = sum(wraps[c]) = (A - sum(d[c])) / M
```

A negative or nonintegral `Q` is impossible. A placement covering `z` currently
zero cells consumes exactly `z` wraps, so placements with `z > Q` are removed.

The intersection and union of a piece's placement masks give the cells it must
and can cover. Summing these indicators across pieces gives each cell's coverage
interval. If the interval cannot attain residue `d[c]`, the state is impossible.
Removing one piece's contribution tests whether that piece must cover or avoid
the cell; placements violating that constraint are removed. These necessary
conditions preserve every valid completion.

A separate probability model ranks branches. Modular convolution of per-piece
coverage probabilities estimates the other pieces' effects on each cell. Prefix
and suffix convolutions omit each piece in turn to score its placements. Workers
vary piece selection, ties, and restarts with growing node budgets. Probabilities
change ordering only; they never prune domains.

## Regional inference

Disjoint 2×2, 2×3, and 3×2 tiles give 16 shifted partitions for `M = 2–3`.
For `M = 4–5`, 2×2, 1×4, and 4×1 tiles give 12 partitions. A region with `k`
cells has `M^k` possible effects, retaining correlations between neighboring cells.

Messages between regions and pieces estimate the probability of the required
joint effect. Convolution on `(Z/MZ)^k` uses a tensor product of `k` transforms
of length `M`. Prefix and suffix products in Fourier space omit each piece
without division by potentially zero coefficients.

Normalized, damped log messages guide placement choices. The most confident
piece is fixed periodically; inconsistent area/deficit bounds abandon an attempt.
Workers share all partition/damping combinations, then try randomized restarts.
Proposed solutions are replayed against every cell in original piece order.

## Guided search

Exact suffix distributions on overlapping 3×3, 3×2, 2×3, and 2×2 windows rank
partial boards. Subtraction tables extend effects one cell at a time, and
convolution iterates only over nonzero piece effects.

Workers expand disjoint groups of frontier states. Each keeps up to the beam
width in candidates, then all lists merge before global truncation. Stable
parent/placement ranks break ties independently of thread scheduling. Only
retained candidates become full board states.

Frontier construction and subsequent backtracking share the phase deadline.
An early miss proceeds to adaptive search; a guided fallback miss restarts
backtracking from the root. Bounded phases can miss valid solutions.

## Preparation and timing

`solver::prepare(game, parallel, exhaustive)` builds legal placements, effect
tables, and pruning bounds. `PreparedSearch::solve()` performs branching,
propagation, inference, restarts, and guided frontier construction. Failed
attempts count as search time. See [benchmarking](benchmarking.md) for the
difference between a search timeout and the native total wall-clock target.
