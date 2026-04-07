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

/// One level of the progressive MC threshold pipeline.
/// Both hit-count and deficit bounds are computed jointly from the same
/// subset of MC trials, so the stated confidence is exact.
pub(crate) struct McLevel {
    /// Max cell hit count allowed (prune when any cell >= this).
    pub hit_count: u8,
    /// Max total deficit allowed at each depth k (index 0..=N).
    pub max_deficit_at_depth: Vec<u32>,
}

/// Run Monte Carlo to find joint hit-count and deficit bounds at
/// progressive confidence levels (p50, p75, p90, p95, ~100%).
///
/// For each percentile P: take the bottom P% of trials sorted by max_hits,
/// then compute both thresholds from that subset. This ensures P% of random
/// solutions satisfy BOTH constraints simultaneously.
pub(crate) fn precompute_mc(
    board: &crate::core::board::Board,
    all_placements: &[Vec<(usize, usize, Bitboard)>],
    m: u8,
) -> Vec<McLevel> {
    let n = all_placements.len();
    if n == 0 || all_placements.iter().any(|p| p.is_empty()) {
        return vec![McLevel { hit_count: 0, max_deficit_at_depth: vec![u32::MAX; n + 1] }];
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
    let initial_deficit = board.total_deficit();
    let m32 = m as u32;

    // Per-trial results: (max_cell_hits, deficit_trajectory).
    struct TrialResult {
        max_hits: u8,
        deficit_at_depth: Vec<u32>,
    }
    let mut trials: Vec<TrialResult> = Vec::with_capacity(num_trials);

    for _ in 0..num_trials {
        let mut cell_hits = [0u8; 225];
        let mut deficit = initial_deficit;
        let mut deficit_at_depth = Vec::with_capacity(n + 1);
        deficit_at_depth.push(deficit);

        for placements in all_placements.iter() {
            let idx = rng.random_range(0..placements.len());
            let mask = placements[idx].2;

            let mut bits = mask;
            let mut zeros_hit = 0u32;
            while !bits.is_zero() {
                let bit = bits.lowest_set_bit() as usize;
                let current = (initial_value[bit] as u32 + m32 * 32 - cell_hits[bit] as u32) % m32;
                if current == 0 { zeros_hit += 1; }
                cell_hits[bit] = cell_hits[bit].saturating_add(1);
                bits.clear_bit(bit as u32);
            }

            deficit = deficit + m32 * zeros_hit - mask.count_ones();
            deficit_at_depth.push(deficit);
        }

        let max_hits = cell_hits.iter().copied().max().unwrap_or(0);
        trials.push(TrialResult { max_hits, deficit_at_depth });
    }

    // Sort trials by max_hits (ascending) for percentile subsetting.
    trials.sort_unstable_by_key(|t| t.max_hits);

    // For each percentile: take the bottom P% of trials,
    // compute hit_count threshold and per-depth deficit bounds from that subset.
    let percentiles = [50usize, 75, 90, 95];
    let mut levels: Vec<McLevel> = Vec::new();

    for &pct in &percentiles {
        let count = (num_trials * pct / 100).max(1);
        let subset = &trials[..count];

        let hit_count = subset.last().unwrap().max_hits.saturating_add(1).min(31);
        let mut max_deficit = vec![0u32; n + 1];
        for trial in subset {
            for (k, &d) in trial.deficit_at_depth.iter().enumerate() {
                if d > max_deficit[k] { max_deficit[k] = d; }
            }
        }

        let level = McLevel { hit_count, max_deficit_at_depth: max_deficit };
        // Skip if identical to previous level.
        if levels.last().map_or(true, |prev: &McLevel| prev.hit_count != level.hit_count) {
            levels.push(level);
        }
    }

    // Final: all trials (max observed + 1).
    let hit_count = trials.last().unwrap().max_hits.saturating_add(1).min(31);
    let mut max_deficit = vec![0u32; n + 1];
    for trial in &trials {
        for (k, &d) in trial.deficit_at_depth.iter().enumerate() {
            if d > max_deficit[k] { max_deficit[k] = d; }
        }
    }
    let final_level = McLevel { hit_count, max_deficit_at_depth: max_deficit };
    if levels.last().map_or(true, |prev| prev.hit_count != final_level.hit_count) {
        levels.push(final_level);
    }

    debug_assert!(levels.windows(2).all(|w| w[0].hit_count < w[1].hit_count),
        "levels should have strictly increasing hit_count");

    levels
}
