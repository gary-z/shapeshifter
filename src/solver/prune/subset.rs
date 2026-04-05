//! Subset reachability pruning.
//!
//! For small subsets of board cells, a suffix DP tracks which cell
//! configurations are achievable by remaining pieces.

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

/// A small subset of board cells for local reachability pruning.
pub(crate) struct SubsetReachability {
    pub(crate) cells: Vec<u32>,
    pub(crate) m: u8,
    pub(crate) num_configs: usize,
    pub(crate) mask: Bitboard,
    pub(crate) reachable: Vec<u8>,
    pub(crate) first_useful: usize,
}

impl SubsetReachability {
    #[inline(always)]
    pub(crate) fn encode_config(&self, board: &Board) -> usize {
        let mut config = 0usize;
        let mut multiplier = 1usize;
        for &bit in &self.cells {
            let mut val = 0u8;
            for d in 1..self.m {
                if board.plane(d).get_bit(bit) {
                    val = d;
                    break;
                }
            }
            config += val as usize * multiplier;
            multiplier *= self.m as usize;
        }
        config
    }

    #[inline(always)]
    pub(crate) fn check(&self, board: &Board, piece_idx: usize) -> bool {
        if piece_idx < self.first_useful {
            return true;
        }
        if (board.plane(0) & self.mask) == self.mask {
            return true;
        }
        let config = self.encode_config(board);
        self.reachable[piece_idx * self.num_configs + config] != 0
    }
}

/// All subset reachability checks.
pub(crate) struct SubsetPrune {
    pub(crate) checks: Vec<SubsetReachability>,
}

impl SubsetPrune {
    /// Check all subsets. Returns false to prune.
    #[inline(always)]
    pub fn try_prune(&self, board: &Board, piece_idx: usize) -> bool {
        for subset in &self.checks {
            if !subset.check(board, piece_idx) {
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
        let sp = SubsetPrune { checks: Vec::new() };
        let board = Board::new_solved(3, 3, 2);
        assert!(sp.try_prune(&board, 0));
    }
}
