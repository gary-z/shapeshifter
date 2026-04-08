//! Parity partition pruning.
//!
//! Splits the board into two groups by a parity function (checkerboard,
//! even-row, even-col, mod-3, etc.). The total deficit contribution to
//! group 0 must be achievable by the remaining pieces' group-0 counts.
//! A suffix DP tracks which totals are reachable.

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::piece::Piece;

/// A single parity partition.
pub(crate) struct ParityPartition {
    pub(crate) mask: Bitboard,
    pub(crate) suffix_max: Vec<u32>,
    pub(crate) suffix_min: Vec<u32>,
    pub(crate) suffix_dp: Vec<Vec<bool>>,
}

/// All parity partitions for pruning.
pub(crate) struct ParityPrune {
    partitions: Vec<ParityPartition>,
}

impl ParityPrune {
    /// Build all parity partitions from pieces, order, and board dimensions.
    pub fn precompute(pieces: &[Piece], order: &[usize], h: u8, w: u8, _m: u8) -> Self {
        let bh = h as usize;
        let bw = w as usize;
        let n = pieces.len();

        let build_partition = |group_fn: &dyn Fn(usize, usize) -> bool,
                               num_offsets: usize,
                               offset_fn: &dyn Fn(usize, usize, usize) -> bool|
                               -> ParityPartition {
            let mut mask = Bitboard::ZERO;
            for r in 0..bh {
                for c in 0..bw {
                    if group_fn(r, c) {
                        mask.set_bit((r * 15 + c) as u32);
                    }
                }
            }

            let mut g0_counts: Vec<Vec<u32>> = Vec::with_capacity(n);
            for i in 0..n {
                let piece = &pieces[order[i]];
                let mut counts = vec![0u32; num_offsets];
                for off in 0..num_offsets {
                    for pr in 0..piece.height() as usize {
                        for pc in 0..piece.width() as usize {
                            if piece.shape().get_bit((pr * 15 + pc) as u32) && offset_fn(pr, pc, off) {
                                counts[off] += 1;
                            }
                        }
                    }
                }
                g0_counts.push(counts);
            }

            let mut suffix_max = vec![0u32; n + 1];
            let mut suffix_min = vec![0u32; n + 1];
            for i in (0..n).rev() {
                suffix_max[i] = suffix_max[i + 1] + *g0_counts[i].iter().max().unwrap();
                suffix_min[i] = suffix_min[i + 1] + *g0_counts[i].iter().min().unwrap();
            }

            let dp_size = suffix_max[0] as usize + 1;
            let mut suffix_dp = vec![vec![false; dp_size]; n + 1];
            suffix_dp[n][0] = true;
            for i in (0..n).rev() {
                for w in 0..dp_size {
                    if suffix_dp[i + 1][w] {
                        for &g0 in &g0_counts[i] {
                            let nw = w + g0 as usize;
                            if nw < dp_size { suffix_dp[i][nw] = true; }
                        }
                    }
                }
            }

            ParityPartition { mask, suffix_max, suffix_min, suffix_dp }
        };

        let mut partitions = Vec::new();

        // Mod-2 partitions.
        partitions.push(build_partition(&|r, c| (r + c) % 2 == 0, 2, &|pr, pc, off| (pr + pc + off) % 2 == 0));
        partitions.push(build_partition(&|r, _c| r % 2 == 0, 2, &|pr, _pc, off| (pr + off) % 2 == 0));
        partitions.push(build_partition(&|_r, c| c % 2 == 0, 2, &|_pr, pc, off| (pc + off) % 2 == 0));

        // Mod-3 row/col partitions.
        if bh >= 6 {
            for tg in 0..3usize {
                partitions.push(build_partition(&|r, _c| r % 3 == tg, 3, &|pr, _pc, off| (pr + off) % 3 == tg));
            }
        }
        if bw >= 6 {
            for tg in 0..3usize {
                partitions.push(build_partition(&|_r, c| c % 3 == tg, 3, &|_pr, pc, off| (pc + off) % 3 == tg));
            }
        }

        // Mod-3 diagonal partitions.
        if bh >= 4 && bw >= 4 {
            for tg in 0..3usize {
                partitions.push(build_partition(&|r, c| (r + c) % 3 == tg, 3, &|pr, pc, off| (pr + pc + off) % 3 == tg));
            }
        }
        if bh >= 4 && bw >= 4 {
            for tg in 0..3usize {
                partitions.push(build_partition(&|r, c| (r + 2 * c) % 3 == tg, 3, &|pr, pc, off| (pr + 2 * pc + off) % 3 == tg));
            }
        }

        Self { partitions }
    }

    /// Check all parity partitions. Returns false to prune.
    #[inline(always)]
    pub fn try_prune(&self, board: &Board, piece_idx: usize, m: u8, remaining_bits: u32) -> bool {
        let m32 = m as u32;
        let total_deficit = board.total_deficit();

        for partition in &self.partitions {
            let mut g0_deficit = 0u32;
            for d in 1..m {
                g0_deficit += d as u32 * (board.plane(d) & partition.mask).count_ones();
            }

            if partition.suffix_max[piece_idx] < g0_deficit {
                return false;
            }
            let g1_deficit = total_deficit - g0_deficit;
            let max_g1 = remaining_bits - partition.suffix_min[piece_idx];
            if max_g1 < g1_deficit {
                return false;
            }

            let dp = &partition.suffix_dp[piece_idx];
            let mut target = g0_deficit;
            let mut found = false;
            while (target as usize) < dp.len() {
                if dp[target as usize] {
                    found = true;
                    break;
                }
                target += m32;
            }
            if !found {
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
    use crate::core::piece::Piece;

    #[test]
    fn test_precompute_creates_partitions() {
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let pp = ParityPrune::precompute(&pieces, &order, 3, 3, 2);
        // 3 mod-2 partitions (checker, even_row, even_col), no mod-3 (board too small)
        assert_eq!(pp.partitions.len(), 3);
    }

    #[test]
    fn test_precompute_large_board_has_mod3() {
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let pp = ParityPrune::precompute(&pieces, &order, 6, 6, 2);
        // 3 mod-2 + 3 mod-3 row + 3 mod-3 col + 3 mod-3 diag + 3 mod-3 antidiag = 15
        assert!(pp.partitions.len() > 3);
    }

    #[test]
    fn test_try_prune_solved_board() {
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let pp = ParityPrune::precompute(&pieces, &order, 3, 3, 2);
        assert!(pp.try_prune(&board, 0, 2, 1));
    }
}
