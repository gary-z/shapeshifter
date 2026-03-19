use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::coverage::{has_sufficient_coverage, precompute_suffix_coverage, CoverageCounter};
use crate::game::Game;

/// A solution is a list of (row, col) placements, one per piece in original order.
pub type Solution = Vec<(usize, usize)>;

/// Backtracking solver with pruning.
/// Pieces are sorted so larger/more-constrained pieces are tried first.
/// Duplicate pieces are detected and their permutations are pruned.
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
            .then_with(|| pieces[*i].shape().limbs.cmp(&pieces[*j].shape().limbs))
    });

    let order: Vec<usize> = indexed.iter().map(|(i, _)| *i).collect();
    let all_placements: Vec<Vec<(usize, usize, Bitboard)>> =
        indexed.into_iter().map(|(_, p)| p).collect();

    let n = pieces.len();

    // Detect which pieces are duplicates of their predecessor (same shape).
    // For a duplicate, we enforce placement index >= predecessor's placement index.
    let is_dup_of_prev: Vec<bool> = (0..n)
        .map(|i| i > 0 && pieces[order[i]] == pieces[order[i - 1]])
        .collect();

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

    let m = board.m();

    let mut sorted_solution = Vec::with_capacity(n);
    if backtrack(
        &board,
        &all_placements,
        &remaining_bits,
        &remaining_perimeter,
        &suffix_coverage,
        &is_dup_of_prev,
        m,
        0,
        0, // min_placement for first piece
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

fn backtrack(
    board: &Board,
    all_placements: &[Vec<(usize, usize, Bitboard)>],
    remaining_bits: &[u32],
    remaining_perimeter: &[u32],
    suffix_coverage: &[CoverageCounter],
    is_dup_of_prev: &[bool],
    m: u8,
    piece_idx: usize,
    min_placement: usize, // minimum placement index (for duplicate pruning)
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    if piece_idx == all_placements.len() {
        return board.is_solved();
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

    // Prune: total remaining increments must match needed increments mod M.
    if remaining_bits[piece_idx] % m as u32 != min_flips % m as u32 {
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

    let placements = &all_placements[piece_idx];
    let mut board = board.clone();
    for (pl_idx, &(row, col, mask)) in placements.iter().enumerate() {
        // Skip placements before min_placement (duplicate symmetry breaking).
        if pl_idx < min_placement {
            continue;
        }

        board.apply_piece(mask);
        solution.push((row, col));

        // If the next piece is a duplicate of this one, it must use placement >= pl_idx.
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
            remaining_bits,
            remaining_perimeter,
            suffix_coverage,
            is_dup_of_prev,
            m,
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
    fn test_many_duplicates() {
        // 3x3, m=2. Board all 1s. Nine identical 1x1 pieces should solve it.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 9]);

        let sol = solve(&game).unwrap();
        assert_eq!(sol.len(), 9);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_duplicate_pairs() {
        // Two pairs of identical pieces.
        let grid: &[&[u8]] = &[&[1, 1, 0], &[1, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let h_piece = Piece::from_grid(&[&[true, true]]); // horizontal domino
        let v_piece = Piece::from_grid(&[&[true], &[true]]); // vertical domino
        let game = Game::new(board, vec![h_piece, h_piece, v_piece, v_piece]);

        let sol = solve(&game);
        // May or may not be solvable with these pieces, but solver should not hang.
        if let Some(ref s) = sol {
            verify_solution(&game, s);
        }
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
}
