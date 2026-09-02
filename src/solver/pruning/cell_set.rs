//! Exact suffix feasibility for small corner regions.
//!
//! If a region has deficit `D_s` and receives `H` placement-cell hits, solving
//! it requires `H = D_s + M * W`. The table minimizes `W`. A global completion
//! has only `(R - D) / M` such extra cycles, so a region needing more is
//! impossible. Charging entire pieces that touch the region would not be sound,
//! because their cells outside the region can still repair the rest of the board.

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

use super::super::PiecePlacements;

const UNREACHABLE: u16 = u16::MAX;
#[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
const MAX_PACKED_LIMBS: usize = 4;

#[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
#[derive(Clone, Copy, Default)]
struct PackedLimb {
    index: u8,
    mask: u64,
    shift: u8,
}

#[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
struct PackedByte {
    limb_index: u8,
    byte_shift: u8,
    configuration_shift: u8,
    extractions: [u8; 256],
}

struct CellSet {
    #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
    packed_limbs: [PackedLimb; MAX_PACKED_LIMBS],
    #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
    packed_limb_count: u8,
    #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
    packed_bytes: Box<[PackedByte]>,
    configuration_weights: Box<[usize]>,
    config_count: usize,
    minimum_extra_cycles: Vec<u16>,
    layer_maximum_extra_cycles: Vec<u16>,
}

impl CellSet {
    fn precompute(mut cells: Vec<u32>, placements: &[PiecePlacements], modulus: u8) -> Self {
        cells.sort_unstable();
        assert!(cells.len() <= u16::BITS as usize);
        let config_count = (modulus as usize).pow(cells.len() as u32);
        let mut limb_masks = [0u64; 4];
        for &cell in &cells {
            limb_masks[cell as usize / 64] |= 1 << (cell % 64);
        }
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        let mut packed_limbs = [PackedLimb::default(); MAX_PACKED_LIMBS];
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        let mut packed_limb_count = 0;
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        let mut packed_shift = 0;
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        for (index, mask) in limb_masks.into_iter().enumerate() {
            if mask == 0 {
                continue;
            }
            packed_limbs[packed_limb_count] = PackedLimb {
                index: index as u8,
                mask,
                shift: packed_shift,
            };
            packed_limb_count += 1;
            packed_shift += mask.count_ones() as u8;
        }
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        debug_assert!(packed_limb_count <= MAX_PACKED_LIMBS);
        #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
        let packed_bytes = build_packed_bytes(limb_masks);

        let configuration_weights = if modulus == 2 {
            Vec::new()
        } else {
            let mut weights = vec![0usize; 1 << cells.len()];
            for (configuration, weight) in weights.iter_mut().enumerate() {
                let mut multiplier = 1usize;
                for cell_index in 0..cells.len() {
                    if configuration & (1 << cell_index) != 0 {
                        *weight += multiplier;
                    }
                    multiplier *= modulus as usize;
                }
            }
            weights
        };
        let piece_effects = placements
            .iter()
            .map(|piece_placements| {
                let mut effects = piece_placements
                    .iter()
                    .map(|&(_, _, mask)| {
                        cells
                            .iter()
                            .enumerate()
                            .fold(0u16, |effect, (index, &cell)| {
                                effect | (u16::from(mask.get_bit(cell)) << index)
                            })
                    })
                    .collect::<Vec<_>>();
                effects.sort_unstable();
                effects.dedup();
                effects
            })
            .collect::<Vec<_>>();

        let piece_count = placements.len();
        let mut minimum_committed_areas = vec![UNREACHABLE; (piece_count + 1) * config_count];
        minimum_committed_areas[piece_count * config_count] = 0;

        for piece_index in (0..piece_count).rev() {
            let current_base = piece_index * config_count;
            let next_base = current_base + config_count;
            for config in 0..config_count {
                let mut best = UNREACHABLE;
                for &effect in &piece_effects[piece_index] {
                    let next_config = apply_effect(config, effect, cells.len(), modulus as usize);
                    let suffix_area = minimum_committed_areas[next_base + next_config];
                    if suffix_area == UNREACHABLE {
                        continue;
                    }
                    let area = (suffix_area as u32 + effect.count_ones())
                        .min((UNREACHABLE - 1) as u32) as u16;
                    best = best.min(area);
                }
                minimum_committed_areas[current_base + config] = best;
            }
        }

        let mut layer_maximum_extra_cycles = vec![0u16; piece_count + 1];
        for piece_index in 0..=piece_count {
            let layer = &mut minimum_committed_areas
                [piece_index * config_count..(piece_index + 1) * config_count];
            let mut maximum_extra_cycles = 0;
            for (config, minimum_area) in layer.iter_mut().enumerate() {
                if *minimum_area == UNREACHABLE {
                    maximum_extra_cycles = UNREACHABLE;
                    continue;
                }
                let deficit = configuration_deficit(config, cells.len(), modulus as usize);
                debug_assert!(*minimum_area >= deficit);
                debug_assert_eq!((*minimum_area - deficit) % modulus as u16, 0);
                *minimum_area = (*minimum_area - deficit) / modulus as u16;
                if maximum_extra_cycles != UNREACHABLE {
                    maximum_extra_cycles = maximum_extra_cycles.max(*minimum_area);
                }
            }
            layer_maximum_extra_cycles[piece_index] = maximum_extra_cycles;
        }

        Self {
            #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
            packed_limbs,
            #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
            packed_limb_count: packed_limb_count as u8,
            #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
            packed_bytes,
            configuration_weights: configuration_weights.into_boxed_slice(),
            config_count,
            minimum_extra_cycles: minimum_committed_areas,
            layer_maximum_extra_cycles,
        }
    }

    #[inline(always)]
    fn allows<const MODULUS: usize>(
        &self,
        board: &Board,
        piece_index: usize,
        available_extra_cycles: u32,
    ) -> bool {
        let maximum_extra_cycles = self.layer_maximum_extra_cycles[piece_index];
        if maximum_extra_cycles != UNREACHABLE
            && available_extra_cycles >= maximum_extra_cycles as u32
        {
            return true;
        }

        let config = self.encode::<MODULUS>(board);
        let minimum_extra_cycles =
            self.minimum_extra_cycles[piece_index * self.config_count + config];
        minimum_extra_cycles != UNREACHABLE && minimum_extra_cycles as u32 <= available_extra_cycles
    }

    #[inline(always)]
    fn encode<const MODULUS: usize>(&self, board: &Board) -> usize {
        if MODULUS == 2 {
            return self.pack_plane(board.plane(1));
        }
        let mut config = 0;
        for deficit in 1..MODULUS {
            let packed = self.pack_plane(board.plane(deficit as u8));
            config += deficit * self.configuration_weights[packed];
        }
        config
    }

    #[inline(always)]
    fn pack_plane(&self, plane: Bitboard) -> usize {
        let limbs = plane.limbs();
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        {
            let mut packed = 0u64;
            for packed_limb in &self.packed_limbs[..self.packed_limb_count as usize] {
                let limb = limbs[packed_limb.index as usize];
                // SAFETY: this code is compiled only when BMI2 is enabled.
                let extracted = unsafe { std::arch::x86_64::_pext_u64(limb, packed_limb.mask) };
                packed |= extracted << packed_limb.shift;
            }
            packed as usize
        }

        #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
        {
            let mut packed = 0usize;
            for packed_byte in &self.packed_bytes {
                let value =
                    (limbs[packed_byte.limb_index as usize] >> packed_byte.byte_shift) as u8;
                packed |= usize::from(packed_byte.extractions[value as usize])
                    << packed_byte.configuration_shift;
            }
            packed
        }
    }
}

#[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
fn build_packed_bytes(limb_masks: [u64; 4]) -> Box<[PackedByte]> {
    let mut packed_bytes = Vec::new();
    let mut configuration_shift = 0;
    for (limb_index, limb_mask) in limb_masks.into_iter().enumerate() {
        for byte_index in 0..8 {
            let mask = (limb_mask >> (byte_index * 8)) as u8;
            if mask == 0 {
                continue;
            }
            let mut extractions = [0u8; 256];
            for (value, extraction) in extractions.iter_mut().enumerate() {
                *extraction = extract_byte(value as u8, mask);
            }
            packed_bytes.push(PackedByte {
                limb_index: limb_index as u8,
                byte_shift: (byte_index * 8) as u8,
                configuration_shift,
                extractions,
            });
            configuration_shift += mask.count_ones() as u8;
        }
    }
    packed_bytes.into_boxed_slice()
}

#[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
fn extract_byte(value: u8, mut mask: u8) -> u8 {
    let mut extracted = 0;
    let mut output_bit = 1;
    while mask != 0 {
        let bit = mask.isolate_lowest_one();
        extracted |= u8::from(value & bit != 0) * output_bit;
        mask ^= bit;
        output_bit <<= 1;
    }
    extracted
}

pub(crate) struct CellSetBound {
    sets: Vec<CellSet>,
}

impl CellSetBound {
    pub(crate) fn precompute(
        placements: &[PiecePlacements],
        height: u8,
        width: u8,
        modulus: u8,
    ) -> Self {
        let sets = corner_rectangle(height, width, modulus).map_or_else(Vec::new, |rectangle| {
            corner_cell_sets(height, width, rectangle)
                .into_iter()
                .map(|cells| CellSet::precompute(cells, placements, modulus))
                .collect()
        });
        Self { sets }
    }

    #[inline(always)]
    pub(crate) fn allows<const MODULUS: usize>(
        &self,
        board: &Board,
        piece_index: usize,
        remaining_area: u32,
    ) -> bool {
        if self.sets.is_empty() {
            return true;
        }
        let available_extra_cycles =
            remaining_area.saturating_sub(board.total_deficit()) / MODULUS as u32;
        self.sets
            .iter()
            .all(|set| set.allows::<MODULUS>(board, piece_index, available_extra_cycles))
    }
}

fn corner_rectangle(height: u8, width: u8, modulus: u8) -> Option<(usize, usize)> {
    match modulus {
        2 => Some((usize::from(height.min(4)), usize::from(width.min(4)))),
        3 | 4 => Some((3, 3)),
        5 => None,
        _ => unreachable!(),
    }
}

fn corner_cell_sets(
    height: u8,
    width: u8,
    (set_height, set_width): (usize, usize),
) -> Vec<Vec<u32>> {
    let height = height as usize;
    let width = width as usize;
    assert!(set_height <= height && set_width <= width);
    let mut sets = Vec::new();
    for (start_row, start_column) in [
        (0, 0),
        (0, width - set_width),
        (height - set_height, 0),
        (height - set_height, width - set_width),
    ] {
        let cells = (start_row..start_row + set_height)
            .flat_map(|row| {
                (start_column..start_column + set_width)
                    .map(move |column| (row * STRIDE + column) as u32)
            })
            .collect::<Vec<_>>();
        if !sets.contains(&cells) {
            sets.push(cells);
        }
    }
    sets
}

fn apply_effect(config: usize, effect: u16, cell_count: usize, modulus: usize) -> usize {
    let mut result = config;
    let mut multiplier = 1usize;
    for cell_index in 0..cell_count {
        if effect & (1 << cell_index) != 0 {
            let digit = result / multiplier % modulus;
            if digit == 0 {
                result += (modulus - 1) * multiplier;
            } else {
                result -= multiplier;
            }
        }
        multiplier *= modulus;
    }
    result
}

fn configuration_deficit(mut config: usize, cell_count: usize, modulus: usize) -> u16 {
    let mut deficit = 0u16;
    for _ in 0..cell_count {
        deficit += (config % modulus) as u16;
        config /= modulus;
    }
    deficit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::Piece;

    fn placements(pieces: &[Piece], height: u8, width: u8) -> Vec<PiecePlacements> {
        pieces
            .iter()
            .map(|piece| piece.placements(height, width))
            .collect()
    }

    #[test]
    fn stores_minimum_area_committed_to_the_set() {
        let domino = Piece::from_grid(&[&[true, true]]);
        let all_placements = placements(&[domino, domino], 3, 4);
        let cells = (0..3)
            .flat_map(|row| (0..3).map(move |column| (row * STRIDE + column) as u32))
            .collect();
        let set = CellSet::precompute(cells, &all_placements, 2);
        let board = Board::from_grid(&[&[0, 1, 0, 0], &[0, 0, 0, 1], &[0, 0, 0, 1]], 2);
        let config = set.encode::<2>(&board);

        assert_eq!(set.minimum_extra_cycles[config], 1);
        assert!(!set.allows::<2>(&board, 0, 0));
    }

    #[test]
    fn rejects_an_unreachable_set_configuration() {
        let domino = Piece::from_grid(&[&[true, true]]);
        let all_placements = placements(&[domino], 3, 3);
        let cells = (0..3)
            .flat_map(|row| (0..3).map(move |column| (row * STRIDE + column) as u32))
            .collect();
        let set = CellSet::precompute(cells, &all_placements, 2);
        let board = Board::from_grid(&[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]], 2);

        assert!(!set.allows::<2>(&board, 0, 0));
    }

    #[test]
    fn encodes_large_base_five_cell_sets() {
        let cells = (0..2)
            .flat_map(|row| (0..4).map(move |column| (row * STRIDE + column) as u32))
            .collect();
        let set = CellSet::precompute(cells, &[], 5);
        let board = Board::from_grid(&[&[4, 4, 4, 4], &[4, 4, 4, 4], &[0, 0, 0, 0]], 5);

        assert_eq!(set.encode::<5>(&board), 5usize.pow(8) - 1);
    }

    fn assert_generated_states_are_allowed<const MODULUS: usize>() {
        fn visit<const MODULUS: usize>(
            bound: &CellSetBound,
            placements: &[PiecePlacements],
            board: Board,
            next_piece: usize,
            suffix_start: usize,
            remaining_area: u32,
        ) {
            if next_piece == placements.len() {
                assert!(
                    bound.allows::<MODULUS>(&board, suffix_start, remaining_area),
                    "rejected generated solvable state: {board:?}, suffix={suffix_start}"
                );
                return;
            }
            for &(_, _, mask) in &placements[next_piece] {
                let mut predecessor = board;
                predecessor.undo_piece(mask);
                visit::<MODULUS>(
                    bound,
                    placements,
                    predecessor,
                    next_piece + 1,
                    suffix_start,
                    remaining_area,
                );
            }
        }

        let pieces = [
            Piece::from_grid(&[&[true, true], &[true, false]]),
            Piece::from_grid(&[&[true, true]]),
            Piece::from_grid(&[&[true], &[true]]),
            Piece::from_grid(&[&[true]]),
        ];
        let all_placements = placements(&pieces, 3, 3);
        let rectangle = if MODULUS == 5 { (2, 2) } else { (3, 3) };
        let bound = CellSetBound {
            sets: corner_cell_sets(3, 3, rectangle)
                .into_iter()
                .map(|cells| CellSet::precompute(cells, &all_placements, MODULUS as u8))
                .collect(),
        };
        for suffix_start in 0..pieces.len() {
            let remaining_area = pieces[suffix_start..].iter().map(Piece::cell_count).sum();
            visit::<MODULUS>(
                &bound,
                &all_placements,
                Board::new_solved(3, 3, MODULUS as u8),
                suffix_start,
                suffix_start,
                remaining_area,
            );
        }
    }

    #[test]
    fn allows_exhaustively_generated_solvable_states() {
        assert_generated_states_are_allowed::<2>();
        assert_generated_states_are_allowed::<3>();
        assert_generated_states_are_allowed::<4>();
        assert_generated_states_are_allowed::<5>();
    }
}
