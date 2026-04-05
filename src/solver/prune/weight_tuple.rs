//! Weight-tuple reachability pruning.
//!
//! Tracks weight-tuples for groups of disjoint cell sets (e.g., 3 adjacent
//! rows). A suffix DP checks which weight-tuples are achievable.

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

/// Weight-tuple reachability for a set of disjoint cell groups.
pub(crate) struct WeightTupleReachability {
    pub(crate) group_masks: Vec<Bitboard>,
    pub(crate) num_groups: usize,
    pub(crate) strides: Vec<usize>,
    pub(crate) num_configs: usize,
    pub(crate) m: u8,
    pub(crate) reachable: Vec<u8>,
}

impl WeightTupleReachability {
    #[inline]
    pub(crate) fn encode(&self, weights: &[u32]) -> usize {
        let mut idx = 0;
        for g in 0..self.num_groups {
            idx += weights[g] as usize * self.strides[g];
        }
        idx
    }

    #[inline]
    pub(crate) fn group_weight(&self, board: &Board, group_idx: usize) -> u32 {
        let mask = self.group_masks[group_idx];
        let mut w = 0u32;
        for d in 1..self.m {
            w += d as u32 * (board.plane(d) & mask).count_ones();
        }
        w
    }

    #[inline]
    pub(crate) fn check(&self, board: &Board, piece_idx: usize) -> bool {
        let mut weights = [0u32; 8];
        for g in 0..self.num_groups {
            weights[g] = self.group_weight(board, g);
        }
        let idx = self.encode(&weights);
        self.reachable[piece_idx * self.num_configs + idx] != 0
    }
}

/// All weight-tuple reachability checks.
pub(crate) struct WeightTuplePrune {
    pub(crate) checks: Vec<WeightTupleReachability>,
}

impl WeightTuplePrune {
    /// Check all weight-tuple groups. Returns false to prune.
    #[inline(always)]
    pub fn try_prune(&self, board: &Board, piece_idx: usize) -> bool {
        for wt in &self.checks {
            if !wt.check(board, piece_idx) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;

    #[test]
    fn test_try_prune_empty_checks() {
        let wtp = WeightTuplePrune { checks: Vec::new() };
        let board = Board::new_solved(3, 3, 2);
        assert!(wtp.try_prune(&board, 0));
    }
}
