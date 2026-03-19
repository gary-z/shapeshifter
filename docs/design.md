# Shapeshifter Solver — Design Doc

## Overview
A Rust-based solver for the Neopets Shapeshifter puzzle. The solver finds placements for all N pieces on an H×W board such that every cell becomes 0 (mod M).

## Board Representation

### Bitboard Layout
The board is up to 14×14 = 196 cells. We use a fixed-width bitboard of 4×u64 = 256 bits.

**Bit indexing:** bit `row * 15 + col` represents cell `(row, col)`. The 15-column stride (not W) ensures a fixed layout regardless of board width, simplifying shift operations. Bits beyond valid columns in each row are always 0.

- Bit 0 = (0, 0) — upper-left
- Bit 1 = (0, 1)
- Bit 15 = (1, 0) — second row
- Bit `r*15 + c` = (r, c)

Maximum bit index: 13×15 + 13 = 208, well within 256 bits.

### Board State
The full board state is represented as `M` bitboards (one per digit value, max 5). For digit `d`, the bitboard has bit `(r,c)` set iff `board[r][c] == d`. These are mutually exclusive — each cell appears in exactly one plane.

The board caches two derived values, maintained incrementally during apply/undo:
- **min_flips**: minimum total cell-increments needed to solve = `sum_{d=1}^{M-1} (M-d) * popcount(planes[d])`
- **active_planes**: count of non-zero planes with any bits set

### Piece Representation
Each piece is a bitboard with bits set at its filled cells, anchored at (0,0). To place a piece at offset `(r, c)`, left-shift by `r * 15 + c`. Each piece also precomputes its cell count and perimeter.

## Solver Strategy

Backtracking search over piece placements with the following optimizations:

### 1. Piece ordering
Pieces are sorted by number of valid placements (fewest first), with a secondary sort by shape to group duplicates together. More-constrained pieces are placed early, reducing the branching factor at the top of the search tree.

### 2. Duplicate piece symmetry breaking
When consecutive pieces in the sorted order have the same shape, we enforce that each duplicate's placement index is ≥ the previous duplicate's. This eliminates redundant permutations. With ~22 out of 36 pieces being duplicates at level 100 (in ~6 groups of up to 12), this prunes enormous amounts of the search tree.

### 3. Min-flips pruning
At each node, if the total popcount of remaining pieces is less than `min_flips`, prune. The board's `min_flips` is maintained incrementally in O(1):
- **apply**: `delta = M * popcount(plane[0] & mask) - popcount(mask)`
- **undo**: `delta = popcount(mask) - M * popcount(plane[1] & mask)`

Note: `(remaining_bits - min_flips) % M` is an invariant (changes by `-M * zeros_hit` per placement, which is 0 mod M). So the modular check only needs to be done once at the root to validate input, not per-node.

### 4. Active planes pruning
Each piece placement can reduce the number of active (non-zero) planes by at most 1. If `active_planes > remaining_pieces`, prune.

### 5. Per-cell coverage pruning
For each piece, precompute its "reach" — the union of all cells it can cover across all valid placements. Suffix coverage counts are stored as 6-layer binary bitboard counters (`CoverageCounter`), enabling O(1) parallel threshold checks across all cells.

At each node, for each non-zero plane d, check that every cell in that plane has coverage ≥ `(M-d)` among remaining pieces. A single bitwise operation per threshold: `(plane[d] & !coverage_ge(M-d)).is_zero()`.

This subsumes unreachable-cell detection (coverage < 1).

### 6. Jaggedness pruning
**Jaggedness** = count of adjacent cell pairs with different values. A solved board has jaggedness 0. Each piece placement can change jaggedness by at most ±perimeter(piece), because only perimeter edges (between covered/uncovered cells) can affect adjacency matches.

Therefore: `jaggedness(board) <= sum(perimeter(remaining_pieces))`. If violated, prune.

Computed efficiently with bitboards: matching pairs = `sum_d popcount(plane[d] & (plane[d] >> 1))` for horizontal + `>> 15` for vertical. Piece perimeters use the same trick: `cells*4 - 2 * (popcount(shape & (shape >> 1)) + popcount(shape & (shape >> 15)))`.

### 7. Cell locking
Cells at 0 where `coverage < M` among remaining pieces can never wrap back to 0 if touched. All placements overlapping these cells are filtered out. Computed as `board.plane(0) & !coverage_ge(M)` — a single bitwise operation.

### 8. 1x1 endgame
When all remaining pieces are single-cell (sorted last due to having the most placements), solve directly: each non-zero cell at value d gets `(M-d)` pieces assigned. O(cells), no search. Eliminates ~3–6 trailing levels of backtracking per search path.

## Performance

Tested with 20 random seeds per level, 1s timeout:

| Levels | Board | M | Pieces | Solve Rate | Avg Time |
|--------|-------|---|--------|------------|----------|
| 1–35 | ≤6×6 | 2 | 2–16 | **100%** | < 15ms |
| 36–45 | 6×6–8×7 | 2–3 | 14–19 | **90–100%** | < 105ms |
| 46–60 | 8×7–8×8 | 3–4 | 16–20 | **75–95%** | < 280ms |
| 61–70 | 10×10 | 3–4 | 17–23 | **75–100%** | < 333ms |
| 71–80 | 10×11 | 3–4 | 18–24 | **50–100%** | < 571ms |
| 81–90 | 12×12 | 3–4 | 20–25 | **60–100%** | < 504ms |
| 91–96 | 14×13 | 4–5 | 23–26 | **85–100%** | < 174ms |
| 97 | 14×13 | 5 | 28 | **100%** | 237ms |
| 98 | 14×13 | 5 | 30 | **60%** | 509ms |
| 99 | 14×13 | 5 | 32 | **45%** | 663ms |
| 100 | 14×14 | 5 | 36 | **25%** | 776ms |

Higher M paradoxically helps: more digit states mean more constraints and tighter pruning.
