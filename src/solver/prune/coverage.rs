//! Per-cell coverage pruning.
//!
//! For each cell c, the maximum number of hits it can receive from the
//! remaining pieces equals the number of remaining pieces that have at least
//! one placement covering c (since each piece places once, contributing 0 or 1
//! hits per cell). If any cell's deficit d exceeds this count, the cell cannot
//! reach zero — prune.
//!
//! Precompute: per depth i and per k in 1..M, a bitboard of cells that are
//! covered by at least k of the remaining pieces [i..n]. Runtime check:
//! `plane(d) & !at_least_k[i][d]` must be zero for every d in 1..M.
//!
//! Complementary to MC hit-count bounds (which are upper bounds from above);
//! this is a deterministic per-cell reachability lower bound.

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

pub(crate) struct CoveragePrune {
    /// Flat layout: `at_least_k[i * max_k + (k - 1)]` = bitboard of cells covered
    /// by at least k of the remaining pieces [i..n]. For `i = n` all values are zero.
    at_least_k: Vec<Bitboard>,
    max_k: usize,
}

impl CoveragePrune {
    /// Build from all placements (indexed by solver piece index) and board modulus.
    pub fn precompute(placements: &[Vec<(usize, usize, Bitboard)>], m: u8) -> Self {
        let n = placements.len();
        let max_k = (m - 1) as usize;
        let mut at_least_k = vec![Bitboard::ZERO; (n + 1) * max_k];

        for i in (0..n).rev() {
            let coverage = placements[i]
                .iter()
                .fold(Bitboard::ZERO, |acc, &(_, _, mask)| acc | mask);

            for k in 0..max_k {
                at_least_k[i * max_k + k] = at_least_k[(i + 1) * max_k + k];
            }

            for k in (1..=max_k).rev() {
                let prev = if k == 1 {
                    coverage
                } else {
                    at_least_k[i * max_k + (k - 2)]
                };
                let cur = at_least_k[i * max_k + (k - 1)];
                at_least_k[i * max_k + (k - 1)] = cur | (prev & coverage);
            }
        }

        Self { at_least_k, max_k }
    }

    /// Returns false (prune) if any cell's deficit exceeds its remaining coverage count.
    #[inline(always)]
    pub fn try_prune(&self, board: &Board, piece_idx: usize) -> bool {
        if self.max_k == 0 {
            return true;
        }
        let base = piece_idx * self.max_k;
        for k in 1..=self.max_k {
            let required = board.plane(k as u8);
            let available = self.at_least_k[base + k - 1];
            if !(required & !available).is_zero() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::Piece;

    #[test]
    fn test_solved_board_passes() {
        // Any pieces, solved board: all planes[d>0] empty → always passes.
        let p = Piece::from_grid(&[&[true]]);
        let placements = vec![p.placements(3, 3)];
        let cp = CoveragePrune::precompute(&placements, 2);
        let board = Board::new_solved(3, 3, 2);
        assert!(cp.try_prune(&board, 0));
    }

    #[test]
    fn test_unreachable_corner_pruned() {
        // 3x3 board, M=2. One 1x3 (horizontal) piece — can only cover row 0, 1, or 2.
        // No piece covers (0, 0) AND (2, 0) simultaneously, but each cell is covered
        // by the 1x3 piece. Place deficit at (2, 2) and give a 1x3 horizontal piece:
        // it can cover (2, 2) via rows 2 → feasible. Now use 1x1 piece that can cover
        // anywhere. So to test the prune, we need a cell NO piece can cover.
        // Construct: a 3x3 piece that only fits at (0,0). Deficit at (2,2) → coverage=1 ≥ 1 → feasible.
        // Instead: 3x3 board with M=3, cell (0,0) = 2 (deficit 2). One 1x1 piece → coverage=1 < 2 → prune.
        let grid: &[&[u8]] = &[&[2, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let p1 = Piece::from_grid(&[&[true]]);
        let placements = vec![p1.placements(3, 3)];
        let cp = CoveragePrune::precompute(&placements, 3);
        // Deficit 2 at (0,0), only 1 piece can hit it → coverage count = 1 < 2.
        assert!(!cp.try_prune(&board, 0));
    }

    #[test]
    fn test_reachable_cell_passes() {
        // 3x3 board, M=3, cell (0,0) = 2 (deficit 2). Two 1x1 pieces → coverage=2 >= 2.
        let grid: &[&[u8]] = &[&[2, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let p1 = Piece::from_grid(&[&[true]]);
        let placements = vec![p1.placements(3, 3), p1.placements(3, 3)];
        let cp = CoveragePrune::precompute(&placements, 3);
        assert!(cp.try_prune(&board, 0));
    }

    #[test]
    fn test_suffix_semantics() {
        // Two pieces: a 1x1 (covers all) and a 3x3 (covers only when placed at (0,0),
        // still covers all 9 cells). Place piece 0 as 3x3, piece 1 as 1x1.
        let p_full = Piece::from_grid(&[
            &[true, true, true],
            &[true, true, true],
            &[true, true, true],
        ]);
        let p1 = Piece::from_grid(&[&[true]]);
        let placements = vec![p_full.placements(3, 3), p1.placements(3, 3)];
        let cp = CoveragePrune::precompute(&placements, 3);

        // Deficit 2 at (0, 0): with both pieces remaining, coverage = 2 → pass.
        let grid: &[&[u8]] = &[&[2, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        assert!(cp.try_prune(&board, 0));

        // With only piece 1 remaining (the 1x1), coverage = 1 < 2 → prune.
        assert!(!cp.try_prune(&board, 1));
    }

    #[test]
    fn test_consistent_with_total_deficit() {
        // Configs where per-cell coverage should always permit feasible states.
        // Solved + empty pieces = no constraints.
        let placements: Vec<Vec<(usize, usize, Bitboard)>> = vec![];
        let cp = CoveragePrune::precompute(&placements, 2);
        let board = Board::new_solved(3, 3, 2);
        assert!(cp.try_prune(&board, 0));
    }
}
