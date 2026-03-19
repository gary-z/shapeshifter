# Solver Pseudocode

```
solve(game):
    pieces = game.pieces
    placements[i] = all valid (row, col, mask) for piece i

    # Sort: fewest placements first, group duplicates together
    sorted_pieces = sort pieces by (len(placements), shape)
    is_dup[i] = (sorted_pieces[i].shape == sorted_pieces[i-1].shape)

    # Precompute suffix arrays (indexed by piece position in sorted order)
    remaining_bits[i]      = sum of cell_count for pieces[i..n]
    remaining_perimeter[i] = sum of perimeter for pieces[i..n]
    reach[i]               = OR of all placement masks for piece i
    suffix_coverage[i]     = binary bitboard counter of reach[i..n]

    return backtrack(board, piece_idx=0, min_placement=0)


backtrack(board, piece_idx, min_placement):
    if piece_idx == n:
        return board.min_flips == 0

    # --- Pruning checks (all O(1) via cached/precomputed values) ---

    # 1. Active planes: each piece removes at most 1
    if board.active_planes > n - piece_idx:  PRUNE

    # 2. Min-flips budget: remaining piece area must cover needed flips
    if remaining_bits[piece_idx] < board.min_flips:  PRUNE

    # 3. Modular: supply and demand must agree mod M
    if remaining_bits[piece_idx] % M != board.min_flips % M:  PRUNE

    # 4. Per-cell coverage: every non-zero cell must be reachable enough times
    for d in 1..M:
        needed = M - d
        if any cell in plane[d] has suffix_coverage[piece_idx] < needed:  PRUNE

    # 5. Jaggedness: boundary complexity must be achievable
    if board.jaggedness > remaining_perimeter[piece_idx]:  PRUNE

    # --- Try placements ---
    for (pl_idx, (row, col, mask)) in placements[piece_idx]:

        # 6. Duplicate symmetry: skip placements before predecessor's
        if pl_idx < min_placement:  SKIP

        board.apply(mask)      # increments covered cells by 1 mod M

        # If next piece is a duplicate, constrain its search
        next_min = pl_idx if is_dup[piece_idx + 1] else 0

        if backtrack(board, piece_idx + 1, next_min):
            return true

        board.undo(mask)       # decrements covered cells by 1 mod M

    return false
```

## Key data structures

```
Board:
    planes[0..M]    : Bitboard per digit value (mutually exclusive)
    min_flips       : u32, incrementally maintained
    active_planes   : u8, recomputed after each apply/undo

Bitboard:
    4 × u64 = 256 bits, stride 15 (bit r*15+c = cell (r,c))

CoverageCounter:
    6 × Bitboard layers = binary count per cell (supports up to 63)
    coverage_ge(k) returns a Bitboard mask in O(1) bitwise ops
```
