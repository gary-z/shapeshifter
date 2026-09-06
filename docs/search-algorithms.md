# Native parallel search

The native solver selects one additional search before the existing parallel
backtracker. On M=3 boards, 56 cells select adaptive domain search and at least
100 cells select regional inference. M=4 boards with at least 64 cells also select
regional inference. Both start with a five-second attempt using
the existing backtracker, preserving its quick solves. The selected additional
search gets 30 seconds and all available workers. Each phase runs on the same
game, and every phase counts toward search time. Other board sizes, serial,
WebAssembly, and exhaustive searches keep the existing backtracker.

Both methods return legal placements in the puzzle's original piece order. They
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
2x2, 2x3, and 3x2 tiles define 16 shifted partitions of an M=3 board. For M=4,
2x2, 1x4, and 4x1 tiles give 12 partitions with at most 256 states per region;
six-cell regions would need 4096 states. A region with
`k` cells has `M^k` possible modular effects. Each piece's complete placement
contributes one such effect to the region.

Messages between regions and pieces estimate the probability that all remaining
pieces supply the required joint effect. This is convolution on `(Z/MZ)^k`: a
tensor product of `k` transforms of length `M`, rather than one cyclic transform
of length `M^k`. Prefix and suffix products in Fourier space remove each piece
without dividing by a potentially zero coefficient.

Normalized, damped log messages guide placement choices. The most confident piece
is fixed periodically, and an inconsistent area/deficit bound abandons the
attempt. Workers first cover all partition/damping combinations (80 for M=3,
60 for M=4) through a shared counter, then explore randomized restarts. Fourier
convolution is tested against independent direct modular convolution, and every partition is checked
to cover each board cell exactly once.

## Timing

`solver::prepare` builds legal placements, effect tables, and pruning bounds.
`PreparedSearch::solve` does all domain propagation, message passing, branching,
decimation, restarts, and guided frontier construction. Failed attempts count
as search time. The benchmark starts its timeout after the worker reports
`READY preparation_ms`, and runs one game at a time with `--parallel`.

The additional phases are bounded searches, so a miss falls back to the existing
progressive Monte Carlo backtracker. The benchmark's overall search timeout also
includes this fallback. Search success rates are measurements over the stated
puzzle sample, rather than a completeness guarantee.

The initial backtracking phase protects quick solutions. Puzzles that previously
needed more than five seconds can still take longer when the additional search
misses, because fallback backtracking starts again after that phase. The board-size
selection reflects measured search behavior: adaptive search helped the 56-cell
M=3 boards, but its extra phase delayed the 8x8 M=3 sample without finding solutions.
Those boards therefore continue directly with the existing backtracker.

## Background

The message-passing and decimation ideas are discussed in
[Yedidia, Freeman, and Weiss, *Characterization of belief propagation and its generalizations*](https://www.merl.com/publications/TR2001-15)
and [Montanari, Ricci-Tersenghi, and Semerjian, *Solving Constraint Satisfaction Problems through Belief Propagation-guided decimation*](https://arxiv.org/abs/0709.1667).
The modular region factors, filtering bounds, and search schedule described above
are specific to this solver; the papers do not establish success rates for
Shapeshifter puzzles.
