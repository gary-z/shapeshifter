//! Subgame pruning via placement filtering.
//!
//! For the current piece, tries each 1D placement on both the row and column
//! subgames. Returns bitmasks of valid row/col positions. The 2D backtracker
//! skips placements at invalid (row, col) combinations.
//!
//! Sound with wrapping — the subgame wrapping model mirrors the full game's
//! modular arithmetic (see subgame/DESIGN.md proof).

use crate::core::board::Board;
use crate::subgame::data::SubgameData;
use crate::subgame::state::SubgameState;

/// Subgame pruning data.
pub(crate) struct SubgamePrune {
    pub(crate) data: SubgameData,
    pub(crate) nodes: std::sync::atomic::AtomicU64,
}

impl SubgamePrune {
    /// Build from board, pieces, and solver order.
    pub fn precompute(board: &Board, pieces: &[crate::core::piece::Piece], order: &[usize]) -> Self {
        Self {
            data: SubgameData::build(board, pieces, order),
            nodes: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get valid row and column positions for the piece at `piece_idx`.
    /// Uses precomputed checks (no backtracking, no budget).
    /// Returns (valid_rows, valid_cols) bitmasks.
    #[inline(always)]
    pub fn valid_positions(
        &self,
        sg_state: &SubgameState,
        piece_idx: usize,
    ) -> (u16, u16) {
        let valid_rows = self.data.row_prune.valid_positions(
            sg_state.row_board(), piece_idx, &self.data.row_placements,
        );
        if valid_rows == 0 {
            return (0, 0); // no valid rows → skip column check entirely
        }
        let valid_cols = self.data.col_prune.valid_positions(
            sg_state.col_board(), piece_idx, &self.data.col_placements,
        );
        if valid_cols == 0 {
            return (0, 0);
        }
        (valid_rows, valid_cols)
    }

    /// Total subgame nodes visited across all checks.
    pub fn total_nodes(&self) -> u64 {
        self.nodes.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    #[test]
    fn test_valid_positions_feasible() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces: Vec<Piece> = (0..9).map(|_| p.clone()).collect();
        let order: Vec<usize> = (0..9).collect();
        let sp = SubgamePrune::precompute(&board, &pieces, &order);
        let sg = SubgameState::new(&sp.data);
        let (vr, vc) = sp.valid_positions(&sg, 0);
        // All positions should be valid for 1x1 piece on uniform board
        assert!(vr != 0);
        assert!(vc != 0);
    }
}
