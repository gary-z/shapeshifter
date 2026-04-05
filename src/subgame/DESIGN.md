# Subgame Pruning

## Overview

The full Shapeshifter game is played on a 2D H x W board. We define two 1D
**subgames** -- the Row subgame (H cells) and the Column subgame (W cells) --
by projecting the board and pieces along each axis. Solving both subgames is
**necessary** to solve the full game, so an infeasible subgame lets us prune
immediately.

## Subgame Board Construction

### Row subgame (H cells)

For each row `r`, sum the per-cell deficits across all columns:

```
row_deficit[r] = sum_{c=0}^{W-1} board[r][c]
```

Range: `[0, W * (M - 1)]`. This value is **not** reduced mod M. It represents
the exact total number of piece-cell hits that must land in row `r` to bring
all cells in that row to zero — assuming no wrapping occurs. Values can exceed
M, which is the key advantage over a modular formulation.

### Column subgame (W cells)

Symmetric:

```
col_deficit[c] = sum_{r=0}^{H-1} board[r][c]
```

### Why unreduced sums are stronger

Consider M = 5 and a row with per-cell deficits `[0, 0, 0, 0, 4, 4]`,
summing to 8.

- **Mod-M version**: `8 mod 5 = 3`. Claims 3 hits suffice.
- **Unreduced version**: stores 8. Correctly requires at least 8 piece-cells.

The unreduced version is strictly tighter.

## Subgame Piece Construction

### Row projection

For a 2D piece P with bounding box h x w, the row projection is a sequence of
`h` positive integers:

```
row_profile[j] = |{c : (j, c) is filled in P}|    for j in 0..h
```

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
- If any cell goes below zero, it **wraps**: a cell at value 0 decremented by
  1 becomes M-1 (not rejected). This mirrors the full game's modular
  arithmetic projected onto the subgame axis.

## Wrapping Model

Cell values are always non-negative (`u16`). The decrement-and-wrap rule:

```
new_value = if old_value >= hit { old_value - hit }
            else { old_value + M * ceil((hit - old_value) / M) - hit }
```

For the common case of a single-unit decrement on a zero cell: `0 → M-1`.

This is equivalent to the full game's behavior projected onto one axis:
when a cell in the full game wraps past zero, it adds M to the deficit.
In the subgame, the row/column sum increases by M per wrapping cell, which
is exactly what `0 → M-1` achieves (the cell needed 0 more hits, but got 1,
so now it needs M-1 more to reach the next multiple of M).

### Goal

The subgame is **solved** when all cells equal 0.

## Proof: Necessary Condition

**Claim**: If the full game is solvable, both subgames are solvable.

**Proof**: Suppose the full game has a solution placing piece `i` at
`(r_i, c_i)`. For any row `r`, the number of piece-cells landing in row `r`
is `sum_i row_profile_i[r - r_i]`. In the full game, each cell `(r, c)`
receives `coverage[r][c]` hits where `coverage[r][c] >= board[r][c]` and
`coverage[r][c] ≡ board[r][c] (mod M)`.

Summing over columns:

```
total_hits_in_row_r = sum_c coverage[r][c] = row_deficit[r] + M * K_r
```

where `K_r = sum_c k_{r,c} >= 0` counts total wraps in row `r`. Placing each
piece `i` at row position `r_i` in the row subgame applies the same total
hits per row. After all pieces, row `r` has value:

```
row_deficit[r] - total_hits_in_row_r = -M * K_r
```

Starting from a non-negative value and reaching `-M * K_r` means the cell
wrapped `K_r` times through 0, each time going `0 → M-1` and then being
decremented further. The final value is 0 (the wrapping brings it to `M-1`,
then `M-1` more hits bring it to 0, repeated `K_r` times). Therefore the
subgame is solved. The argument is symmetric for columns. **QED**

## Computation Cost

The subgame is cheaper than the full game:

| Property        | Full game        | Subgame            |
|-----------------|------------------|--------------------|
| Board cells     | H x W (up to 196)| H or W (up to 14) |
| Placements/piece| up to H x W      | up to H (or W)     |
| Branching factor| O(H * W)         | O(max(H, W))       |

The subgame has the same number of pieces (N, up to 36) but dramatically fewer
placements per piece.
