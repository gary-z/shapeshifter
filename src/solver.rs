use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::coverage::{has_sufficient_coverage, precompute_suffix_coverage, CoverageCounter};
use crate::game::Game;

/// A solution is a list of (row, col) placements, one per piece in original order.
pub type Solution = Vec<(usize, usize)>;

/// Flood-fill one connected component from a seed bit within `region`.
/// Returns the component mask. Uses bitboard-parallel expansion.
fn flood_fill(seed_bit: u32, region: Bitboard) -> Bitboard {
    let mut component = Bitboard::from_bit(seed_bit);
    loop {
        // Expand in 4 cardinal directions, masked to valid region.
        let expanded = component
            | (component << 1)
            | (component >> 1)
            | (component << 15)
            | (component >> 15);
        let expanded = expanded & region;
        if expanded == component {
            break;
        }
        component = expanded;
    }
    component
}

/// Check connected components of the non-zero region (using locked cells as walls).
/// For each component, verify that reachable pieces have enough perimeter to smooth
/// out the component's jaggedness.
/// Check connected components of the non-zero region (using locked cells as walls).
/// For each component, verify:
/// - Reachable pieces have enough cell_counts to cover min_flips
/// - Reachable pieces have enough perimeter to cover jaggedness
/// Also computes sum of active_planes across components (returned for caller to check).
fn check_components(
    board: &Board,
    locked_mask: Bitboard,
    reaches: &[Bitboard],
    perimeters: &[u32],
    cell_counts: &[u32],
    m: u8,
    piece_idx: usize,
) -> bool {

    // Non-zero region, excluding locked cells (which are walls).
    let mut nz = Bitboard::ZERO;
    for d in 1..m {
        nz |= board.plane(d);
    }
    let region = nz & !locked_mask;

    if region.is_zero() {
        return true;
    }

    let mut remaining_nz = region;
    let mut component_count = 0u32;

    while !remaining_nz.is_zero() {
        let seed = remaining_nz.lowest_set_bit();
        let component = flood_fill(seed, remaining_nz);
        remaining_nz = remaining_nz & !component;
        component_count += 1;

        // Compute component's min_flips.
        let mut comp_min_flips = 0u32;
        for d in 1..m {
            comp_min_flips += (m - d) as u32 * (board.plane(d) & component).count_ones();
        }

        // Component jaggedness.
        let h_pairs = component & (component >> 1);
        let v_pairs = component & (component >> 15);
        let total_pairs = h_pairs.count_ones() + v_pairs.count_ones();
        let mut matching = 0u32;
        for d in 0..m {
            let p = board.plane(d) & component;
            matching += (p & (p >> 1) & h_pairs).count_ones();
            matching += (p & (p >> 15) & v_pairs).count_ones();
        }
        let comp_jaggedness = total_pairs - matching;

        // Sum perimeter and cell_counts of remaining pieces that can reach this component.
        let mut reachable_perimeter = 0u32;
        let mut reachable_bits = 0u32;
        for pi in piece_idx..reaches.len() {
            if !(reaches[pi] & component).is_zero() {
                reachable_perimeter += perimeters[pi];
                reachable_bits += cell_counts[pi];
            }
        }

        // Per-component pruning checks.
        if comp_jaggedness > reachable_perimeter {
            return false;
        }
        if comp_min_flips > reachable_bits {
            return false;
        }

        if component_count >= 16 {
            break;
        }
    }

    true
}

/// Backtracking solver with pruning.
/// Pieces are sorted so larger/more-constrained pieces are tried first.
/// Duplicate pieces are detected and their permutations are pruned.
/// Trailing 1x1 pieces are solved directly without search.
pub fn solve(game: &Game) -> Option<Solution> {
    let board = game.board().clone();
    let pieces = game.pieces();
    let h = board.height();
    let w = board.width();

    // Build (original_index, placements) and sort: fewer placements first.
    // Secondary sort by shape to group duplicates together.
    let mut indexed: Vec<(usize, Vec<(usize, usize, Bitboard)>)> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.placements(h, w)))
        .collect();
    indexed.sort_by(|(i, a_pl), (j, b_pl)| {
        a_pl.len()
            .cmp(&b_pl.len())
            .then_with(|| pieces[*j].perimeter().cmp(&pieces[*i].perimeter()))
            .then_with(|| pieces[*j].cell_count().cmp(&pieces[*i].cell_count()))
            .then_with(|| pieces[*i].shape().limbs.cmp(&pieces[*j].shape().limbs))
    });

    let order: Vec<usize> = indexed.iter().map(|(i, _)| *i).collect();
    let all_placements: Vec<Vec<(usize, usize, Bitboard)>> =
        indexed.into_iter().map(|(_, p)| p).collect();

    let n = pieces.len();

    // Detect which pieces are duplicates of their predecessor (same shape).
    let is_dup_of_prev: Vec<bool> = (0..n)
        .map(|i| i > 0 && pieces[order[i]] == pieces[order[i - 1]])
        .collect();

    // Find where trailing 1x1 pieces start (they're sorted last = most placements).
    let single_cell_start = (0..n)
        .rposition(|i| pieces[order[i]].cell_count() != 1)
        .map(|i| i + 1)
        .unwrap_or(0);

    // Precompute suffix sums of piece cell counts and perimeters in sorted order.
    let mut remaining_bits = vec![0u32; n + 1];
    let mut remaining_perimeter = vec![0u32; n + 1];
    for i in (0..n).rev() {
        remaining_bits[i] = remaining_bits[i + 1] + pieces[order[i]].cell_count();
        remaining_perimeter[i] = remaining_perimeter[i + 1] + pieces[order[i]].perimeter();
    }

    // Precompute per-piece reach: union of all placement masks.
    let reaches: Vec<Bitboard> = all_placements
        .iter()
        .map(|placements| {
            let mut reach = Bitboard::ZERO;
            for &(_, _, mask) in placements {
                reach |= mask;
            }
            reach
        })
        .collect();

    // Precompute suffix coverage in binary bitboard layers.
    let suffix_coverage = precompute_suffix_coverage(&reaches);

    // Precompute sorted-order perimeters and cell counts for component checks.
    let sorted_perimeters: Vec<u32> = (0..n).map(|i| pieces[order[i]].perimeter()).collect();
    let sorted_cell_counts: Vec<u32> = (0..n).map(|i| pieces[order[i]].cell_count()).collect();

    let m = board.m();

    let mut sorted_solution = Vec::with_capacity(n);
    if backtrack(
        &board,
        &all_placements,
        &reaches,
        &sorted_perimeters,
        &sorted_cell_counts,
        &remaining_bits,
        &remaining_perimeter,
        &suffix_coverage,
        &is_dup_of_prev,
        m,
        h,
        w,
        single_cell_start,
        0,
        0,
        &mut sorted_solution,
    ) {
        // Map solution back to original piece order.
        let mut solution = vec![(0, 0); n];
        for (sorted_idx, &(row, col)) in sorted_solution.iter().enumerate() {
            solution[order[sorted_idx]] = (row, col);
        }
        Some(solution)
    } else {
        None
    }
}

/// Try to solve remaining pieces when they're all 1x1.
/// Each cell at value d needs (M-d)%M hits. Total hits must equal number of pieces.
/// Returns true and fills solution if solvable.
fn solve_single_cells(
    board: &Board,
    m: u8,
    h: u8,
    w: u8,
    num_pieces: usize,
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    // Count total hits needed and verify it matches available pieces.
    let mut needed = 0u32;
    for d in 1..m {
        needed += (m - d) as u32 * board.plane(d).count_ones();
    }
    if needed as usize != num_pieces {
        return false;
    }

    // Assign pieces to cells: for each non-zero cell, emit (M-d) placements.
    // Process cells in row-major order.
    let base_len = solution.len();
    for r in 0..h as usize {
        for c in 0..w as usize {
            let val = board.get(r, c);
            if val != 0 {
                let hits = (m - val) as usize;
                for _ in 0..hits {
                    solution.push((r, c));
                }
            }
        }
    }

    debug_assert_eq!(solution.len() - base_len, num_pieces);
    true
}

fn backtrack(
    board: &Board,
    all_placements: &[Vec<(usize, usize, Bitboard)>],
    reaches: &[Bitboard],
    perimeters: &[u32],
    cell_counts: &[u32],
    remaining_bits: &[u32],
    remaining_perimeter: &[u32],
    suffix_coverage: &[CoverageCounter],
    is_dup_of_prev: &[bool],
    m: u8,
    h: u8,
    w: u8,
    single_cell_start: usize,
    piece_idx: usize,
    min_placement: usize,
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    if piece_idx == all_placements.len() {
        return board.is_solved();
    }

    // If all remaining pieces are 1x1, solve directly.
    if piece_idx >= single_cell_start {
        let num_remaining = all_placements.len() - piece_idx;
        return solve_single_cells(board, m, h, w, num_remaining, solution);
    }

    let remaining = all_placements.len() - piece_idx;

    // Prune: each piece can eliminate at most one active plane.
    if board.active_planes() as usize > remaining {
        return false;
    }

    // Prune: if remaining piece bits can't cover the minimum flips needed.
    let min_flips = board.min_flips_needed();
    if remaining_bits[piece_idx] < min_flips {
        return false;
    }

    // Prune: insufficient coverage per cell.
    if !has_sufficient_coverage(board, &suffix_coverage[piece_idx], m) {
        return false;
    }

    // Prune: jaggedness exceeds total remaining perimeter.
    if board.jaggedness() > remaining_perimeter[piece_idx] {
        return false;
    }

    // Compute locked mask: cells at 0 where remaining coverage < M.
    let locked_mask = board.plane(0) & !suffix_coverage[piece_idx].coverage_ge(m);

    // Prune: per-component checks (jaggedness, min_flips, active_planes).
    // Only at top levels where the subtree justifies the flood-fill cost.
    if piece_idx < 4 {
        if !check_components(
            board, locked_mask, reaches, perimeters, cell_counts,
            m, piece_idx,
        ) {
            return false;
        }
    }

    // Micro-opt: unavoidable waste for the current piece. If every valid placement
    // hits at least Z zero cells, the budget must absorb M*Z waste.
    // Equivalent to the min_flips check at depth+1, but avoids iterating all placements.
    let placements = &all_placements[piece_idx];
    let zero_plane = board.plane(0);
    let mut min_zero_hit = u32::MAX;
    for (pl_idx, &(_, _, mask)) in placements.iter().enumerate() {
        if pl_idx < min_placement {
            continue;
        }
        if !(mask & locked_mask).is_zero() {
            continue;
        }
        let z = (mask & zero_plane).count_ones();
        if z < min_zero_hit {
            min_zero_hit = z;
        }
        if min_zero_hit == 0 {
            break;
        }
    }
    if min_zero_hit > 0 && min_zero_hit < u32::MAX {
        if remaining_bits[piece_idx] < min_flips + m as u32 * min_zero_hit {
            return false;
        }
    }

    let mut board = board.clone();
    for (pl_idx, &(row, col, mask)) in placements.iter().enumerate() {
        if pl_idx < min_placement {
            continue;
        }

        // Skip placements that touch locked cells.
        if !(mask & locked_mask).is_zero() {
            continue;
        }

        board.apply_piece(mask);
        solution.push((row, col));

        let next_min = if piece_idx + 1 < all_placements.len()
            && is_dup_of_prev[piece_idx + 1]
        {
            pl_idx
        } else {
            0
        };

        if backtrack(
            &board,
            all_placements,
            reaches,
            perimeters,
            cell_counts,
            remaining_bits,
            remaining_perimeter,
            suffix_coverage,
            is_dup_of_prev,
            m,
            h,
            w,
            single_cell_start,
            piece_idx + 1,
            next_min,
            solution,
        ) {
            return true;
        }

        solution.pop();
        board.undo_piece(mask);
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::game::Game;
    use crate::piece::Piece;

    fn verify_solution(game: &Game, solution: &Solution) {
        let mut board = game.board().clone();
        for (i, &(row, col)) in solution.iter().enumerate() {
            let mask = game.pieces()[i].placed_at(row, col);
            board.apply_piece(mask);
        }
        assert!(board.is_solved(), "solution did not solve the board");
    }

    #[test]
    fn test_trivial_solve() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), 1);
        assert_eq!(sol[0], (0, 0));
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_two_pieces() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece, piece]);
        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_no_solution() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game).is_none());
    }

    #[test]
    fn test_all_single_cells() {
        // 3x3, m=2. Board all 1s. Nine 1x1 pieces.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 9]);
        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), 9);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_single_cells_m3() {
        // 3x3, m=3. Cell (0,0)=1 needs 2 hits, cell (0,1)=2 needs 1 hit. 3 pieces total.
        let grid: &[&[u8]] = &[&[1, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 3]);
        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), 3);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_single_cells_insufficient() {
        // 3x3, m=2. Two 1s but only one piece.
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game).is_none());
    }

    #[test]
    fn test_mixed_then_single() {
        // Mix of multi-cell and single-cell pieces.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let big = Piece::from_grid(&[&[true, true], &[true, false]]); // L-shape, 3 cells
        let small = Piece::from_grid(&[&[true]]); // 1x1
        let game = Game::new(board, vec![big, small]);
        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_generated_game_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
        let game = crate::generate::generate_for_level(1, &mut rng).unwrap();
        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), game.pieces().len());
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_generated_level_5_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(123);
        let game = crate::generate::generate_for_level(5, &mut rng).unwrap();
        let sol = solve(&game).unwrap();
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_min_flips_pruning() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.min_flips_needed(), 9);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game).is_none());
    }

    #[test]
    fn test_solution_maps_to_original_order() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let p0 = Piece::from_grid(&[&[true]]);
        let p1 = Piece::from_grid(&[&[true, true]]);
        let game = Game::new(board, vec![p0, p1]);
        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_coverage_pruning_unreachable() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true], &[true], &[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game).is_none());
    }

    #[test]
    fn test_generated_levels_solvable() {
        for level in [1, 5, 10, 20, 25, 30] {
            let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
            let game = crate::generate::generate_for_level(level, &mut rng).unwrap();
            let sol = solve(&game);
            assert!(sol.is_some(), "level {level} should be solvable");
            verify_solution(&game, &sol.unwrap());
        }
    }

    /// Fuzz test: generate many random games across a variety of board sizes, M values,
    /// and piece counts. Every generated game is guaranteed solvable by construction.
    /// Verify the solver finds a valid solution for each.
    #[test]
    fn test_fuzz_soundness() {
        use rayon::prelude::*;
        use crate::generate::generate_game;
        use crate::level::LevelSpec;

        // Test configurations: (M, rows, cols, num_pieces)
        // Keep piece counts low enough to be solvable within reasonable time.
        let configs: Vec<(u8, u8, u8, u8)> = vec![
            // Small boards, M=2
            (2, 3, 3, 2), (2, 3, 3, 4), (2, 3, 3, 6), (2, 3, 3, 8),
            // Small boards, M=3
            (3, 3, 3, 3), (3, 3, 3, 5), (3, 3, 3, 7),
            // Medium boards, M=2
            (2, 4, 3, 5), (2, 4, 3, 8), (2, 4, 3, 12),
            (2, 4, 4, 6), (2, 4, 4, 10), (2, 4, 4, 14),
            // Medium boards, M=3
            (3, 4, 3, 6), (3, 4, 3, 10),
            (3, 4, 4, 8), (3, 4, 4, 12),
            // Medium boards, M=4
            (4, 4, 4, 6), (4, 4, 4, 10),
            // Larger boards, M=2
            (2, 6, 6, 8), (2, 6, 6, 12),
            // Larger boards, M=3
            (3, 6, 6, 8), (3, 6, 6, 12),
            // Larger boards, M=4
            (4, 6, 6, 8), (4, 6, 6, 10),
            // Larger boards, M=5
            (5, 6, 6, 6), (5, 6, 6, 8),
            // Big boards, low piece count
            (2, 8, 7, 8), (3, 8, 7, 8), (4, 8, 8, 8),
            (2, 10, 10, 8), (3, 10, 10, 8), (4, 10, 10, 8),
        ];

        let seeds: Vec<u64> = (0..50).collect();

        let failures: Vec<String> = configs
            .par_iter()
            .flat_map(|&(m, rows, cols, shapes)| {
                let spec = LevelSpec {
                    level: 0,
                    shifts: m,
                    rows,
                    columns: cols,
                    shapes,
                    preview: false,
                };
                seeds.par_iter().filter_map(move |&seed| {
                    let mut rng =
                        <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                    let game = generate_game(&spec, &mut rng);
                    let sol = solve(&game);
                    match sol {
                        None => Some(format!(
                            "FAIL: no solution found for M={} {}x{} pieces={} seed={}",
                            m, rows, cols, shapes, seed
                        )),
                        Some(ref s) => {
                            // Verify the solution is correct.
                            let mut board = game.board().clone();
                            for (i, &(row, col)) in s.iter().enumerate() {
                                let mask = game.pieces()[i].placed_at(row, col);
                                board.apply_piece(mask);
                            }
                            if !board.is_solved() {
                                Some(format!(
                                    "FAIL: invalid solution for M={} {}x{} pieces={} seed={}",
                                    m, rows, cols, shapes, seed
                                ))
                            } else {
                                None
                            }
                        }
                    }
                }).collect::<Vec<_>>()
            })
            .collect();

        if !failures.is_empty() {
            for f in &failures[..failures.len().min(20)] {
                eprintln!("{}", f);
            }
            panic!("{} fuzz test failures (showing first 20)", failures.len());
        }
    }
}
