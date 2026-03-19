use crate::board::Board;
use crate::game::Game;

/// A solution is a list of (row, col) placements, one per piece in order.
pub type Solution = Vec<(usize, usize)>;

/// Brute-force backtracking solver.
pub fn solve(game: &Game) -> Option<Solution> {
    let board = game.board().clone();
    let pieces = game.pieces();
    let mut solution = Vec::with_capacity(pieces.len());

    // Precompute all valid placements for each piece.
    let h = board.height();
    let w = board.width();
    let all_placements: Vec<Vec<(usize, usize, _)>> = pieces
        .iter()
        .map(|p| p.placements(h, w))
        .collect();

    if backtrack(&board, &all_placements, 0, &mut solution) {
        Some(solution)
    } else {
        None
    }
}

fn backtrack(
    board: &Board,
    all_placements: &[Vec<(usize, usize, crate::bitboard::Bitboard)>],
    piece_idx: usize,
    solution: &mut Solution,
) -> bool {
    if piece_idx == all_placements.len() {
        return board.is_solved();
    }

    let mut board = board.clone();
    for &(row, col, mask) in &all_placements[piece_idx] {
        board.apply_piece(mask);
        solution.push((row, col));

        if backtrack(&board, all_placements, piece_idx + 1, solution) {
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

    /// Verify a solution by replaying it and checking the board is solved.
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
        // 3x3, m=2. One 1x1 piece. Board has a single 1, rest 0.
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
        // 3x3, m=2. Two 1x1 pieces. Board has two 1s.
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
        // 3x3, m=3. Board all 1s. One 1x1 piece can only increment one cell.
        // After placing: one cell becomes 2, rest stay 1. Not solvable.
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
}
