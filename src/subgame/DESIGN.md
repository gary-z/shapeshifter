# Subgame Pruning

## Overview

The full Shapeshifter game is played on a 2D H x W board. We define two 1D
**subgames** -- the Row subgame (H cells) and the Column subgame (W cells) --
by projecting the board and pieces along each axis. Solving both subgames is
**necessary but not sufficient** to solve the full game, so an infeasible
subgame lets us prune immediately.

## Decrement Formulation

Both the full game and the subgames use a **decrement-to-zero** model rather
than the original increment-and-wrap model. This makes the relationship between
the two levels transparent:

- Each cell stores its **deficit**: how many increments it still needs to reach
  0 mod M. For a full-game cell with value `v`, the deficit is `(M - v) % M`.
- Placing a piece **decrements** each covered cell by the piece's contribution
  at that position.
- The goal is to bring every cell to exactly zero. A cell going negative means
  overshoot -- infeasible on that search branch.

This is mathematically equivalent to the original formulation but easier to
reason about: deficits start high and shrink toward zero, never wrapping.

## Subgame Board Construction

### Row subgame (H cells)

For each row `r`, sum the per-cell deficits across all columns:

```
row_deficit[r] = sum_{c=0}^{W-1} (M - board[r][c]) % M
```

Range: `[0, W * (M - 1)]`. This value is **not** reduced mod M. It represents
the exact total number of piece-cell hits that must land in row `r`.

### Column subgame (W cells)

Symmetric:

```
col_deficit[c] = sum_{r=0}^{H-1} (M - board[r][c]) % M
```

Range: `[0, H * (M - 1)]`.

### Why unreduced sums are stronger

Consider M = 5 and a row `[0, 0, 0, 0, 1, 1]`. The per-cell deficits are
`[0, 0, 0, 0, 4, 4]`, summing to 8.

- **Mod-M version**: `8 mod 5 = 3`. Claims 3 increments suffice.
- **Unreduced version**: stores 8. Correctly requires exactly 8 piece-cells.

The unreduced version is strictly tighter and catches infeasibility that the
mod version misses.

## Subgame Piece Construction

### Row projection

For a 2D piece P with bounding box h x w, the row projection is a sequence of
`h` positive integers:

```
row_profile[j] = |{c : (j, c) is filled in P}|    for j in 0..h
```

Each entry is in `[1, 5]` (pieces are 1-5 rows tall, and `from_grid` enforces
no empty border rows). The profile has 1-5 entries.

### Column projection

Symmetric:

```
col_profile[j] = |{r : (r, j) is filled in P}|    for j in 0..w
```

### Subgame placement

Placing a piece with profile `[t_0, t_1, ..., t_{k-1}]` at position `p` in a
subgame of length N:

- Requires `p + k <= N` (piece fits).
- Decrements cell `p + j` by `t_j` for each `j in 0..k`.
- If any cell goes below zero, the placement is infeasible.

## Proof: Necessary Condition

**Claim**: If the full game is solvable, both subgames are solvable.

**Proof**: Suppose the full game has a solution placing piece `i` at `(r_i, c_i)`.
For any row `r`, the full-game solution satisfies:

```
coverage[r][c] ≡ (M - board[r][c])  (mod M)    for all c
```

where `coverage[r][c]` is the number of pieces covering cell `(r, c)`.
Specifically, `coverage[r][c] = (M - board[r][c]) % M + k_{r,c} * M` for some
`k_{r,c} >= 0` (wrapping count). Summing over all columns in row `r`:

```
sum_c coverage[r][c] = row_deficit[r] + M * K_r
```

where `K_r = sum_c k_{r,c} >= 0`. The left side also equals the total
piece-cells landing in row `r`, which is `sum_i row_profile_i[r - r_i]` (for
pieces whose row span includes `r`). This is exactly the total decrement
applied to row `r` when we place each piece `i` at row position `r_i` in the
row subgame.

After all pieces, row `r` in the subgame has value
`row_deficit[r] - (row_deficit[r] + M * K_r) = -M * K_r ≡ 0 (mod M)`.

When no wrapping occurs (`K_r = 0` for all `r`, i.e.
`total_piece_cells == min_flips`), all cells reach exactly 0.

Therefore the same row positions that solve the full game also solve the row
subgame. The argument is symmetric for the column subgame. **QED**

## Proof: Not Sufficient

**Claim**: Both subgames being solvable does NOT imply the full game is solvable.

**Counterexample**: 3 x 3 board, M = 3:

```
Board:           Deficits ((M - v) % M):
  0 1 2            0 2 1
  2 0 1            1 0 2
  1 2 0            2 1 0
```

Three pieces: all 1 x 3 horizontal bars (shape `[###]`).

**Row subgame**: deficit = `[0+2+1, 1+0+2, 2+1+0]` = `[3, 3, 3]`.
Each piece has row profile = `[3]` (1 row, 3 cells). Place one piece per row:
`[3-3, 3-3, 3-3]` = `[0, 0, 0]`. **Solved.**

**Column subgame**: deficit = `[0+1+2, 2+0+1, 1+2+0]` = `[3, 3, 3]`.
Each piece has column profile = `[1, 1, 1]` (3 columns, 1 cell each).
On a 3-wide board, the only valid column position is 0. Each piece decrements
all three columns by 1. Three pieces: `[3-3, 3-3, 3-3]` = `[0, 0, 0]`. **Solved.**

**Full game**: The only valid column position for a 1 x 3 bar on a 3-wide board
is column 0. The row subgame forces one bar per row. So each bar covers an
entire row, and every cell is hit exactly once. But cell (0,0) has deficit 0
and receives 1 hit -- overshoot. In the original model: value 0 increments to
1 (mod 3), not 0. Exhaustively:

| Placement          | Hits per row | Result (original model)      |
|--------------------|-------------|------------------------------|
| 1 per row (forced) | all cells +1 | `1 2 0 / 0 1 2 / 2 0 1` ≠ 0 |
| 2 in row 0, 1 in 1 | row 0: +2   | `2 0 1 / 0 1 2 / 1 2 0` ≠ 0 |
| 3 in row 0         | row 0: +3≡0 | `0 1 2 / 2 0 1 / 1 2 0` ≠ 0 |
| (all 10 combos)    | ...         | none solve the board         |

No arrangement of three 1 x 3 bars solves this board. **The full game is
unsolvable, but both subgames are solvable. QED**

### Why the gap exists

The subgames check **marginal sums** (total piece-cells per row / per column).
The full game requires **cell-level** constraints. Satisfying both marginals is
analogous to matching the row and column sums of a matrix -- it constrains but
does not determine the individual entries.

In the counterexample, the row subgame says "3 hits in each row" and the column
subgame says "3 hits in each column," which is consistent. But the only way to
achieve this with 1 x 3 bars is to hit every cell exactly once -- and that
doesn't match the per-cell deficits `[0,2,1; 1,0,2; 2,1,0]`.

## Wrapping Caveat

The "decrement to exactly zero" formulation assumes each cell in the full game
receives exactly its minimum coverage: `(M - board[r][c]) % M` hits. This
holds when `total_piece_cells == min_flips` (total cells across all pieces
equals total deficit across the board). In this case no cell ever wraps past
zero, and the unreduced subgame goal of "all cells reach exactly 0" is sound.

When `total_piece_cells > min_flips`, some cells must wrap (receive more hits
than their minimum deficit). The excess hits satisfy
`total_piece_cells - min_flips ≡ 0 (mod M)`. In this case the strict
"exactly zero" goal could falsely reject valid solutions. The correct
subgame goal generalizes to: all cells reach a value `≡ 0 (mod M)`.

In practice:

- The solver already checks `total_piece_cells >= min_flips` and
  `total_piece_cells ≡ min_flips (mod M)` as necessary conditions.
- After piece cancellation (removing groups of M identical pieces), most
  puzzles have `total_piece_cells == min_flips` exactly.
- When wrapping is needed, we can fall back to the modular check or allow
  subgame cells to end at any non-negative multiple of M rather than only 0.

The unreduced deficit is still valuable even with wrapping: it provides a
**lower bound** on required piece contribution per row/column that the modular
version cannot.

## Computation Cost

The subgame is cheaper than the full game:

| Property        | Full game        | Subgame            |
|-----------------|------------------|--------------------|
| Board cells     | H x W (up to 196)| H or W (up to 14) |
| Placements/piece| up to H x W      | up to H (or W)     |
| Branching factor| O(H * W)         | O(max(H, W))       |

The subgame has the same number of pieces (N, up to 36) but dramatically fewer
placements per piece. Combined with the 1D structure enabling tighter DP bounds,
infeasibility can often be detected orders of magnitude faster.

## Reframing the Full Game as Decrement-to-Zero

To unify reasoning between the full game and subgames, we propose reframing the
full game board to store deficits rather than raw values:

### Current model (increment-and-wrap)

- Cell stores value `v in [0, M)`.
- `apply_piece` increments covered cells by 1 mod M.
- Solved when all cells are 0.
- `min_flips = sum (M - d) * popcount(planes[d])` for d > 0.

### Proposed model (decrement-to-zero)

- Cell stores deficit `d = (M - v) % M`, in `[0, M)`.
- `apply_piece` decrements covered cells by 1, with floor at 0 via mod M
  (deficit `d` becomes `(d - 1 + M) % M`, but conceptually we track the
  remaining work).
- Solved when all deficits are 0 (same check).
- `min_flips` is the sum of all deficits (same value, just reframed).

The two models are isomorphic -- a board with value `v` in the current model is
a board with deficit `(M - v) % M` in the new model, and `apply_piece` in one
corresponds exactly to `apply_piece` in the other. The bitboard plane layout
stays the same; only the interpretation changes:

- `planes[0]` = cells with deficit 0 (already solved) = cells with value 0
- `planes[d]` = cells needing d more hits = cells with value `(M - d) % M`

### Why reframe?

1. **Subgame construction becomes a plain sum**: the subgame cell value for
   row `r` is `sum of deficits in row r`, directly summing the same quantity.
2. **Piece application is uniform**: both the full game and subgame decrement
   cells. No conceptual mismatch between "increment mod M" and "decrement
   toward zero."
3. **Bounds are clearer**: `min_flips` directly reads as "total remaining
   work." Each piece application reduces it by exactly `popcount(piece_mask)`.
   Overshoot (hitting a 0-deficit cell) increases it by `M - 1`, which is
   the natural penalty.

### Migration plan

The reframe is largely an interpretation change. The existing `Board` already
computes `min_flips` as the sum of deficits and checks `min_flips == 0` for
solved. The bitboard layout is identical under either interpretation:

| Current field       | Current meaning                   | Deficit meaning                     |
|---------------------|-----------------------------------|-------------------------------------|
| `planes[0]`         | cells with value 0                | cells with deficit 0 (solved)       |
| `planes[d]` (d > 0) | cells with value d                | cells with deficit `(M - d) % M`    |
| `min_flips`         | sum of `(M - d) * count(planes[d])` | sum of all deficits (same value)  |
| `apply_piece`       | increment covered cells mod M     | rotate deficits (same bit ops)      |
| `is_solved`         | `min_flips == 0`                  | all deficits are 0 (same check)     |

No runtime behavior changes. The concrete steps are:

1. **Subgame module** (`src/subgame/`): Build natively in decrement-to-zero.
   Board cells store unreduced deficit sums. Pieces decrement. Goal is zero.

2. **Full game comments**: Optionally update comments in `board.rs` to use
   deficit language, but this is cosmetic and can be deferred.

3. **Subgame construction**: Add methods to `Board` (or a standalone function)
   that produce subgame boards and pieces from the full game:
   - `to_row_subgame() -> (SubgameBoard, Vec<SubgamePiece>)`
   - `to_col_subgame() -> (SubgameBoard, Vec<SubgamePiece>)`

4. **Integration**: Wire subgame feasibility checks into the solver's pruning
   pipeline, likely as a precomputation step (check once before backtracking)
   and optionally during search (recheck after each piece placement).
