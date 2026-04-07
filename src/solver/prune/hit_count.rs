//! Per-cell hit-count pruning via Monte Carlo threshold.
//!
//! During precompute, Monte Carlo random piece placements establish upper
//! bounds on per-cell hit counts, separately for each starting cell value.
//! Cells starting at 0 rarely need hits (only wrapping), while cells at
//! M-1 need many. Per-value thresholds capture this.
//!
//! Each pipeline threshold level states a confidence: "X% of random solutions
//! satisfy all per-value constraints simultaneously."
//!
//! Hit counts are stored as binary-encoded bitboard planes (5 planes = counts
//! up to 31). Increment is a parallel ripple-carry add — ~10 bitboard ops.
//! The struct is Copy so we use copy-make (no undo needed).

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use rand::{RngExt, SeedableRng};

/// Number of binary planes (5 bits → counts 0–31).
const NUM_PLANES: usize = 5;
/// Maximum M (modulus) we support.
const MAX_M: usize = 6;

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

    /// Returns true if any cell in `region` has hit count >= threshold.
    #[inline(always)]
    pub fn any_in_mask_gte(&self, region: Bitboard, threshold: u8) -> bool {
        if threshold == 0 || region.is_zero() { return false; }

        // Fast path: if no cell in region has any bit set at or above the MSB
        // of threshold, no cell in region can reach threshold.
        let msb = 7 - threshold.leading_zeros() as usize;
        let mut high_any = Bitboard::ZERO;
        for b in msb..NUM_PLANES {
            high_any = high_any | self.planes[b];
        }
        if (high_any & region).is_zero() { return false; }

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
        // Mask to region cells only.
        !(!borrow & region).is_zero()
    }
}

/// Per-starting-value threshold set for one confidence level.
#[derive(Clone)]
pub(crate) struct HitCountThreshold {
    /// threshold[v] = max allowed hits for cells starting at value v.
    pub thresholds: [u8; MAX_M],
    pub m: u8,
}

/// Precomputed masks and thresholds for hit-count pruning.
pub(crate) struct HitCountData {
    /// Bitboard mask for cells that start at value v.
    pub value_masks: [Bitboard; MAX_M],
    /// Number of starting values (= M).
    pub m: u8,
    /// Progressive threshold levels (tightest first).
    pub levels: Vec<HitCountThreshold>,
}

impl HitCountData {
    /// Check if any cell exceeds the thresholds for the given level.
    #[inline(always)]
    pub fn any_exceeds(&self, hits: &HitCounter, thresholds: &[u8; MAX_M]) -> bool {
        for v in 0..self.m as usize {
            if thresholds[v] > 0 && hits.any_in_mask_gte(self.value_masks[v], thresholds[v]) {
                return true;
            }
        }
        false
    }
}

/// Build per-starting-value masks from the initial board.
fn build_value_masks(board: &Board) -> [Bitboard; MAX_M] {
    let mut masks = [Bitboard::ZERO; MAX_M];
    let h = board.height() as usize;
    let w = board.width() as usize;
    for r in 0..h {
        for c in 0..w {
            let v = board.get(r, c) as usize;
            masks[v].set_bit((r * 15 + c) as u32);
        }
    }
    masks
}

/// Run Monte Carlo to find per-starting-value hit count distributions.
/// Returns HitCountData with progressive threshold levels.
pub(crate) fn precompute(
    board: &Board,
    all_placements: &[Vec<(usize, usize, Bitboard)>],
) -> HitCountData {
    let m = board.m();
    let h = board.height() as usize;
    let w = board.width() as usize;
    let value_masks = build_value_masks(board);

    let n = all_placements.len();
    if n == 0 || all_placements.iter().any(|p| p.is_empty()) {
        return HitCountData { value_masks, m, levels: vec![] };
    }

    // Classify each board cell by starting value.
    let mut cell_start_value = [0u8; 225];
    for r in 0..h {
        for c in 0..w {
            cell_start_value[r * 15 + c] = board.get(r, c);
        }
    }

    let num_trials: usize = 10_000;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(0x5348_4150_4553_4849);

    // Per trial: max hits for cells at each starting value.
    let m_usize = m as usize;
    let mut trial_maxes: Vec<[u8; MAX_M]> = Vec::with_capacity(num_trials);

    for _ in 0..num_trials {
        let mut cell_hits = [0u8; 225];
        for placements in all_placements {
            let idx = rng.random_range(0..placements.len());
            let mask = placements[idx].2;
            let mut bits = mask;
            while !bits.is_zero() {
                let bit = bits.lowest_set_bit();
                cell_hits[bit as usize] = cell_hits[bit as usize].saturating_add(1);
                bits.clear_bit(bit);
            }
        }

        let mut maxes = [0u8; MAX_M];
        for r in 0..h {
            for c in 0..w {
                let bit = r * 15 + c;
                let v = cell_start_value[bit] as usize;
                if cell_hits[bit] > maxes[v] {
                    maxes[v] = cell_hits[bit];
                }
            }
        }
        trial_maxes.push(maxes);
    }

    // For each target percentile, compute per-value thresholds.
    // Use independent per-value percentiles. Joint probability >= stated
    // (conservative due to positive correlation between groups).
    let targets = [50usize, 75, 90, 95];
    let mut levels: Vec<HitCountThreshold> = Vec::new();

    for &pct in &targets {
        let mut thresholds = [0u8; MAX_M];
        for v in 0..m_usize {
            let mut vals: Vec<u8> = trial_maxes.iter().map(|t| t[v]).collect();
            vals.sort_unstable();
            let idx = (num_trials * pct / 100).min(num_trials - 1);
            thresholds[v] = vals[idx].saturating_add(1).min(31);
        }
        let t = HitCountThreshold { thresholds, m };
        if levels.last().map_or(true, |prev: &HitCountThreshold| prev.thresholds[..m_usize] != t.thresholds[..m_usize]) {
            levels.push(t);
        }
    }

    // Final: max observed + 1 per value.
    let mut final_t = [0u8; MAX_M];
    for v in 0..m_usize {
        let max_v = trial_maxes.iter().map(|t| t[v]).max().unwrap_or(0);
        final_t[v] = max_v.saturating_add(1).min(31);
    }
    let ft = HitCountThreshold { thresholds: final_t, m };
    if levels.last().map_or(true, |prev| prev.thresholds[..m_usize] != ft.thresholds[..m_usize]) {
        levels.push(ft);
    }

    HitCountData { value_masks, m, levels }
}
