use std::simd::u16x16;

use crate::core::board::Board;
use crate::core::piece::Piece;

use super::board::SubgameBoard;
use super::generate::{board_col_deficits, board_row_deficits, piece_col_profile, piece_row_profile};
use super::piece::SubgamePiece;

/// Precomputed subgame data for all pieces in solver order.
///
/// Built once during solver precomputation. Provides O(1) lookup of shifted
/// profiles by `(piece_index, position)` so that [`super::state::SubgameState`]
/// can be updated incrementally during the main search with no allocation.
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
    ///
    /// Length of inner Vec = number of valid row positions for that piece
    /// (`board_height - piece_height + 1`).
    row_shifted: Vec<Vec<u16x16>>,
    /// `col_shifted[piece_idx]` maps col position → shifted u16x16 profile.
    col_shifted: Vec<Vec<u16x16>>,
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

        for &orig_idx in order {
            let piece = &pieces[orig_idx];

            let rp = piece_row_profile(piece);
            let row_pls = rp.placements(board_h);
            row_shifted.push(row_pls.iter().map(|&(_, shifted)| shifted).collect());
            row_profiles.push(rp);

            let cp = piece_col_profile(piece);
            let col_pls = cp.placements(board_w);
            col_shifted.push(col_pls.iter().map(|&(_, shifted)| shifted).collect());
            col_profiles.push(cp);
        }

        Self {
            row_board,
            col_board,
            row_profiles,
            col_profiles,
            row_shifted,
            col_shifted,
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
    ///
    /// # Panics
    /// Panics if `row` is out of range for this piece.
    #[inline(always)]
    pub fn row_shifted_at(&self, piece_idx: usize, row: usize) -> u16x16 {
        self.row_shifted[piece_idx][row]
    }

    /// Shifted col profile for piece `piece_idx` placed at column `col`.
    ///
    /// # Panics
    /// Panics if `col` is out of range for this piece.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_basic() {
        // 3x3, M=2, all zeros. Two 1x1 pieces.
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p, p];
        let order = vec![0, 1];

        let data = SubgameData::build(&board, &pieces, &order);

        assert_eq!(data.num_pieces(), 2);
        assert!(data.row_board().is_solved());
        assert!(data.col_board().is_solved());

        // 1x1 piece has row profile [1], fits in 3 row positions
        assert_eq!(data.row_profile(0).len(), 1);
        assert_eq!(data.num_row_positions(0), 3);
        assert_eq!(data.num_col_positions(0), 3);
    }

    #[test]
    fn test_build_nontrivial() {
        // 3x3, M=3 board with deficits
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 0, 1], &[1, 2, 0]];
        let board = Board::from_grid(grid, 3);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let pieces = vec![bar, bar, bar];
        let order = vec![0, 1, 2];

        let data = SubgameData::build(&board, &pieces, &order);

        // Row deficits: [3, 3, 3]
        assert_eq!(data.row_board().get(0), 3);
        assert_eq!(data.row_board().get(1), 3);
        assert_eq!(data.row_board().get(2), 3);

        // 1x3 bar has row profile [3] (1 row, 3 cells)
        assert_eq!(data.row_profile(0).len(), 1);
        assert_eq!(data.row_profile(0).get(0), 3);
        // Fits in 3 row positions on a 3-row board
        assert_eq!(data.num_row_positions(0), 3);

        // Col profile: [1, 1, 1] (3 cols)
        assert_eq!(data.col_profile(0).len(), 3);
        // Fits in 1 col position on a 3-col board
        assert_eq!(data.num_col_positions(0), 1);
    }

    #[test]
    fn test_shifted_profiles() {
        let board = Board::new_solved(4, 4, 2);
        // Vertical domino
        let piece = Piece::from_grid(&[&[true], &[true]]);
        let pieces = vec![piece];
        let order = vec![0];

        let data = SubgameData::build(&board, &pieces, &order);

        // Row profile [1, 1], 3 valid positions on 4-row board
        assert_eq!(data.num_row_positions(0), 3);

        // Position 0: lanes [0]=1, [1]=1, rest=0
        let s0 = data.row_shifted_at(0, 0);
        let arr0 = s0.to_array();
        assert_eq!(arr0[0], 1);
        assert_eq!(arr0[1], 1);
        assert_eq!(arr0[2], 0);

        // Position 2: lanes [2]=1, [3]=1, rest=0
        let s2 = data.row_shifted_at(0, 2);
        let arr2 = s2.to_array();
        assert_eq!(arr2[0], 0);
        assert_eq!(arr2[1], 0);
        assert_eq!(arr2[2], 1);
        assert_eq!(arr2[3], 1);
    }

    #[test]
    fn test_solver_order_respected() {
        let board = Board::new_solved(4, 4, 2);
        let bar_h = Piece::from_grid(&[&[true, true, true]]); // 1x3
        let bar_v = Piece::from_grid(&[&[true], &[true], &[true]]); // 3x1
        let pieces = vec![bar_h, bar_v];

        // Solver reverses the order
        let order = vec![1, 0];
        let data = SubgameData::build(&board, &pieces, &order);

        // Solver index 0 = original index 1 = bar_v (3x1)
        assert_eq!(data.row_profile(0).len(), 3); // 3 rows
        assert_eq!(data.col_profile(0).len(), 1); // 1 col

        // Solver index 1 = original index 0 = bar_h (1x3)
        assert_eq!(data.row_profile(1).len(), 1); // 1 row
        assert_eq!(data.col_profile(1).len(), 3); // 3 cols
    }
}
