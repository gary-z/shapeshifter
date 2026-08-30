//! Per-cell modular interval pruning.
//!
//! For every solver depth and board cell, precompute the minimum and maximum
//! number of times the remaining pieces can cover that cell. A cell whose
//! current deficit is `d` needs `d + k*M` further hits. If no such value lies
//! in the precomputed interval, the branch cannot be completed.

use crate::core::bitboard::Bitboard;
use crate::core::board::{Board, MAX_M};

const NUM_BITS: usize = 225;

pub(crate) struct CellIntervalPrune {
    /// At each depth, one mask per current cell value. A set bit means that
    /// value has no congruent remaining-hit count in the cell's interval.
    infeasible: Vec<[Bitboard; MAX_M]>,
}

impl CellIntervalPrune {
    pub fn precompute(
        all_placements: &[Vec<(usize, usize, Bitboard)>],
        m: u8,
        valid_mask: Bitboard,
    ) -> Self {
        let n = all_placements.len();
        let mut possible = Vec::with_capacity(n);
        let mut mandatory = Vec::with_capacity(n);

        for placements in all_placements {
            let mut any = Bitboard::ZERO;
            let mut every = if placements.is_empty() {
                Bitboard::ZERO
            } else {
                valid_mask
            };
            for &(_, _, mask) in placements {
                any |= mask;
                every &= mask;
            }
            possible.push(any);
            mandatory.push(every);
        }

        let mut suffix_min = vec![[0u8; NUM_BITS]; n + 1];
        let mut suffix_max = vec![[0u8; NUM_BITS]; n + 1];
        for depth in (0..n).rev() {
            suffix_min[depth] = suffix_min[depth + 1];
            suffix_max[depth] = suffix_max[depth + 1];
            let mut cells = valid_mask;
            while !cells.is_zero() {
                let bit = cells.lowest_set_bit() as usize;
                suffix_min[depth][bit] += mandatory[depth].get_bit(bit as u32) as u8;
                suffix_max[depth][bit] += possible[depth].get_bit(bit as u32) as u8;
                cells.clear_bit(bit as u32);
            }
        }

        let mut infeasible = vec![[Bitboard::ZERO; MAX_M]; n + 1];
        for depth in 0..=n {
            let mut cells = valid_mask;
            while !cells.is_zero() {
                let bit = cells.lowest_set_bit() as usize;
                let min_hits = suffix_min[depth][bit];
                let max_hits = suffix_max[depth][bit];
                for value in 0..m {
                    let offset = (value + m - min_hits % m) % m;
                    if min_hits + offset > max_hits {
                        infeasible[depth][value as usize].set_bit(bit as u32);
                    }
                }
                cells.clear_bit(bit as u32);
            }
        }

        Self { infeasible }
    }

    #[inline(always)]
    pub fn try_prune<const M: usize>(&self, board: &Board, depth: usize) -> bool {
        let masks = &self.infeasible[depth];
        let mut value = 0;
        while value < M {
            if !(board.plane(value as u8) & masks[value]).is_zero() {
                return false;
            }
            value += 1;
        }
        true
    }

    /// Derive cells that the current piece must cover or must avoid so the
    /// child state passes the interval test at `next_depth`.
    #[inline(always)]
    pub fn placement_constraints<const M: usize>(
        &self,
        board: &Board,
        next_depth: usize,
    ) -> (Bitboard, Bitboard) {
        let masks = &self.infeasible[next_depth];
        let mut required = Bitboard::ZERO;
        let mut forbidden = Bitboard::ZERO;
        let mut value = 0;
        while value < M {
            let cells = board.plane(value as u8);
            required |= cells & masks[value];
            forbidden |= cells & masks[(value + M - 1) % M];
            value += 1;
        }
        (required, forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::Piece;

    #[test]
    fn rejects_fixed_hit_with_wrong_residue() {
        let piece = Piece::from_grid(&[
            &[true, true, true],
            &[true, true, true],
            &[true, true, true],
        ]);
        let placements = vec![piece.placements(3, 3)];
        let board = Board::new_solved(3, 3, 3);
        let prune = CellIntervalPrune::precompute(&placements, 3, board.valid_mask());
        assert!(!prune.try_prune::<3>(&board, 0));
    }

    #[test]
    fn accepts_fixed_hit_with_matching_residue() {
        let piece = Piece::from_grid(&[
            &[true, true, true],
            &[true, true, true],
            &[true, true, true],
        ]);
        let placements = vec![piece.placements(3, 3)];
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 3);
        let prune = CellIntervalPrune::precompute(&placements, 3, board.valid_mask());
        assert!(prune.try_prune::<3>(&board, 0));
    }

    #[test]
    fn rejects_deficit_above_possible_coverage() {
        let piece = Piece::from_grid(&[&[true]]);
        let placements = vec![piece.placements(3, 3)];
        let grid: &[&[u8]] = &[&[2, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let prune = CellIntervalPrune::precompute(&placements, 3, board.valid_mask());
        assert!(!prune.try_prune::<3>(&board, 0));
    }

    #[test]
    fn forces_coverage_needed_by_the_suffix() {
        let piece = Piece::from_grid(&[&[true]]);
        let placements = vec![piece.placements(3, 3), piece.placements(3, 3)];
        let grid: &[&[u8]] = &[&[2, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let prune = CellIntervalPrune::precompute(&placements, 3, board.valid_mask());
        let (required, forbidden) = prune.placement_constraints::<3>(&board, 1);
        assert!(required.get_bit(0));
        assert!(!forbidden.get_bit(0));
    }
}
