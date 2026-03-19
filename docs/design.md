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
The full board state is represented as `M` bitboards (one per digit value). For digit `d`, the bitboard has bit `(r,c)` set iff `board[r][c] == d`. These are mutually exclusive — exactly one of the M bitboards has each cell set.

### Piece Representation
Each piece is a bitboard with bits set at its filled cells, anchored at (0,0). To place a piece at offset `(r, c)`, left-shift by `r * 15 + c`.

### Operations
- **Bitwise ops**: AND, OR, XOR, NOT, shift left/right
- **Population count**: count set bits
- **Bounds checking**: verify a shifted piece doesn't exceed board dimensions (no bits set outside valid region)

## Solver Strategy
Backtracking search over piece placements with two key optimizations:

### Min-flips pruning
The board tracks a cached `min_flips` value: the minimum total cell-increments needed to solve, computed as `sum_{d=1}^{M-1} (M - d) * popcount(planes[d])`. This is maintained incrementally during `apply_piece`/`undo_piece` in O(1):
- **apply**: `delta = M * popcount(plane[0] & mask) - popcount(mask)` (cells at 0 wrap to cost M-1; all others decrease by 1)
- **undo**: `delta = popcount(mask) - M * popcount(plane[1] & mask)`

At each backtracking step, if the total popcount of remaining pieces is less than `min_flips`, the branch is pruned.

### Piece ordering
Pieces are sorted by number of valid placements (fewest first). Larger/more-constrained pieces are placed early, reducing the branching factor at the top of the search tree. Smaller pieces placed later benefit more from min-flips pruning since less budget remains.

### Performance
| Levels | Board | M | Pieces | Worst-case time |
|--------|-------|---|--------|-----------------|
| 1–25   | ≤4×4  | 2 | 2–16   | < 1ms           |
| 26–30  | 4×4   | 3 | 11–15  | < 1ms           |
| 31–40  | 6×6   | 2–3 | 12–18 | < 204ms       |
| 41–48  | 8×7   | 2–3 | 15–18 | < 654ms       |
