//! Global total-deficit pruning.
//!
//! The total deficit is the sum of all per-cell deficits on the board.
//! The remaining piece cells (sum of cell_count for pieces not yet placed)
//! must be >= total_deficit, otherwise there aren't enough hits to solve.
//!
//! This is the single most important pruning check (>10x node increase without it).

use crate::core::board::Board;
use crate::core::piece::Piece;

/// Precomputed data for total-deficit pruning.
pub(crate) struct TotalDeficitPrune {
    /// remaining_bits[i] = total cell count of pieces [i..n].
    /// remaining_bits[n] = 0.
    remaining_bits: Vec<u32>,
}

impl TotalDeficitPrune {
    /// Build from pieces in solver order.
    pub fn precompute(pieces: &[Piece], order: &[usize]) -> Self {
        let n = pieces.len();
        let mut remaining_bits = vec![0u32; n + 1];
        for i in (0..n).rev() {
            remaining_bits[i] = remaining_bits[i + 1] + pieces[order[i]].cell_count();
        }
        Self { remaining_bits }
    }

    /// Total remaining piece cells from piece_idx onward.
    #[inline(always)]
    pub fn remaining_bits(&self, piece_idx: usize) -> u32 {
        self.remaining_bits[piece_idx]
    }

    /// Returns false (prune) if remaining piece cells < board's total deficit.
    #[inline(always)]
    pub fn try_prune(&self, board: &Board, piece_idx: usize) -> bool {
        self.remaining_bits[piece_idx] >= board.total_deficit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    #[test]
    fn test_precompute_basic() {
        // Two pieces: 1x1 (1 cell) and 1x2 (2 cells).
        let p1 = Piece::from_grid(&[&[true]]);
        let p2 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1, p2];
        let order = vec![0, 1];

        let td = TotalDeficitPrune::precompute(&pieces, &order);

        assert_eq!(td.remaining_bits(0), 3); // 1 + 2
        assert_eq!(td.remaining_bits(1), 2); // just p2
        assert_eq!(td.remaining_bits(2), 0); // none left
    }

    #[test]
    fn test_precompute_reordered() {
        let p1 = Piece::from_grid(&[&[true]]);        // 1 cell
        let p2 = Piece::from_grid(&[&[true, true]]);   // 2 cells
        let p3 = Piece::from_grid(&[&[true, true, true]]); // 3 cells
        let pieces = vec![p1, p2, p3];
        let order = vec![2, 0, 1]; // p3 first, then p1, then p2

        let td = TotalDeficitPrune::precompute(&pieces, &order);

        assert_eq!(td.remaining_bits(0), 6); // 3 + 1 + 2
        assert_eq!(td.remaining_bits(1), 3); // 1 + 2
        assert_eq!(td.remaining_bits(2), 2); // just p2
    }

    #[test]
    fn test_try_prune_feasible() {
        // 3x3 board, deficit = 3 (three cells at 1), pieces have 4 cells total.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 3);

        let p1 = Piece::from_grid(&[&[true, true]]);
        let p2 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1, p2];
        let order = vec![0, 1];
        let td = TotalDeficitPrune::precompute(&pieces, &order);

        // 4 >= 3 → feasible
        assert!(td.try_prune(&board, 0));
    }

    #[test]
    fn test_try_prune_infeasible() {
        // 3x3 board, deficit = 5, but only 2 piece cells.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 5);

        let p1 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1];
        let order = vec![0];
        let td = TotalDeficitPrune::precompute(&pieces, &order);

        // 2 < 5 → prune
        assert!(!td.try_prune(&board, 0));
    }

    #[test]
    fn test_try_prune_exact_match() {
        // 3x3 board, deficit = 2, exactly 2 piece cells.
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 2);

        let p1 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1];
        let order = vec![0];
        let td = TotalDeficitPrune::precompute(&pieces, &order);

        // 2 >= 2 → feasible
        assert!(td.try_prune(&board, 0));
    }
}
