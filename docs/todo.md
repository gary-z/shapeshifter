# Solver Optimizations — TODO

## Per-cell constraint checks

### 1. Unreachable cell pruning [done]
If any cell has value d > 0 but no remaining piece placement can cover it, prune.
Subsumed by #2.

### 2. Insufficient coverage pruning [done]
Cell at value d has deficit `(M-d) % M` (hits still needed). If at most K remaining
pieces can cover this cell and `K < deficit`, prune. Implemented via per-piece coverage bitmaps and
a remaining-coverage count board.

### 3. Forced placement propagation
If a cell needs exactly K more hits (deficit = K) and exactly K remaining pieces can cover it,
all K must cover that cell — constraining their placement positions. Can cascade.

### 4. Global modular check [done]
`sum(remaining piece cell counts) ≡ total_deficit (mod M)`.
Cheap global check that catches impossible states where supply has wrong residue.
