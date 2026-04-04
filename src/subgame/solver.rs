use super::game::SubgameGame;

/// Result of a subgame solve attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgameSolveResult {
    /// A solution was found. Contains the placement positions (one per piece).
    Solved(Vec<usize>),
    /// No solution exists.
    Unsolvable,
}

/// Statistics tracked during solving.
#[derive(Debug, Clone, Default)]
pub struct SolverStats {
    /// Total number of nodes visited in the search tree.
    pub nodes_visited: u64,
    /// Number of placements rejected by underflow detection.
    pub underflow_rejections: u64,
    /// Number of branches pruned by total deficit check.
    pub deficit_prunes: u64,
}

/// Naive brute-force subgame solver with backtracking.
///
/// Tries all placements of each piece in order. The structure is designed
/// to make adding pruning techniques straightforward:
///
/// - **Total deficit pruning**: remaining piece cells must exactly equal remaining deficit.
/// - **Per-cell bounds**: each cell's deficit must be achievable by remaining pieces.
/// - **Duplicate piece symmetry breaking**: skip placements that reorder identical pieces.
///
/// Currently only total-deficit pruning is implemented. The solver returns the
/// first solution found, or `Unsolvable` if none exists.
pub struct SubgameSolver {
    /// The subgame to solve.
    game: SubgameGame,
    /// Placement positions chosen so far (one per placed piece).
    solution: Vec<usize>,
    /// Solver statistics.
    stats: SolverStats,
}

impl SubgameSolver {
    /// Create a new solver for the given subgame.
    pub fn new(game: SubgameGame) -> Self {
        let n = game.pieces().len();
        Self {
            game,
            solution: Vec::with_capacity(n),
            stats: SolverStats::default(),
        }
    }

    /// Solve the subgame. Returns the result and solver statistics.
    pub fn solve(mut self) -> (SubgameSolveResult, SolverStats) {
        // Quick feasibility check: total piece cells must equal total deficit.
        let total_cells = self.game.remaining_cells_from(0);
        let total_deficit = self.game.board().total_deficit();
        if total_cells != total_deficit {
            return (SubgameSolveResult::Unsolvable, self.stats);
        }

        if self.game.board().is_solved() && self.game.pieces().is_empty() {
            return (SubgameSolveResult::Solved(vec![]), self.stats);
        }

        let found = self.backtrack(0);
        if found {
            (SubgameSolveResult::Solved(self.solution.clone()), self.stats)
        } else {
            (SubgameSolveResult::Unsolvable, self.stats)
        }
    }

    /// Recursive backtracking search.
    ///
    /// `depth` is the index of the current piece to place.
    fn backtrack(&mut self, depth: usize) -> bool {
        self.stats.nodes_visited += 1;

        // Base case: all pieces placed.
        if depth >= self.game.pieces().len() {
            return self.game.board().is_solved();
        }

        // --- Pruning: total deficit check ---
        // Remaining piece cells must exactly equal remaining deficit.
        let remaining_cells = self.game.remaining_cells_from(depth);
        let remaining_deficit = self.game.board().total_deficit();
        if remaining_cells != remaining_deficit {
            self.stats.deficit_prunes += 1;
            return false;
        }

        // Try each placement for the current piece.
        let placements = self.game.placements_for(depth).to_vec();
        for &(pos, shifted) in &placements {
            // Try applying the placement.
            if !self.game.board_mut().apply_piece(shifted) {
                self.stats.underflow_rejections += 1;
                continue;
            }

            self.solution.push(pos);

            // Recurse to the next piece.
            if self.backtrack(depth + 1) {
                return true;
            }

            // Undo and try the next placement.
            self.solution.pop();
            self.game.board_mut().undo_piece(shifted);
        }

        false
    }
}

/// Convenience function: check if a subgame is solvable.
pub fn is_solvable(game: SubgameGame) -> bool {
    let solver = SubgameSolver::new(game);
    matches!(solver.solve().0, SubgameSolveResult::Solved(_))
}

/// Convenience function: solve a subgame and return the result with stats.
pub fn solve(game: SubgameGame) -> (SubgameSolveResult, SolverStats) {
    SubgameSolver::new(game).solve()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subgame::board::SubgameBoard;
    use crate::subgame::piece::SubgamePiece;
    use crate::subgame::game::SubgameGame;
    use crate::subgame::generate::{to_row_subgame, to_col_subgame};
    use crate::core::board::Board;
    use crate::core::piece::Piece;
    use crate::game::Game;

    #[test]
    fn test_solve_trivial() {
        // Board [1], piece [1] -> place at 0
        let board = SubgameBoard::from_cells(&[1]);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, stats) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0]));
        assert!(stats.nodes_visited >= 1);
    }

    #[test]
    fn test_solve_two_pieces() {
        // Board [2, 2], two pieces with profile [1, 1]
        // Both placed at position 0: [2,2] - [1,1] - [1,1] = [0,0]
        let board = SubgameBoard::from_cells(&[2, 2]);
        let piece = SubgamePiece::from_profile(&[1, 1]);
        let game = SubgameGame::new(board, vec![piece, piece]);
        let (result, _) = solve(game);
        match result {
            SubgameSolveResult::Solved(positions) => {
                assert_eq!(positions.len(), 2);
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn test_solve_unsolvable_deficit_mismatch() {
        // Board [3], piece [2] -> total deficit 3 != piece cells 2
        let board = SubgameBoard::from_cells(&[3]);
        let piece = SubgamePiece::from_profile(&[2]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, _) = solve(game);
        assert_eq!(result, SubgameSolveResult::Unsolvable);
    }

    #[test]
    fn test_solve_unsolvable_no_valid_placement() {
        // Board [1, 3], piece [2, 2] -> deficit matches (4 = 4) but cell 0 underflows
        let board = SubgameBoard::from_cells(&[1, 3]);
        let piece = SubgamePiece::from_profile(&[2, 2]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, stats) = solve(game);
        assert_eq!(result, SubgameSolveResult::Unsolvable);
        assert!(stats.underflow_rejections > 0);
    }

    #[test]
    fn test_solve_multiple_placements() {
        // Board [1, 0, 1], piece [1] twice
        // First piece at pos 0, second at pos 2 (or vice versa)
        let board = SubgameBoard::from_cells(&[1, 0, 1]);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece, piece]);
        let (result, _) = solve(game);
        match result {
            SubgameSolveResult::Solved(positions) => {
                assert_eq!(positions.len(), 2);
                let mut sorted = positions.clone();
                sorted.sort();
                assert_eq!(sorted, vec![0, 2]);
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn test_solve_larger_board() {
        // Board [2, 2, 2, 2], two pieces with profile [1, 1, 1, 1]
        let board = SubgameBoard::from_cells(&[2, 2, 2, 2]);
        let piece = SubgamePiece::from_profile(&[1, 1, 1, 1]);
        let game = SubgameGame::new(board, vec![piece, piece]);
        let (result, _) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0, 0]));
    }

    #[test]
    fn test_design_counterexample_both_subgames_solvable() {
        // From DESIGN.md: 3x3, M=3, three 1x3 horizontal bars.
        // Both subgames should be solvable even though full game is not.
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 0, 1], &[1, 2, 0]];
        let board = Board::from_grid(grid, 3);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let game = Game::new(board, vec![bar, bar, bar]);

        let row_sg = to_row_subgame(&game);
        let col_sg = to_col_subgame(&game);

        assert!(is_solvable(row_sg));
        assert!(is_solvable(col_sg));
    }

    #[test]
    fn test_solve_stats_tracking() {
        let board = SubgameBoard::from_cells(&[1, 1]);
        let piece = SubgamePiece::from_profile(&[1, 1]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, stats) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0]));
        assert!(stats.nodes_visited >= 2); // root + base case
    }

    #[test]
    fn test_is_solvable_convenience() {
        let board = SubgameBoard::from_cells(&[1]);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece]);
        assert!(is_solvable(game));
    }

    #[test]
    fn test_solve_generated_game_row_subgame() {
        // Generate a game by working backwards from a solved board, then check
        // that the row subgame is solvable.
        let board = Board::new_solved(3, 3, 2);
        let p1 = Piece::from_grid(&[&[true, true], &[true, false]]);
        let p2 = Piece::from_grid(&[&[true]]);

        // Build a game by undoing pieces from a solved board.
        let mut b = board;
        let mask1 = p1.placed_at(0, 0);
        b.undo_piece(mask1);
        let mask2 = p2.placed_at(1, 1);
        b.undo_piece(mask2);
        let game = Game::new(b, vec![p1, p2]);

        let row_sg = to_row_subgame(&game);
        assert!(is_solvable(row_sg));
    }

    #[test]
    fn test_solve_generated_game_col_subgame() {
        let board = Board::new_solved(3, 3, 2);
        let p1 = Piece::from_grid(&[&[true, true], &[true, false]]);
        let p2 = Piece::from_grid(&[&[true]]);

        let mut b = board;
        let mask1 = p1.placed_at(0, 0);
        b.undo_piece(mask1);
        let mask2 = p2.placed_at(1, 1);
        b.undo_piece(mask2);
        let game = Game::new(b, vec![p1, p2]);

        let col_sg = to_col_subgame(&game);
        assert!(is_solvable(col_sg));
    }

    #[test]
    fn test_solve_with_different_profiles() {
        // Board [3, 2, 1], pieces: [2, 1] and [1, 1, 1]
        // Piece 0 at pos 0: [3,2,1]-[2,1,0]=[1,1,1], then piece 1 at pos 0: [1,1,1]-[1,1,1]=[0,0,0]
        let board = SubgameBoard::from_cells(&[3, 2, 1]);
        let p1 = SubgamePiece::from_profile(&[2, 1]);
        let p2 = SubgamePiece::from_profile(&[1, 1, 1]);
        let game = SubgameGame::new(board, vec![p1, p2]);
        let (result, _) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0, 0]));
    }
}
