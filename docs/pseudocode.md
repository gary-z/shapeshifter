# Solver Pseudocode

```
solve(game):
    pieces = game.pieces
    placements[i] = all valid (row, col, mask) for piece i

    # Sort: fewest placements first, group duplicates by shape
    sorted_pieces = sort pieces by (len(placements), shape)
    is_dup[i] = (sorted_pieces[i].shape == sorted_pieces[i-1].shape)
    single_cell_start = index where trailing 1x1 pieces begin

    # Precompute suffix arrays (indexed by piece position in sorted order)
    remaining_bits[i]      = sum of cell_count for pieces[i..n]
    remaining_perimeter[i] = sum of perimeter for pieces[i..n]
    reach[i]               = OR of all placement masks for piece i
    suffix_coverage[i]     = binary bitboard counter of reach[i..n]

    return backtrack(board, piece_idx=0, min_placement=0)


backtrack(board, piece_idx, min_placement):
    if piece_idx == n:
        return board.total_deficit == 0

    # --- 1x1 endgame: solve remaining single-cell pieces directly ---
    if piece_idx >= single_cell_start:
        for each non-zero cell at deficit d:
            assign d pieces to that cell
        return total_assigned == remaining_pieces

    # --- Pruning checks ---

    # Active planes: each piece removes at most 1
    if board.active_planes > n - piece_idx:  PRUNE

    # Total-deficit budget: remaining piece area must cover needed deficit
    if remaining_bits[piece_idx] < board.total_deficit:  PRUNE

    # (Modular check is an invariant — only validated once at root)

    # Per-cell coverage: every non-zero cell must be reachable enough times
    for d in 1..M:
        needed = d  # board values are deficits directly
        if any cell in plane[d] has suffix_coverage[piece_idx] < needed:  PRUNE

    # Jaggedness: boundary complexity must be achievable
    if board.jaggedness > remaining_perimeter[piece_idx]:  PRUNE

    # Cell locking: cells at 0 with coverage < M can't be touched
    locked = board.plane(0) & ~suffix_coverage[piece_idx].coverage_ge(M)

    # --- Try placements ---
    for (pl_idx, (row, col, mask)) in placements[piece_idx]:

        # Duplicate symmetry: skip placements before predecessor's
        if pl_idx < min_placement:  SKIP

        # Cell locking: skip placements that touch locked cells
        if (mask & locked) != 0:  SKIP

        board.apply(mask)

        # If next piece is a duplicate, constrain its search
        next_min = pl_idx if is_dup[piece_idx + 1] else 0

        if backtrack(board, piece_idx + 1, next_min):
            return true

        board.undo(mask)

    return false
```

## Key data structures

```
Board:
    planes[0..M]    : Bitboard per digit value (mutually exclusive)
    total_deficit   : u32, incrementally maintained on apply/undo
    active_planes   : u8, recomputed after each apply/undo

Bitboard:
    4 × u64 = 256 bits, stride 15 (bit r*15+c = cell (r,c))

CoverageCounter:
    6 × Bitboard layers = binary count per cell (supports up to 63)
    coverage_ge(k) returns a Bitboard mask in O(1) bitwise ops
    Precomputed as suffix sums over piece reaches
```

## Invariant
`(remaining_bits - total_deficit) % M` is constant throughout the search.
Each placement changes it by `-M * zeros_hit ≡ 0 (mod M)`.
Only needs to be checked once at the root for input validation.
