//! Subgame pruning.
//!
//! Projects the 2D board into 1D row and column subgames. If either
//! subgame is infeasible, the full game is provably unsolvable.
//! Only sound when remaining_piece_cells == total_deficit (no wrapping).

use crate::core::board::Board;
use crate::subgame::data::SubgameData;

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

    /// Check subgame feasibility using incrementally tracked 1D boards.
    /// Only sound when remaining_bits == total_deficit (no wrapping).
    #[inline(always)]
    pub fn try_prune(
        &self,
        sg_state: &crate::subgame::state::SubgameState,
        piece_idx: usize,
        remaining_bits: u32,
        total_deficit: u32,
    ) -> bool {
        if remaining_bits != total_deficit {
            return true; // wrapping possible — can't prune
        }
        let (feasible, sg_nodes) = self.data.check_feasible(
            *sg_state.row_board(), *sg_state.col_board(), piece_idx,
        );
        self.nodes.fetch_add(sg_nodes, std::sync::atomic::Ordering::Relaxed);
        feasible
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
    use crate::subgame::state::SubgameState;

    #[test]
    fn test_try_prune_solved_board() {
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let sp = SubgamePrune::precompute(&board, &pieces, &order);
        let sg = SubgameState::new(&sp.data);
        // Solved board, 1 remaining piece cell, deficit=0, remaining_bits=1 != 0
        // → wrapping guard fires, returns true (skip check)
        assert!(sp.try_prune(&sg, 0, 1, board.total_deficit()));
    }

    #[test]
    fn test_try_prune_no_wrapping() {
        // 3x3 M=2, all cells at deficit 1 → total_deficit=9
        // 9 pieces of 1 cell each → remaining_bits=9=total_deficit, no wrapping
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces: Vec<Piece> = (0..9).map(|_| p.clone()).collect();
        let order: Vec<usize> = (0..9).collect();
        let sp = SubgamePrune::precompute(&board, &pieces, &order);
        let sg = SubgameState::new(&sp.data);

        // Should be feasible — place one piece per cell
        assert!(sp.try_prune(&sg, 0, 9, board.total_deficit()));
    }
}
