# Native parallel search

Every native parallel, non-exhaustive solve uses the same schedule: five seconds of the
existing backtracker, 25 seconds of adaptive domain search, then 25 seconds of
regional inference before fallback. Each phase gets all available workers on
the same game, and every phase counts toward search time. Serial, WebAssembly,
and exhaustive searches keep the existing backtracker.

Algorithm eligibility follows a state-space budget. Regional factors use at
most 1,024 joint states. The guided frontier is available in fallback
when each 3x3 window needs at most 32,768 states (`M^9`); its exponential tables
are omitted otherwise. Neither decision uses board area or piece count.

Adaptive and regional searches return legal placements in the puzzle's original piece order. They
replay every proposed solution against every cell before reporting success.

## Adaptive placement domains

Each piece starts with all of its legal placements. Fixing one piece can shrink
the other pieces' domains, so the next piece is chosen from the current domains
instead of a fixed initial order.

For residual cell deficits `d[c]`, remaining piece area `A`, and modulus `M`, any
completion must have remaining coverage counts

```
hits[c] = d[c] + M * wraps[c],  wraps[c] >= 0
Q = sum(wraps[c]) = (A - sum(d[c])) / M
```

A negative or nonintegral `Q` is impossible. Placing a piece on `z` cells whose
current deficit is zero reduces `Q` by exactly `z`. Thus a placement with more
than `Q` such cells can be removed from its domain.

For each unfixed piece, intersecting its placement masks gives the cells it
must cover; their union gives the cells it can cover. Summing those indicators
across pieces gives an inclusive interval `[lower[c], upper[c]]` for each cell's
remaining coverage. If no count in that interval has residue `d[c]`, the state
is impossible.

Removing one piece's contribution from the interval lets the solver test its
two alternatives at that cell: cover it or leave it uncovered. If an alternative
cannot attain the required residue, placements taking that alternative are
removed. New singleton domains become fixed placements. All bounds are recomputed
until no domain changes. These are necessary conditions, so filtering cannot
remove a placement used by a valid completion.

Search ordering uses a separate probability model. A piece's chance of covering
a cell is its fraction of remaining placements that cover it. Modular convolution
of these Bernoulli distributions estimates the other pieces' coverage. Prefix
and suffix convolutions produce the distribution with each piece omitted. The
log probability of the required residual count ranks its placements.

Workers choose pieces by confidence, smallest domain, or domain size relative
to piece area. Independent random streams diversify ties and restarts; each
stream has a growing node budget. Probabilities affect ordering only, and never
prune a domain. Brute-force tests check that domain propagation preserves every
solution on small boards, including restricted domains and modular wraps.

## Regional inference

The larger boards benefit from keeping neighboring cells together. Disjoint
2x2, 2x3, and 3x2 tiles define 16 shifted partitions when six-cell factors fit
the 1,024-state budget. Otherwise 2x2, 1x4, and 4x1 tiles give 12 partitions.
This selects six-cell regions for M=2 and M=3, and four-cell regions for M=4
and M=5. A region with
`k` cells has `M^k` possible modular effects. Each piece's complete placement
contributes one such effect to the region.

Messages between regions and pieces estimate the probability that all remaining
pieces supply the required joint effect. This is convolution on `(Z/MZ)^k`: a
tensor product of `k` transforms of length `M`, rather than one cyclic transform
of length `M^k`. Prefix and suffix products in Fourier space remove each piece
without dividing by a potentially zero coefficient.

Normalized, damped log messages guide placement choices. The most confident piece
is fixed periodically, and an inconsistent area/deficit bound abandons the
attempt. Workers first cover all partition/damping combinations (80 with
six-cell regions, 60 with four-cell regions) through a shared counter, then
explore randomized restarts. Fourier
convolution is tested against independent direct modular convolution, and every partition is checked
to cover each board cell exactly once.

## Timing

`solver::prepare` builds legal placements, effect tables, and pruning bounds.
`PreparedSearch::solve` does all domain propagation, message passing, branching,
decimation, restarts, and guided frontier construction. Failed attempts count
as search time. The benchmark starts its timeout after the worker reports
`READY preparation_ms`, and runs one game at a time with `--parallel`.

The additional phases are bounded searches, so a miss falls back to backtracking
with exact pruning bounds. The benchmark's overall search timeout also
includes this fallback. Search success rates are measurements over the stated
puzzle sample, rather than a completeness guarantee.

The initial backtracking phase protects quick solutions. Puzzles that previously
needed more than five seconds can take longer when the additional searches
miss, because fallback backtracking starts again afterward. The common schedule
accepts this latency tradeoff while targeting at least half of each level's
puzzles within two minutes of search.

## Guided fallback

The guided frontier ranks prefixes using exact suffix distributions on overlapping
windows. It retains the best 200,000 states at each of the first eight depths.
Workers expand disjoint groups of parent states, keeping up to 200,000 candidate
placements each. Any globally retained candidate must appear in its worker's
local list. The lists are merged before truncation, so a worker's assigned
parents do not receive a fixed share of the frontier. Stable parent/placement
ranks break ties independently of thread scheduling.

Only the retained placements become full board states. The complete guided
attempt, including frontier construction and subsequent backtracking, has a
30-second deadline. If it expires, ordinary backtracking restarts from the root.
This gives fallback search time without a board-size exception. Tests compare
the retained prefixes, their order, and visited counts with one and four workers,
and check cancellation during both frontier construction and DFS.

## Exact backtracking bounds

Monte Carlo trajectory sampling and percentile envelopes are removed. Probability
models guide the native searches; backtracking feasibility uses deterministic
bounds on total deficit, jaggedness, small components, partitions, and cell sets.
The exact jaggedness check previously lived inside the Monte Carlo wrapper and
is preserved in the common feasibility function. Search positions no longer
carry hit-count histories, and root search runs without sampled-bound retries.
Serial and WebAssembly builds use this same exact backtracker.

The [uniform search validation](benchmarks/2026-09-06-uniform/README.md)
compares the same policy with and without Monte Carlo. It records the per-level
results and the remaining individual regression accepted when retiring it.

## Background

The message-passing and decimation ideas are discussed in
[Yedidia, Freeman, and Weiss, *Characterization of belief propagation and its generalizations*](https://www.merl.com/publications/TR2001-15)
and [Montanari, Ricci-Tersenghi, and Semerjian, *Solving Constraint Satisfaction Problems through Belief Propagation-guided decimation*](https://arxiv.org/abs/0709.1667).
The modular region factors, filtering bounds, and search schedule described above
are specific to this solver; the papers do not establish success rates for
Shapeshifter puzzles.
