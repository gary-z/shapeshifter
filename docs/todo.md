# Solver Optimizations — TODO

## Per-cell constraint checks

### 1. Unreachable cell pruning [done]
If any cell has value d > 0 but no remaining piece placement can cover it, prune.
Subsumed by #2.

### 2. Insufficient coverage pruning [done]
Cell at value d needs `(M-d) % M` increments. If at most K remaining pieces can cover
this cell and `K < (M-d) % M`, prune. Implemented via per-piece coverage bitmaps and
a remaining-coverage count board.

### 3. Forced placement propagation
If a cell needs exactly K more increments and exactly K remaining pieces can cover it,
all K must cover that cell — constraining their placement positions. Can cascade.

### 4. Global modular check [done]
`sum(remaining piece cell counts) ≡ min_flips (mod M)`.
Cheap global check that catches impossible states where supply has wrong residue.
