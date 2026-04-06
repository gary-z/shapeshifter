//! Regional budget pruning.
//!
//! For small rectangular regions of the board, computes:
//! - min_contribution: minimum piece-cells that MUST land in the region
//!   (pieces too large to avoid it)
//! - max_contribution: maximum piece-cells that CAN land in the region
//!   (suffix sum across remaining pieces)
//!
//! The region's deficit + forced overhead must be coverable by remaining
//! pieces' max contribution. This is tighter than global total-deficit
//! because it accounts for piece geometry forcing overlap with specific regions.

use crate::core::board::Board;
use crate::core::piece::Piece;

/// A single rectangular region check.
struct RegionCheck {
    /// Top-left corner (row, col).
    r0: usize,
    c0: usize,
    /// Size (rows, cols).
    rh: usize,
    cw: usize,
    /// Forced overhead for this region: ceil((min_total - deficit) / M) * M.
    /// Only nonzero if min_total > deficit.
    forced_overhead: u32,
    /// Per-piece min contribution to this region (minimum across all placements).
    piece_min: Vec<u32>,
    /// Per-piece max contribution to this region (maximum across all placements).
    piece_max: Vec<u32>,
    /// Suffix sum of piece_max: suffix_max_budget[i] = sum of piece_max[i..n].
    suffix_max_budget: Vec<u32>,
    /// Suffix sum of piece_min: suffix_min_budget[i] = sum of piece_min[i..n].
    suffix_min_budget: Vec<u32>,
    /// Bitboard mask for the region (for fast deficit computation).
    mask: crate::core::bitboard::Bitboard,
}

/// Precomputed regional budget checks.
pub(crate) struct RegionBudgetPrune {
    checks: Vec<RegionCheck>,
    m: u8,
}

impl RegionBudgetPrune {
    pub fn precompute(
        board: &Board,
        pieces: &[Piece],
        order: &[usize],
        all_placements: &[Vec<(usize, usize, crate::core::bitboard::Bitboard)>],
        h: u8,
        w: u8,
        m: u8,
    ) -> Self {
        let bh = h as usize;
        let bw = w as usize;
        let n = pieces.len();
        let m_val = m as u32;

        let mut checks = Vec::new();

        // Scan rectangular regions from 3x3 to 5x5.
        for sz in 3..=5usize.min(bh).min(bw) {
            for r0 in 0..=bh - sz {
                for c0 in 0..=bw - sz {
                    let mut mask = crate::core::bitboard::Bitboard::ZERO;
                    let mut region_deficit = 0u32;
                    for r in r0..r0 + sz {
                        for c in c0..c0 + sz {
                            mask.set_bit((r * 15 + c) as u32);
                            region_deficit += board.get(r, c) as u32;
                        }
                    }

                    let mut piece_min = Vec::with_capacity(n);
                    let mut piece_max = Vec::with_capacity(n);

                    for i in 0..n {
                        let piece = &pieces[order[i]];
                        let ph = piece.height() as usize;
                        let pw = piece.width() as usize;
                        let mut lo = u32::MAX;
                        let mut hi = 0u32;

                        for &(pr, pc, _) in &all_placements[i] {
                            let mut count = 0u32;
                            for dr in 0..ph {
                                for dc in 0..pw {
                                    if piece.shape().get_bit((dr * 15 + dc) as u32) {
                                        let r = pr + dr;
                                        let c = pc + dc;
                                        if r >= r0 && r < r0 + sz && c >= c0 && c < c0 + sz {
                                            count += 1;
                                        }
                                    }
                                }
                            }
                            lo = lo.min(count);
                            hi = hi.max(count);
                        }
                        if lo == u32::MAX { lo = 0; }
                        piece_min.push(lo);
                        piece_max.push(hi);
                    }

                    let min_total: u32 = piece_min.iter().sum();

                    // Only keep regions with forced overhead.
                    if min_total <= region_deficit {
                        continue;
                    }

                    let excess = min_total - region_deficit;
                    let forced_overhead = ((excess + m_val - 1) / m_val) * m_val;

                    // Build suffix sums.
                    let mut suffix_max = vec![0u32; n + 1];
                    let mut suffix_min = vec![0u32; n + 1];
                    for i in (0..n).rev() {
                        suffix_max[i] = suffix_max[i + 1] + piece_max[i];
                        suffix_min[i] = suffix_min[i + 1] + piece_min[i];
                    }

                    checks.push(RegionCheck {
                        r0, c0, rh: sz, cw: sz,
                        forced_overhead,
                        piece_min,
                        piece_max,
                        suffix_max_budget: suffix_max,
                        suffix_min_budget: suffix_min,
                        mask,
                    });
                }
            }
        }

        // Sort by forced_overhead descending (strongest checks first).
        checks.sort_by(|a, b| b.forced_overhead.cmp(&a.forced_overhead));
        // Keep at most 10 checks to bound per-node cost.
        checks.truncate(10);

        Self { checks, m }
    }

    /// Returns false (prune) if any region's budget is exceeded.
    #[inline(always)]
    pub fn try_prune(&self, board: &Board, piece_idx: usize) -> bool {
        let m = self.m;
        for check in &self.checks {
            // Compute current region deficit from the board.
            let mut region_deficit = 0u32;
            for d in 1..m {
                region_deficit += d as u32 * (board.plane(d) & check.mask).count_ones();
            }

            // Max contribution from remaining pieces.
            let max_budget = check.suffix_max_budget[piece_idx];

            // The region needs at least (region_deficit + forced_overhead_remaining).
            // forced_overhead_remaining = max(0, suffix_min[piece_idx] - region_deficit)
            // rounded up to multiple of M.
            let min_remaining = check.suffix_min_budget[piece_idx];
            let forced = if min_remaining > region_deficit {
                let excess = min_remaining - region_deficit;
                let m32 = m as u32;
                ((excess + m32 - 1) / m32) * m32
            } else {
                0
            };

            if max_budget < region_deficit + forced {
                return false;
            }
        }
        true
    }
}
