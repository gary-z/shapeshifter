//! Depth-aware hit-count, deficit, and jaggedness bounds from random placements.
//!
//! The search starts with percentile bounds and retries with progressively wider
//! sample sets. The final level includes every sampled forward trajectory and
//! disables bounds that are not safe to carry into that fallback.

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use rand::{RngExt, SeedableRng};

const NUM_PLANES: usize = 5;
const TRIAL_COUNT: usize = 100_000;
const GRID_CAPACITY: usize = STRIDE * STRIDE;
const MAX_PIECES: usize = 36;

#[derive(Clone, Copy)]
pub(crate) struct HitCounter {
    planes: [Bitboard; NUM_PLANES],
}

impl HitCounter {
    pub(crate) fn new() -> Self {
        Self {
            planes: [Bitboard::ZERO; NUM_PLANES],
        }
    }

    #[inline(always)]
    pub(crate) fn apply_piece(&mut self, mask: Bitboard) {
        let mut carry = mask;
        for plane in &mut self.planes {
            let new = *plane ^ carry;
            carry = *plane & carry;
            *plane = new;
            if carry.is_zero() {
                break;
            }
        }
    }

    #[inline(always)]
    fn any_cell_reaches(&self, threshold: u8) -> bool {
        if threshold == 0 {
            return false;
        }

        // Fast path: if no cell has any bit set at or above the MSB of threshold,
        // then no cell can reach threshold. This skips the full comparison at
        // early depths when all counts are small.
        let most_significant_bit = 7 - threshold.leading_zeros() as usize;
        let mut high_bits = Bitboard::ZERO;
        for bit in most_significant_bit..NUM_PLANES {
            high_bits = high_bits | self.planes[bit];
        }
        if high_bits.is_zero() {
            return false;
        }

        // Full comparison via parallel subtraction: count - threshold.
        // borrow = 1 means count < threshold; borrow = 0 means count >= threshold.
        let mut borrow = Bitboard::ZERO;
        for bit in 0..NUM_PLANES {
            let count_plane = self.planes[bit];
            if (threshold >> bit) & 1 == 1 {
                borrow = !count_plane | borrow;
            } else {
                borrow = !count_plane & borrow;
            }
        }
        // Non-board cells have count 0, so if threshold > 0 they have borrow = 1.
        !(!borrow).is_zero()
    }
}

struct MonteCarloLevel {
    max_hits_at_depth: Vec<u8>,
    max_deficit_at_depth: Vec<u32>,
    max_jaggedness_at_depth: Vec<u32>,
    max_reverse_deficit: Vec<u32>,
    max_reverse_jaggedness: Vec<u32>,
}

pub(crate) struct MonteCarloBounds {
    levels: Vec<MonteCarloLevel>,
    selected_level: std::sync::atomic::AtomicUsize,
    piece_count: usize,
}

impl MonteCarloBounds {
    pub(crate) fn precompute(
        board: &Board,
        placements: &[Vec<(usize, usize, Bitboard)>],
        modulus: u8,
    ) -> Self {
        let levels = sample_levels(board, placements, modulus);
        Self {
            selected_level: std::sync::atomic::AtomicUsize::new(levels.len() - 1),
            piece_count: placements.len(),
            levels,
        }
    }

    pub(crate) fn level_count(&self) -> usize {
        self.levels.len()
    }

    pub(crate) fn select_level(&self, level: usize) {
        self.selected_level
            .store(level, std::sync::atomic::Ordering::Relaxed);
    }

    #[inline(always)]
    pub(crate) fn allows_state<const MODULUS: usize>(
        &self,
        board: &Board,
        piece_index: usize,
        jaggedness_bound: &super::jaggedness::JaggednessBound,
    ) -> bool {
        let level_index = self
            .selected_level
            .load(std::sync::atomic::Ordering::Relaxed);
        let level = &self.levels[level_index];
        let deficit = board.total_deficit();

        if deficit > level.max_deficit_at_depth[piece_index] {
            return false;
        }

        let remaining_pieces = self.piece_count - piece_index;
        if deficit > level.max_reverse_deficit[remaining_pieces] {
            return false;
        }

        let jaggedness = super::jaggedness::measure::<MODULUS>(
            board,
            jaggedness_bound.horizontal_mask(),
            jaggedness_bound.vertical_mask(),
        );
        let total_jaggedness =
            jaggedness.horizontal_circular_distance + jaggedness.vertical_circular_distance;
        if total_jaggedness > level.max_jaggedness_at_depth[piece_index] {
            return false;
        }
        if total_jaggedness > level.max_reverse_jaggedness[remaining_pieces] {
            return false;
        }
        if !jaggedness_bound.allows(&jaggedness, piece_index, MODULUS as u8) {
            return false;
        }

        true
    }

    #[inline(always)]
    pub(crate) fn exceeds_hit_threshold(&self, hits: &HitCounter, depth: usize) -> bool {
        let level_index = self
            .selected_level
            .load(std::sync::atomic::Ordering::Relaxed);
        let threshold = self.levels[level_index].max_hits_at_depth[depth];
        threshold > 0 && hits.any_cell_reaches(threshold)
    }
}

fn count_adjacent_mismatches(
    cell_values: &[u8; GRID_CAPACITY],
    height: usize,
    width: usize,
) -> u32 {
    let mut mismatch_count = 0;
    for row in 0..height {
        for column in 0..width {
            let value = cell_values[row * STRIDE + column];
            if column + 1 < width && cell_values[row * STRIDE + column + 1] != value {
                mismatch_count += 1;
            }
            if row + 1 < height && cell_values[(row + 1) * STRIDE + column] != value {
                mismatch_count += 1;
            }
        }
    }
    mismatch_count
}

fn sample_levels(
    board: &Board,
    placements: &[Vec<(usize, usize, Bitboard)>],
    modulus: u8,
) -> Vec<MonteCarloLevel> {
    let piece_count = placements.len();
    if piece_count == 0 || placements.iter().any(Vec::is_empty) {
        return vec![MonteCarloLevel {
            max_hits_at_depth: vec![0; piece_count + 1],
            max_deficit_at_depth: vec![u32::MAX; piece_count + 1],
            max_jaggedness_at_depth: vec![u32::MAX; piece_count + 1],
            max_reverse_deficit: vec![u32::MAX; piece_count + 1],
            max_reverse_jaggedness: vec![u32::MAX; piece_count + 1],
        }];
    }
    debug_assert!(piece_count <= MAX_PIECES);

    let height = board.height() as usize;
    let width = board.width() as usize;
    let mut initial_values = [0u8; GRID_CAPACITY];
    for row in 0..height {
        for column in 0..width {
            initial_values[row * STRIDE + column] = board.get(row, column);
        }
    }

    let mut horizontal_mask = Bitboard::ZERO;
    let mut vertical_mask = Bitboard::ZERO;
    for row in 0..height {
        for column in 0..width {
            let bit = (row * STRIDE + column) as u32;
            if column + 1 < width {
                horizontal_mask.set_bit(bit);
            }
            if row + 1 < height {
                vertical_mask.set_bit(bit);
            }
        }
    }

    let initial_jaggedness = match modulus {
        2 => super::jaggedness::measure::<2>(board, horizontal_mask, vertical_mask),
        3 => super::jaggedness::measure::<3>(board, horizontal_mask, vertical_mask),
        4 => super::jaggedness::measure::<4>(board, horizontal_mask, vertical_mask),
        5 => super::jaggedness::measure::<5>(board, horizontal_mask, vertical_mask),
        _ => unreachable!(),
    };
    let initial_jaggedness = initial_jaggedness.horizontal_circular_distance
        + initial_jaggedness.vertical_circular_distance;
    let initial_deficit = board.total_deficit();
    let modulus_u32 = modulus as u32;

    const MAX_BUCKETS: usize = 32;
    let mut bucket_counts = [0u32; MAX_BUCKETS];
    let mut bucket_max_hits = vec![[0u8; MAX_BUCKETS]; piece_count + 1];
    let mut bucket_max_deficit = vec![[0u32; MAX_BUCKETS]; piece_count + 1];
    let mut bucket_max_jaggedness = vec![[0u32; MAX_BUCKETS]; piece_count + 1];
    let mut forward_rng = rand::rngs::SmallRng::seed_from_u64(0x5348_4150_4553_4849);

    for _ in 0..TRIAL_COUNT {
        let mut cell_hits = [0u8; GRID_CAPACITY];
        let mut cell_values = initial_values;
        let mut deficit = initial_deficit;
        let mut running_max_hits = 0u8;
        let mut max_hits_by_depth = [0u8; MAX_PIECES + 1];
        let mut deficit_by_depth = [0u32; MAX_PIECES + 1];
        let mut jaggedness_by_depth = [0u32; MAX_PIECES + 1];
        deficit_by_depth[0] = deficit;
        jaggedness_by_depth[0] = initial_jaggedness;

        for (piece_index, piece_placements) in placements.iter().enumerate() {
            let placement_index = forward_rng.random_range(0..piece_placements.len());
            let mask = piece_placements[placement_index].2;
            let mut remaining_bits = mask;
            let mut zeros_hit = 0u32;

            while !remaining_bits.is_zero() {
                let bit = remaining_bits.lowest_set_bit() as usize;
                let old_value = cell_values[bit];
                if old_value == 0 {
                    zeros_hit += 1;
                }
                cell_values[bit] = if old_value == 0 {
                    modulus - 1
                } else {
                    old_value - 1
                };
                cell_hits[bit] = cell_hits[bit].saturating_add(1);
                running_max_hits = running_max_hits.max(cell_hits[bit]);
                remaining_bits.clear_bit(bit as u32);
            }

            deficit = deficit + modulus_u32 * zeros_hit - mask.count_ones();
            let depth = piece_index + 1;
            deficit_by_depth[depth] = deficit;
            max_hits_by_depth[depth] = running_max_hits;
            jaggedness_by_depth[depth] = count_adjacent_mismatches(&cell_values, height, width);
        }

        let bucket_index = running_max_hits.min(MAX_BUCKETS as u8 - 1) as usize;
        bucket_counts[bucket_index] += 1;
        for depth in 0..=piece_count {
            bucket_max_hits[depth][bucket_index] =
                bucket_max_hits[depth][bucket_index].max(max_hits_by_depth[depth]);
            bucket_max_deficit[depth][bucket_index] =
                bucket_max_deficit[depth][bucket_index].max(deficit_by_depth[depth]);
            bucket_max_jaggedness[depth][bucket_index] =
                bucket_max_jaggedness[depth][bucket_index].max(jaggedness_by_depth[depth]);
        }
    }

    // Reverse depth is the number of pieces remaining in the forward search.
    let mut max_reverse_deficit = vec![0u32; piece_count + 1];
    let mut max_reverse_jaggedness = vec![0u32; piece_count + 1];
    let mut reverse_rng = rand::rngs::SmallRng::seed_from_u64(0x5245_5645_5253_454D);
    for _ in 0..TRIAL_COUNT {
        let mut cell_values = [0u8; GRID_CAPACITY];
        let mut deficit = 0u32;

        for reverse_index in 0..piece_count {
            let piece_placements = &placements[piece_count - 1 - reverse_index];
            let placement_index = reverse_rng.random_range(0..piece_placements.len());
            let mask = piece_placements[placement_index].2;
            let mut remaining_bits = mask;
            let mut zeros_hit = 0u32;

            while !remaining_bits.is_zero() {
                let bit = remaining_bits.lowest_set_bit() as usize;
                let old_value = cell_values[bit];
                if old_value == 0 {
                    zeros_hit += 1;
                }
                cell_values[bit] = if old_value == 0 {
                    modulus - 1
                } else {
                    old_value - 1
                };
                remaining_bits.clear_bit(bit as u32);
            }

            deficit = deficit + modulus_u32 * zeros_hit - mask.count_ones();
            let reverse_depth = reverse_index + 1;
            max_reverse_deficit[reverse_depth] = max_reverse_deficit[reverse_depth].max(deficit);
            max_reverse_jaggedness[reverse_depth] = max_reverse_jaggedness[reverse_depth]
                .max(count_adjacent_mismatches(&cell_values, height, width));
        }
    }

    // Each percentile level uses one consistent subset of sampled trajectories.
    let percentiles = [50usize, 75, 90, 95];
    let mut levels = Vec::new();
    let build_level = |maximum_bucket: usize| -> MonteCarloLevel {
        let mut max_hits = vec![0u8; piece_count + 1];
        let mut max_deficit = vec![0u32; piece_count + 1];
        let mut max_jaggedness = vec![0u32; piece_count + 1];
        for depth in 0..=piece_count {
            for bucket_index in 0..=maximum_bucket {
                if bucket_counts[bucket_index] == 0 {
                    continue;
                }
                max_hits[depth] = max_hits[depth].max(bucket_max_hits[depth][bucket_index]);
                max_deficit[depth] =
                    max_deficit[depth].max(bucket_max_deficit[depth][bucket_index]);
                max_jaggedness[depth] =
                    max_jaggedness[depth].max(bucket_max_jaggedness[depth][bucket_index]);
            }
        }
        for max_hit in &mut max_hits {
            *max_hit = max_hit.saturating_add(1).min(31);
        }
        MonteCarloLevel {
            max_hits_at_depth: max_hits,
            max_deficit_at_depth: max_deficit,
            max_jaggedness_at_depth: max_jaggedness,
            max_reverse_deficit: max_reverse_deficit.clone(),
            max_reverse_jaggedness: max_reverse_jaggedness.clone(),
        }
    };

    for percentile in percentiles {
        let required_samples = (TRIAL_COUNT * percentile / 100) as u32;
        let mut cumulative_samples = 0u32;
        let mut maximum_bucket = 0;
        for (bucket_index, bucket_count) in bucket_counts.iter().enumerate() {
            cumulative_samples += *bucket_count;
            if cumulative_samples >= required_samples {
                maximum_bucket = bucket_index;
                break;
            }
        }

        let level = build_level(maximum_bucket);
        let final_hit_threshold = *level.max_hits_at_depth.last().unwrap();
        if levels.last().is_none_or(|previous: &MonteCarloLevel| {
            *previous.max_hits_at_depth.last().unwrap() != final_hit_threshold
        }) {
            levels.push(level);
        }
    }

    // The widest level keeps only sampled bounds that are monotonic over valid states.
    let mut final_level = build_level(MAX_BUCKETS - 1);
    final_level.max_jaggedness_at_depth.fill(u32::MAX);
    final_level.max_reverse_deficit.fill(u32::MAX);
    final_level.max_reverse_jaggedness.fill(u32::MAX);

    let final_hit_threshold = *final_level.max_hits_at_depth.last().unwrap();
    if levels
        .last()
        .is_none_or(|previous| *previous.max_hits_at_depth.last().unwrap() != final_hit_threshold)
    {
        levels.push(final_level);
    } else {
        let widest_level = levels.last_mut().unwrap();
        widest_level.max_jaggedness_at_depth.fill(u32::MAX);
        widest_level.max_reverse_deficit.fill(u32::MAX);
        widest_level.max_reverse_jaggedness.fill(u32::MAX);
    }

    debug_assert!(
        levels
            .windows(2)
            .all(|pair| { pair[0].max_hits_at_depth.last() < pair[1].max_hits_at_depth.last() }),
        "levels should have strictly increasing final hit thresholds"
    );

    levels
}
