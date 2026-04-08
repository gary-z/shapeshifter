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
/// All bounds are computed jointly from the same subset of MC trials,
/// so the stated confidence is exact.
pub(crate) struct McLevel {
    /// Max cell hit count allowed at each depth k (index 0..=N).
    pub max_hits_at_depth: Vec<u8>,
    /// Max total deficit allowed at each depth k (index 0..=N).
    pub max_deficit_at_depth: Vec<u32>,
    /// Max total jaggedness (H + V mismatched adjacent pairs) at each depth k.
    pub max_jagg_at_depth: Vec<u32>,
    /// Generation-direction bounds: max deficit with j pieces remaining (index 0..=N).
    /// Computed by placing pieces from a solved board. Index j = pieces remaining.
    /// Non-monotonic → disabled at final pipeline level for safety.
    pub gen_max_deficit_by_remaining: Vec<u32>,
    pub gen_max_jagg_by_remaining: Vec<u32>,
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
        return vec![McLevel {
            max_hits_at_depth: vec![0; n + 1],
            max_deficit_at_depth: vec![u32::MAX; n + 1],
            max_jagg_at_depth: vec![u32::MAX; n + 1],
            gen_max_deficit_by_remaining: vec![u32::MAX; n + 1],
            gen_max_jagg_by_remaining: vec![u32::MAX; n + 1],
        }];
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

    let num_trials: usize = 100_000;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(0x5348_4150_4553_4849);
    let initial_deficit = board.total_deficit();
    let m32 = m as u32;

    // Bucket trials by final max_hits value (0..32) to avoid storing all trials.
    // For each bucket, track per-depth max values and trial count.
    const MAX_BUCKETS: usize = 32;
    let mut bucket_count = [0u32; MAX_BUCKETS];
    let mut bucket_max_hits = vec![[0u8; MAX_BUCKETS]; n + 1];   // [depth][bucket]
    let mut bucket_max_deficit = vec![[0u32; MAX_BUCKETS]; n + 1]; // [depth][bucket]
    let mut bucket_max_jagg = vec![[0u32; MAX_BUCKETS]; n + 1];   // [depth][bucket]

    // Precompute board jaggedness masks.
    let mut jagg_h_mask = Bitboard::ZERO;
    let mut jagg_v_mask = Bitboard::ZERO;
    for r in 0..h {
        for c in 0..w {
            let bit = (r * 15 + c) as u32;
            if c + 1 < w { jagg_h_mask.set_bit(bit); }
            if r + 1 < h { jagg_v_mask.set_bit(bit); }
        }
    }

    for _ in 0..num_trials {
        let mut cell_hits = [0u8; 225];
        let mut deficit = initial_deficit;
        let mut running_max_hits: u8 = 0;

        // Simulate board for jaggedness: track cell values.
        let mut cell_value = [0u8; 225];
        cell_value[..225].copy_from_slice(&initial_value[..225]);

        // Depth 0 jaggedness.
        let jagg0 = board.split_jaggedness(jagg_h_mask, jagg_v_mask);
        let jagg0_total = jagg0.circular_h + jagg0.circular_v;

        // We'll store final max_hits as bucket key, accumulate per-depth maxes.
        let mut depth_max_hits = [0u8; 37]; // max 36 pieces + 1
        let mut depth_deficit = [0u32; 37];
        let mut depth_jagg = [0u32; 37];
        depth_deficit[0] = deficit;
        depth_jagg[0] = jagg0_total;

        for (k, placements) in all_placements.iter().enumerate() {
            let idx = rng.random_range(0..placements.len());
            let mask = placements[idx].2;

            let mut bits = mask;
            let mut zeros_hit = 0u32;
            while !bits.is_zero() {
                let bit = bits.lowest_set_bit() as usize;
                let old_val = cell_value[bit];
                if old_val == 0 { zeros_hit += 1; }
                cell_value[bit] = if old_val == 0 { m - 1 } else { old_val - 1 };
                cell_hits[bit] = cell_hits[bit].saturating_add(1);
                if cell_hits[bit] > running_max_hits {
                    running_max_hits = cell_hits[bit];
                }
                bits.clear_bit(bit as u32);
            }

            deficit = deficit + m32 * zeros_hit - mask.count_ones();
            depth_deficit[k + 1] = deficit;
            depth_max_hits[k + 1] = running_max_hits;

            // Compute jaggedness from cell_value array.
            let mut jagg: u32 = 0;
            for r in 0..h {
                for c in 0..w {
                    let v = cell_value[r * 15 + c];
                    if c + 1 < w && cell_value[r * 15 + c + 1] != v { jagg += 1; }
                    if r + 1 < h && cell_value[(r + 1) * 15 + c] != v { jagg += 1; }
                }
            }
            depth_jagg[k + 1] = jagg;
        }

        let bucket = running_max_hits.min(MAX_BUCKETS as u8 - 1) as usize;
        bucket_count[bucket] += 1;
        for k in 0..=n {
            if depth_max_hits[k] > bucket_max_hits[k][bucket] {
                bucket_max_hits[k][bucket] = depth_max_hits[k];
            }
            if depth_deficit[k] > bucket_max_deficit[k][bucket] {
                bucket_max_deficit[k][bucket] = depth_deficit[k];
            }
            if depth_jagg[k] > bucket_max_jagg[k][bucket] {
                bucket_max_jagg[k][bucket] = depth_jagg[k];
            }
        }
    }

    // --- Generation-direction MC: start from solved board, place pieces in reverse ---
    // Bounds indexed by j = number of pieces placed from solved = pieces remaining in solver.
    // gen_max_deficit[j] = max deficit after placing j pieces from solved (100th percentile).
    // gen_max_jagg[j] = max jaggedness after placing j pieces from solved.
    let mut gen_max_deficit = vec![0u32; n + 1]; // index 0 = 0 pieces from solved = solved board
    let mut gen_max_jagg = vec![0u32; n + 1];
    {
        let mut rng2 = rand::rngs::SmallRng::seed_from_u64(0x4745_4E44_4952_4543);
        for _ in 0..num_trials {
            let mut cell_value = [0u8; 225]; // solved board = all zeros
            let mut deficit: u32 = 0;

            for j in 0..n {
                // Place pieces in reverse solver order: piece n-1-j
                let pi = n - 1 - j;
                let placements = &all_placements[pi];
                let idx = rng2.random_range(0..placements.len());
                let mask = placements[idx].2;

                let mut bits = mask;
                let mut zeros_hit = 0u32;
                while !bits.is_zero() {
                    let bit = bits.lowest_set_bit() as usize;
                    let old_val = cell_value[bit];
                    if old_val == 0 { zeros_hit += 1; }
                    cell_value[bit] = if old_val == 0 { m - 1 } else { old_val - 1 };
                    bits.clear_bit(bit as u32);
                }

                deficit = deficit + m32 * zeros_hit - mask.count_ones();
                let remaining = j + 1; // pieces placed from solved = pieces remaining in solver
                if deficit > gen_max_deficit[remaining] {
                    gen_max_deficit[remaining] = deficit;
                }

                // Compute jaggedness.
                let mut jagg: u32 = 0;
                for r in 0..h {
                    for c in 0..w {
                        let v = cell_value[r * 15 + c];
                        if c + 1 < w && cell_value[r * 15 + c + 1] != v { jagg += 1; }
                        if r + 1 < h && cell_value[(r + 1) * 15 + c] != v { jagg += 1; }
                    }
                }
                if jagg > gen_max_jagg[remaining] {
                    gen_max_jagg[remaining] = jagg;
                }
            }
        }
    }

    // Build levels from bucket accumulations.
    // For percentile P: take buckets 0..b where cumulative count >= P% of trials.
    // Per-depth maxes are the max across included buckets.
    let percentiles = [50usize, 75, 90, 95];
    let mut levels: Vec<McLevel> = Vec::new();

    let build_level = |up_to_bucket: usize| -> McLevel {
        let mut max_hits = vec![0u8; n + 1];
        let mut max_deficit = vec![0u32; n + 1];
        let mut max_jagg = vec![0u32; n + 1];
        for k in 0..=n {
            for b in 0..=up_to_bucket {
                if bucket_count[b] == 0 { continue; }
                if bucket_max_hits[k][b] > max_hits[k] { max_hits[k] = bucket_max_hits[k][b]; }
                if bucket_max_deficit[k][b] > max_deficit[k] { max_deficit[k] = bucket_max_deficit[k][b]; }
                if bucket_max_jagg[k][b] > max_jagg[k] { max_jagg[k] = bucket_max_jagg[k][b]; }
            }
        }
        // +1 safety margin on hit counts.
        for h in &mut max_hits {
            *h = h.saturating_add(1).min(31);
        }
        McLevel {
            max_hits_at_depth: max_hits, max_deficit_at_depth: max_deficit,
            max_jagg_at_depth: max_jagg,
            gen_max_deficit_by_remaining: gen_max_deficit.clone(),
            gen_max_jagg_by_remaining: gen_max_jagg.clone(),
        }
    };

    for &pct in &percentiles {
        let needed = (num_trials * pct / 100) as u32;
        let mut cumulative = 0u32;
        let mut up_to = 0;
        for b in 0..MAX_BUCKETS {
            cumulative += bucket_count[b];
            if cumulative >= needed { up_to = b; break; }
        }
        let level = build_level(up_to);
        let final_hit = *level.max_hits_at_depth.last().unwrap();
        if levels.last().map_or(true, |prev: &McLevel|
            *prev.max_hits_at_depth.last().unwrap() != final_hit
        ) {
            levels.push(level);
        }
    }

    // Final: all buckets. Jaggedness set to u32::MAX (disabled) since it's
    // non-monotonic and MC can't guarantee coverage of all valid states.
    let mut final_level = build_level(MAX_BUCKETS - 1);
    for j in &mut final_level.max_jagg_at_depth { *j = u32::MAX; }
    for d in &mut final_level.gen_max_deficit_by_remaining { *d = u32::MAX; }
    for j in &mut final_level.gen_max_jagg_by_remaining { *j = u32::MAX; }
    let final_hit = *final_level.max_hits_at_depth.last().unwrap();
    if levels.last().map_or(true, |prev|
        *prev.max_hits_at_depth.last().unwrap() != final_hit
    ) {
        levels.push(final_level);
    } else {
        // Final deduped with last percentile level — disable non-monotonic bounds.
        let last = levels.last_mut().unwrap();
        for j in &mut last.max_jagg_at_depth { *j = u32::MAX; }
        for d in &mut last.gen_max_deficit_by_remaining { *d = u32::MAX; }
        for j in &mut last.gen_max_jagg_by_remaining { *j = u32::MAX; }
    }

    debug_assert!(levels.windows(2).all(|w|
        w[0].max_hits_at_depth.last() < w[1].max_hits_at_depth.last()),
        "levels should have strictly increasing final hit_count");

    levels
}
