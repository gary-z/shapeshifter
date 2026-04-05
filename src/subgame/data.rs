use std::simd::u16x16;

use crate::core::board::Board;
use crate::core::piece::Piece;

use super::board::SubgameBoard;
use super::game::SubgameGame;
use super::generate::{board_col_deficits, board_row_deficits, piece_col_profile, piece_row_profile};
use super::piece::SubgamePiece;
use super::solver::SubgameAxisPrune;

/// Precomputed subgame data for all pieces in solver order.
///
/// Built once during solver precomputation. Provides O(1) lookup of shifted
/// profiles by `(piece_index, position)` so that [`super::state::SubgameState`]
/// can be updated incrementally during the main search with no allocation.
///
/// Also holds precomputed pruning data (parity, subset SAT, count-sat, endgame)
/// for fast feasibility checks during search.
pub struct SubgameData {
    /// Initial row subgame board (before any pieces are placed).
    row_board: SubgameBoard,
    /// Initial column subgame board (before any pieces are placed).
    col_board: SubgameBoard,
    /// Row profile for each piece (in solver piece order).
    row_profiles: Vec<SubgamePiece>,
    /// Column profile for each piece (in solver piece order).
    col_profiles: Vec<SubgamePiece>,
    /// `row_shifted[piece_idx]` maps row position → shifted u16x16 profile.
    row_shifted: Vec<Vec<u16x16>>,
    /// `col_shifted[piece_idx]` maps col position → shifted u16x16 profile.
    col_shifted: Vec<Vec<u16x16>>,
    /// Precomputed placements for each piece in the row subgame:
    /// `row_placements[i]` = Vec of `(position, shifted_profile)`.
    row_placements: Vec<Vec<(usize, u16x16)>>,
    /// Precomputed placements for each piece in the col subgame.
    col_placements: Vec<Vec<(usize, u16x16)>>,
    /// Precomputed pruning data for row subgame feasibility checks.
    row_prune: SubgameAxisPrune,
    /// Precomputed pruning data for col subgame feasibility checks.
    col_prune: SubgameAxisPrune,
}

impl SubgameData {
    /// Build subgame data from the full board and pieces in solver order.
    ///
    /// `order[i]` maps solver piece index → original piece index.
    /// The resulting profiles and shifted tables follow solver piece order.
    pub fn build(board: &Board, pieces: &[Piece], order: &[usize]) -> Self {
        let row_board = board_row_deficits(board);
        let col_board = board_col_deficits(board);
        let board_h = board.height();
        let board_w = board.width();

        let n = order.len();
        let mut row_profiles = Vec::with_capacity(n);
        let mut col_profiles = Vec::with_capacity(n);
        let mut row_shifted = Vec::with_capacity(n);
        let mut col_shifted = Vec::with_capacity(n);
        let mut row_placements = Vec::with_capacity(n);
        let mut col_placements = Vec::with_capacity(n);

        for &orig_idx in order {
            let piece = &pieces[orig_idx];

            let rp = piece_row_profile(piece);
            let row_pls = rp.placements(board_h);
            row_shifted.push(row_pls.iter().map(|&(_, shifted)| shifted).collect());
            row_placements.push(row_pls);
            row_profiles.push(rp);

            let cp = piece_col_profile(piece);
            let col_pls = cp.placements(board_w);
            col_shifted.push(col_pls.iter().map(|&(_, shifted)| shifted).collect());
            col_placements.push(col_pls);
            col_profiles.push(cp);
        }

        // Build pruning data for feasibility checks.
        // Construct temporary SubgameGames to drive the precomputation.
        let row_prune = if !row_profiles.is_empty() {
            let row_game = SubgameGame::from_parts(
                row_board, row_profiles.clone(), row_placements.clone(),
            );
            SubgameAxisPrune::precompute(&row_game)
        } else {
            SubgameAxisPrune::empty()
        };
        let col_prune = if !col_profiles.is_empty() {
            let col_game = SubgameGame::from_parts(
                col_board, col_profiles.clone(), col_placements.clone(),
            );
            SubgameAxisPrune::precompute(&col_game)
        } else {
            SubgameAxisPrune::empty()
        };

        Self {
            row_board,
            col_board,
            row_profiles,
            col_profiles,
            row_shifted,
            col_shifted,
            row_placements,
            col_placements,
            row_prune,
            col_prune,
        }
    }

    /// Initial row subgame board.
    #[inline(always)]
    pub fn row_board(&self) -> &SubgameBoard {
        &self.row_board
    }

    /// Initial column subgame board.
    #[inline(always)]
    pub fn col_board(&self) -> &SubgameBoard {
        &self.col_board
    }

    /// Row profile for piece at solver index `i`.
    #[inline(always)]
    pub fn row_profile(&self, i: usize) -> &SubgamePiece {
        &self.row_profiles[i]
    }

    /// Column profile for piece at solver index `i`.
    #[inline(always)]
    pub fn col_profile(&self, i: usize) -> &SubgamePiece {
        &self.col_profiles[i]
    }

    /// Shifted row profile for piece `piece_idx` placed at row `row`.
    #[inline(always)]
    pub fn row_shifted_at(&self, piece_idx: usize, row: usize) -> u16x16 {
        self.row_shifted[piece_idx][row]
    }

    /// Shifted col profile for piece `piece_idx` placed at column `col`.
    #[inline(always)]
    pub fn col_shifted_at(&self, piece_idx: usize, col: usize) -> u16x16 {
        self.col_shifted[piece_idx][col]
    }

    /// Number of valid row positions for piece `piece_idx`.
    #[inline(always)]
    pub fn num_row_positions(&self, piece_idx: usize) -> usize {
        self.row_shifted[piece_idx].len()
    }

    /// Number of valid col positions for piece `piece_idx`.
    #[inline(always)]
    pub fn num_col_positions(&self, piece_idx: usize) -> usize {
        self.col_shifted[piece_idx].len()
    }

    /// Number of pieces.
    #[inline(always)]
    pub fn num_pieces(&self) -> usize {
        self.row_profiles.len()
    }

    /// All row profiles (in solver order).
    pub fn row_profiles(&self) -> &[SubgamePiece] {
        &self.row_profiles
    }

    /// All column profiles (in solver order).
    pub fn col_profiles(&self) -> &[SubgamePiece] {
        &self.col_profiles
    }

    /// Check if both row and col subgames are feasible from `from_piece` onward.
    ///
    /// Uses precomputed pruning data (parity, subset SAT, count-sat, endgame)
    /// with a node budget to bound cost.
    ///
    /// Returns `(feasible, subgame_nodes_visited)`.
    pub fn check_feasible(
        &self,
        row_board: SubgameBoard,
        col_board: SubgameBoard,
        from_piece: usize,
    ) -> (bool, u64) {
        let mut nodes = 0u64;

        let (row_ok, row_nodes) = self.row_prune.check_feasible(
            row_board, from_piece, &self.row_placements,
        );
        nodes += row_nodes;
        if !row_ok {
            return (false, nodes);
        }

        let (col_ok, col_nodes) = self.col_prune.check_feasible(
            col_board, from_piece, &self.col_placements,
        );
        nodes += col_nodes;
        if !col_ok {
            return (false, nodes);
        }

        (true, nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    fn make_test_data() -> SubgameData {
        // 3x3 M=2, all deficit 1 → 9 cells of deficit.
        // Two pieces: 1x1 (1 cell) + 1x2 (2 cells) = 3 cells total.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let p1 = Piece::from_grid(&[&[true]]);
        let p2 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1, p2];
        let order = vec![0, 1];
        SubgameData::build(&board, &pieces, &order)
    }

    #[test]
    fn test_build_basic() {
        let data = make_test_data();
        assert_eq!(data.num_pieces(), 2);
    }

    #[test]
    fn test_build_nontrivial() {
        let grid: &[&[u8]] = &[&[1, 0, 1], &[0, 1, 0], &[1, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let p1 = Piece::from_grid(&[&[true, true]]);
        let p2 = Piece::from_grid(&[&[true], &[true]]);
        let pieces = vec![p1, p2];
        let order = vec![0, 1];
        let data = SubgameData::build(&board, &pieces, &order);
        assert_eq!(data.num_pieces(), 2);
        assert_eq!(data.row_profiles().len(), 2);
        assert_eq!(data.col_profiles().len(), 2);
    }

    #[test]
    fn test_build_profiles_and_placements() {
        let board = Board::new_solved(4, 4, 2);
        let p1 = Piece::from_grid(&[&[true]]);
        let p2 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1, p2];
        let order = vec![0, 1];
        let data = SubgameData::build(&board, &pieces, &order);
        assert_eq!(data.num_pieces(), 2);
        assert_eq!(data.row_profiles()[0].cell_count(), 1);
        assert_eq!(data.row_profiles()[1].cell_count(), 2);
    }

    #[test]
    fn test_shifted_profiles() {
        let data = make_test_data();
        assert_eq!(data.num_row_positions(0), 3);
        assert_eq!(data.num_row_positions(1), 3);
    }

    #[test]
    fn test_solver_order_respected() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let p1 = Piece::from_grid(&[&[true]]);
        let p2 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1, p2];
        let order = vec![1, 0]; // reversed
        let data = SubgameData::build(&board, &pieces, &order);
        assert_eq!(data.row_profile(0).cell_count(), 2);
        assert_eq!(data.row_profile(1).cell_count(), 1);
    }

    #[test]
    fn test_feasible_solvable_subgame() {
        // 3x3 M=2, all deficit 1. 9 single-cell pieces → solvable.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces: Vec<Piece> = (0..9).map(|_| p.clone()).collect();
        let order: Vec<usize> = (0..9).collect();
        let data = SubgameData::build(&board, &pieces, &order);
        let (ok, _nodes) = data.check_feasible(*data.row_board(), *data.col_board(), 0);
        assert!(ok);
    }

    #[test]
    fn test_infeasible_deficit_mismatch() {
        // 3x3 M=2, all deficit 1 → total 9. Only 2 pieces → infeasible.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p.clone(), p.clone()];
        let order = vec![0, 1];
        let data = SubgameData::build(&board, &pieces, &order);
        let (ok, _nodes) = data.check_feasible(*data.row_board(), *data.col_board(), 0);
        assert!(!ok);
    }
}
