//! Per-cell hit-count pruning via Monte Carlo threshold.
//!
//! During precompute, Monte Carlo random piece placements establish an upper
//! bound N on per-cell hit counts: the maximum number of pieces covering any
//! single cell across many random solutions. During search, if any cell has
//! already been hit more than N times (with pieces still to place), prune.
//!
//! Hit counts are stored as binary-encoded bitboard planes (5 planes = counts
//! up to 31). Increment is a parallel ripple-carry add — ~10 bitboard ops.
//! The struct is Copy so we use copy-make (no undo needed).

use crate::core::bitboard::Bitboard;
use rand::{RngExt, SeedableRng};

/// Number of binary planes (5 bits → counts 0–31).
const NUM_PLANES: usize = 5;

/// Per-cell hit counter using binary-encoded bitboard planes.
/// Plane k holds bit k of each cell's count. Copy-make friendly.
#[derive(Clone, Copy)]
pub(crate) struct HitCounter {
    planes: [Bitboard; NUM_PLANES],
}

impl HitCounter {
    /// All counts zero.
    pub fn new() -> Self {
        Self { planes: [Bitboard::ZERO; NUM_PLANES] }
    }

    /// Increment hit count by 1 for every cell in `mask`.
    /// Parallel ripple-carry addition across planes.
    #[inline(always)]
    pub fn apply_piece(&mut self, mask: Bitboard) {
        let mut carry = mask;
        for plane in &mut self.planes {
            let new = *plane ^ carry;
            carry = *plane & carry;
            *plane = new;
            if carry.is_zero() { break; }
        }
    }

    /// Returns true if any cell's hit count >= threshold.
    #[inline(always)]
    pub fn any_cell_gte(&self, threshold: u8) -> bool {
        if threshold == 0 { return false; }

        // Fast path: if no cell has any bit set at or above the MSB of threshold,
        // then no cell can reach threshold. This skips the full comparison at
        // early depths when all counts are small.
        let msb = 7 - threshold.leading_zeros() as usize;
        let mut high_any = Bitboard::ZERO;
        for b in msb..NUM_PLANES {
            high_any = high_any | self.planes[b];
        }
        if high_any.is_zero() { return false; }

        // Full comparison via parallel subtraction: count - threshold.
        // borrow = 1 means count < threshold; borrow = 0 means count >= threshold.
        let mut borrow = Bitboard::ZERO;
        for bit in 0..NUM_PLANES {
            let p = self.planes[bit];
            if (threshold >> bit) & 1 == 1 {
                borrow = !p | borrow;
            } else {
                borrow = !p & borrow;
            }
        }
        // Non-board cells have count 0, so if threshold > 0 they have borrow = 1.
        !(!borrow).is_zero()
    }
}

/// Results of Monte Carlo precomputation.
pub(crate) struct McResults {
    /// Progressive hit-count thresholds [p50+1, p75+1, p90+1, p95+1, max+1].
    pub hit_count_thresholds: Vec<u8>,
    /// Max total deficit observed at each depth (after placing k pieces).
    /// Index k = after placing pieces 0..k-1. Length = n+1.
    pub max_deficit_at_depth: Vec<u32>,
}

/// Run Monte Carlo to find per-cell hit count distribution and per-depth
/// deficit bounds across random solutions.
pub(crate) fn precompute_mc(
    board: &crate::core::board::Board,
    all_placements: &[Vec<(usize, usize, Bitboard)>],
    m: u8,
) -> McResults {
    let n = all_placements.len();
    if n == 0 || all_placements.iter().any(|p| p.is_empty()) {
        return McResults {
            hit_count_thresholds: vec![],
            max_deficit_at_depth: vec![0; n + 1],
        };
    }

    // Precompute initial cell values for deficit tracking.
    let h = board.height() as usize;
    let w = board.width() as usize;
    let mut initial_value = [0u8; 225];
    for r in 0..h {
        for c in 0..w {
            initial_value[r * 15 + c] = board.get(r, c);
        }
    }

    let num_trials: usize = 10_000;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(0x5348_4150_4553_4849);
    let mut trial_maxes = Vec::with_capacity(num_trials);
    let mut max_deficit = vec![0u32; n + 1];
    let initial_deficit = board.total_deficit();
    let m32 = m as u32;

    for _ in 0..num_trials {
        let mut cell_hits = [0u8; 225];
        let mut deficit = initial_deficit;

        if deficit > max_deficit[0] { max_deficit[0] = deficit; }

        for (k, placements) in all_placements.iter().enumerate() {
            let idx = rng.random_range(0..placements.len());
            let mask = placements[idx].2;

            // For each cell in the mask: count zeros_hit and update cell_hits.
            let mut bits = mask;
            let mut zeros_hit = 0u32;
            while !bits.is_zero() {
                let bit = bits.lowest_set_bit() as usize;
                // Current cell value = (initial - hits_so_far) mod M.
                let current = (initial_value[bit] as u32 + m32 * 32 - cell_hits[bit] as u32) % m32;
                if current == 0 { zeros_hit += 1; }
                cell_hits[bit] = cell_hits[bit].saturating_add(1);
                bits.clear_bit(bit as u32);
            }

            // Deficit update: M * zeros_hit - piece_cells.
            deficit = deficit + m32 * zeros_hit - mask.count_ones();
            let depth = k + 1;
            if deficit > max_deficit[depth] { max_deficit[depth] = deficit; }
        }

        let trial_max = cell_hits.iter().copied().max().unwrap_or(0);
        trial_maxes.push(trial_max);
    }

    trial_maxes.sort_unstable();

    let percentiles = [50, 75, 90, 95];
    let mut thresholds: Vec<u8> = percentiles.iter().map(|&p| {
        let idx = (num_trials * p / 100).min(num_trials - 1);
        trial_maxes[idx].saturating_add(1).min(31)
    }).collect();

    let max_observed = *trial_maxes.last().unwrap();
    thresholds.push(max_observed.saturating_add(1).min(31));
    thresholds.dedup();

    McResults { hit_count_thresholds: thresholds, max_deficit_at_depth: max_deficit }
}
