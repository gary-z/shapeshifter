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
use rand::RngExt;

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
        // Fast path: if no cell has any bit set at or above the MSB of threshold,
        // then no cell can reach threshold. This skips the full comparison at
        // early depths when all counts are small.
        let msb = 7 - threshold.leading_zeros() as usize; // highest bit in threshold
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

/// Run Monte Carlo to find the max per-cell hit count across random solutions.
/// Returns `max_observed + 1` as a conservative hard-prune threshold,
/// or 0 if pruning should be disabled (too few placements to be meaningful).
pub(crate) fn precompute_threshold(
    all_placements: &[Vec<(usize, usize, Bitboard)>],
) -> u8 {
    let n = all_placements.len();
    if n == 0 { return 0; }

    // Check all pieces have at least one placement.
    if all_placements.iter().any(|p| p.is_empty()) { return 0; }

    let num_trials = 10_000;
    let mut rng = rand::rng();
    let mut max_observed: u8 = 0;

    for _ in 0..num_trials {
        let mut cell_hits = [0u8; 225];
        for placements in all_placements {
            let idx = rng.random_range(0..placements.len());
            let mask = placements[idx].2;
            // Iterate set bits.
            let mut m = mask;
            while !m.is_zero() {
                let bit = m.lowest_set_bit();
                cell_hits[bit as usize] = cell_hits[bit as usize].saturating_add(1);
                m.clear_bit(bit);
            }
        }
        let trial_max = cell_hits.iter().copied().max().unwrap_or(0);
        if trial_max > max_observed {
            max_observed = trial_max;
        }
    }

    // Threshold: prune if any cell hits exceed max observed.
    // Adding 1 for safety margin (actual solution is one sample we didn't see).
    max_observed.saturating_add(1).min(31)
}
