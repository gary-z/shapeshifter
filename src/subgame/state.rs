use std::simd::u16x16;

use super::board::SubgameBoard;
use super::data::SubgameData;

/// Runtime subgame state maintained alongside the main 2D board during search.
///
/// Holds the current row and column subgame boards, updated incrementally as
/// pieces are placed/undone in the main solver. At ~68 bytes this struct is
/// `Copy`, making it trivial to use with the copy-make backtracking pattern:
/// snapshot before a placement, discard on backtrack.
///
/// # Usage (future integration)
///
/// ```ignore
/// // In backtrack, before trying placements:
/// let subgame_snapshot = subgame_state;
///
/// // For each candidate placement (row, col) of piece_idx:
/// let mut sg = subgame_snapshot;
/// if !sg.apply_piece(subgame_data, piece_idx, row, col) {
///     continue; // subgame infeasible → prune
/// }
/// // Optionally run subgame solver on sg for deeper pruning...
/// ```
#[derive(Clone, Copy)]
pub struct SubgameState {
    row_board: SubgameBoard,
    col_board: SubgameBoard,
}

impl SubgameState {
    /// Create the initial subgame state from precomputed data.
    pub fn new(data: &SubgameData) -> Self {
        Self {
            row_board: *data.row_board(),
            col_board: *data.col_board(),
        }
    }

    /// Current row subgame board.
    #[inline(always)]
    pub fn row_board(&self) -> &SubgameBoard {
        &self.row_board
    }

    /// Current column subgame board.
    #[inline(always)]
    pub fn col_board(&self) -> &SubgameBoard {
        &self.col_board
    }

    /// Apply a piece placement to both subgame boards.
    ///
    /// `piece_idx` is the solver-order piece index. `row` and `col` are the
    /// placement coordinates from the main solver.
    ///
    /// Returns `true` if both subgame boards accepted the placement (no
    /// underflow). Returns `false` if either subgame detects underflow,
    /// meaning the placement is infeasible — the state is left in an
    /// indeterminate state and should be discarded (consistent with
    /// copy-make usage).
    #[inline(always)]
    pub fn apply_piece(
        &mut self,
        data: &SubgameData,
        piece_idx: usize,
        row: usize,
        col: usize,
    ) -> bool {
        let row_shifted = data.row_shifted_at(piece_idx, row);
        if !self.row_board.apply_piece(row_shifted) {
            return false;
        }

        let col_shifted = data.col_shifted_at(piece_idx, col);
        if !self.col_board.apply_piece(col_shifted) {
            // Row board was modified but col failed — state is indeterminate.
            // With copy-make, caller discards this copy anyway.
            return false;
        }

        true
    }

    /// Apply a piece unconditionally (allowing underflow wrapping).
    /// Keeps state consistent when wrapping occurs in the full game.
    #[inline(always)]
    pub fn apply_piece_wrapping(
        &mut self,
        data: &SubgameData,
        piece_idx: usize,
        row: usize,
        col: usize,
    ) {
        let row_shifted = data.row_shifted_at(piece_idx, row);
        self.row_board.apply_piece_wrapping(row_shifted);
        let col_shifted = data.col_shifted_at(piece_idx, col);
        self.col_board.apply_piece_wrapping(col_shifted);
    }

    /// Undo a piece placement on both subgame boards.
    ///
    /// Only needed if NOT using the copy-make pattern.
    #[inline(always)]
    pub fn undo_piece(
        &mut self,
        data: &SubgameData,
        piece_idx: usize,
        row: usize,
        col: usize,
    ) {
        let row_shifted = data.row_shifted_at(piece_idx, row);
        self.row_board.undo_piece(row_shifted);

        let col_shifted = data.col_shifted_at(piece_idx, col);
        self.col_board.undo_piece(col_shifted);
    }

    /// Apply a piece using raw shifted profiles (avoids re-lookup when caller
    /// already has them).
    #[inline(always)]
    pub fn apply_piece_raw(
        &mut self,
        row_shifted: u16x16,
        col_shifted: u16x16,
    ) -> bool {
        if !self.row_board.apply_piece(row_shifted) {
            return false;
        }
        if !self.col_board.apply_piece(col_shifted) {
            return false;
        }
        true
    }

    /// Undo a piece using raw shifted profiles.
    #[inline(always)]
    pub fn undo_piece_raw(
        &mut self,
        row_shifted: u16x16,
        col_shifted: u16x16,
    ) {
        self.row_board.undo_piece(row_shifted);
        self.col_board.undo_piece(col_shifted);
    }

    /// True if both subgame boards are solved (all cells at zero).
    #[inline(always)]
    pub fn is_solved(&self) -> bool {
        self.row_board.is_solved() && self.col_board.is_solved()
    }

    /// Total remaining deficit in the row subgame.
    #[inline(always)]
    pub fn row_deficit(&self) -> u32 {
        self.row_board.total_deficit()
    }

    /// Total remaining deficit in the column subgame.
    #[inline(always)]
    pub fn col_deficit(&self) -> u32 {
        self.col_board.total_deficit()
    }
}

impl std::fmt::Debug for SubgameState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SubgameState(row={:?}, col={:?})",
            self.row_board, self.col_board
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    fn make_test_data() -> (Board, Vec<Piece>, Vec<usize>, SubgameData) {
        // 3x3, M=2, board with deficits:
        // 0 1 → deficits: 0 1   row deficits: [1, 1, 0]
        // 1 0              1 0   col deficits: [1, 1]
        // But let's use a 3x3 for more room.
        //
        // Board (M=2):
        //   1 0 1     deficits: 1 0 1
        //   0 1 0     deficits: 0 1 0
        //   1 0 1     deficits: 1 0 1
        // Row deficits: [2, 1, 2]
        // Col deficits: [2, 1, 2]
        let grid: &[&[u8]] = &[&[1, 0, 1], &[0, 1, 0], &[1, 0, 1]];
        let board = Board::from_grid(grid, 2);

        // Two pieces: a 1x1 and a horizontal domino (1x2)
        let p1 = Piece::from_grid(&[&[true]]);
        let p2 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1, p2];
        let order = vec![0, 1]; // solver order = original order

        let data = SubgameData::build(&board, &pieces, &order);
        (board, pieces, order, data)
    }

    #[test]
    fn test_new_state() {
        let (_, _, _, data) = make_test_data();
        let state = SubgameState::new(&data);

        assert_eq!(state.row_deficit(), 5); // 2+1+2
        assert_eq!(state.col_deficit(), 5); // 2+1+2
        assert!(!state.is_solved());
    }

    #[test]
    fn test_apply_piece() {
        let (_, _, _, data) = make_test_data();
        let mut state = SubgameState::new(&data);

        // Place piece 0 (1x1) at (0, 0)
        // Row: subtract [1] shifted to pos 0 → row deficit [2-1, 1, 2] = [1, 1, 2]
        // Col: subtract [1] shifted to pos 0 → col deficit [2-1, 1, 2] = [1, 1, 2]
        assert!(state.apply_piece(&data, 0, 0, 0));
        assert_eq!(state.row_deficit(), 4);
        assert_eq!(state.col_deficit(), 4);
    }

    #[test]
    fn test_apply_and_undo() {
        let (_, _, _, data) = make_test_data();
        let mut state = SubgameState::new(&data);
        let original_row_deficit = state.row_deficit();
        let original_col_deficit = state.col_deficit();

        assert!(state.apply_piece(&data, 0, 0, 0));
        state.undo_piece(&data, 0, 0, 0);

        assert_eq!(state.row_deficit(), original_row_deficit);
        assert_eq!(state.col_deficit(), original_col_deficit);
    }

    #[test]
    fn test_copy_make_pattern() {
        let (_, _, _, data) = make_test_data();
        let state = SubgameState::new(&data);

        // Snapshot, then modify the copy
        let snapshot = state;
        let mut branch = snapshot;
        assert!(branch.apply_piece(&data, 0, 0, 0));

        // Original snapshot is unchanged
        assert_eq!(snapshot.row_deficit(), state.row_deficit());
        assert_ne!(branch.row_deficit(), snapshot.row_deficit());
    }

    #[test]
    fn test_apply_underflow_rejected() {
        // 3x3 board, M=2. Top row all zeros (deficit 0), bottom rows have deficits.
        // Board values:
        //   0 0 0   → deficits: 0 0 0
        //   0 0 0   → deficits: 0 0 0
        //   1 1 1   → deficits: 1 1 1
        // Row deficits: [0, 0, 3], Col deficits: [1, 1, 1]
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        // Piece: vertical bar covering 3 rows → row profile [1, 1, 1]
        let piece = Piece::from_grid(&[&[true], &[true], &[true]]);
        let pieces = vec![piece];
        let order = vec![0];
        let data = SubgameData::build(&board, &pieces, &order);

        let mut state = SubgameState::new(&data);
        // Place at row 0: row profile [1, 1, 1] at pos 0 → subtract from [0, 0, 3]
        // Row 0 has deficit 0, subtracting 1 → underflow
        assert!(!state.apply_piece(&data, 0, 0, 0));
    }

    #[test]
    fn test_solve_via_subgame() {
        // 3x3, M=2, board values all 1 → deficits all 1.
        // Row deficits: [3, 3, 3], Col deficits: [3, 3, 3]
        // Three 1x3 horizontal bars (each covers one row fully).
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let pieces = vec![bar, bar, bar];
        let order = vec![0, 1, 2];
        let data = SubgameData::build(&board, &pieces, &order);

        let mut state = SubgameState::new(&data);
        assert!(!state.is_solved());

        // Place bar 0 at (0, 0): row deficit [3-3, 3, 3] = [0, 3, 3]
        assert!(state.apply_piece(&data, 0, 0, 0));
        assert!(!state.is_solved());

        // Place bar 1 at (1, 0): row deficit [0, 3-3, 3] = [0, 0, 3]
        assert!(state.apply_piece(&data, 1, 1, 0));
        assert!(!state.is_solved());

        // Place bar 2 at (2, 0): row deficit [0, 0, 0], col deficit [0, 0, 0]
        assert!(state.apply_piece(&data, 2, 2, 0));
        assert!(state.is_solved());
    }

    #[test]
    fn test_size_is_small() {
        // SubgameState should be lightweight for copy-make
        assert!(std::mem::size_of::<SubgameState>() <= 128);
    }
}
