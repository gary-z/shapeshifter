//! Rejects states whose remaining pieces cannot cover the board's deficit.
//!
//! A connected multi-cell piece covering an isolated nonzero cell must also touch
//! a currently zero neighbor. Each distinct zero neighbor touched consumes at
//! least one of the `(remaining cells - deficit) / M` future zero-hit cycles.
//! When only one such cycle remains, the piece shapes determine whether every
//! required placement can share that same zero cell.

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::{Board, MAX_M};
use crate::core::piece::Piece;

use super::super::PiecePlacements;

pub(crate) struct TotalDeficitBound {
    remaining_cells: Vec<u32>,
    remaining_single_cell_pieces: Vec<u32>,
    remaining_horizontal_dominoes: Vec<u32>,
    remaining_vertical_dominoes: Vec<u32>,
    placement_masks: Vec<Vec<Bitboard>>,
    adjacent_placement_indices: Vec<AdjacentPlacementIndices>,
    shape_piece_order_by_suffix: Vec<Box<[u8]>>,
    pieces_are_connected: bool,
    valid_mask: Bitboard,
}

const DIRECTION_COUNT: usize = 4;
const BITBOARD_BITS: usize = 256;

struct AdjacentPlacementIndices {
    offsets: Box<[u16]>,
    indices: Box<[u8]>,
}

impl AdjacentPlacementIndices {
    fn precompute(placements: &[Bitboard]) -> Self {
        let mut counts = [0u16; BITBOARD_BITS * DIRECTION_COUNT];
        for &placement in placements {
            for_each_adjacent_pair(placement, |target, direction| {
                counts[target as usize * DIRECTION_COUNT + direction] += 1;
            });
        }

        let mut offsets = Vec::with_capacity(counts.len() + 1);
        let mut total = 0u16;
        offsets.push(0u16);
        for count in counts {
            total += count;
            offsets.push(total);
        }

        let mut indices = vec![0u8; total as usize];
        let mut cursors = offsets[..offsets.len() - 1].to_vec();
        for (placement_index, &placement) in placements.iter().enumerate() {
            let placement_index = u8::try_from(placement_index).unwrap();
            for_each_adjacent_pair(placement, |target, direction| {
                let bucket = target as usize * DIRECTION_COUNT + direction;
                indices[cursors[bucket] as usize] = placement_index;
                cursors[bucket] += 1;
            });
        }

        Self {
            offsets: offsets.into_boxed_slice(),
            indices: indices.into_boxed_slice(),
        }
    }

    #[inline(always)]
    fn get(&self, target: u32, direction: usize) -> &[u8] {
        let bucket = target as usize * DIRECTION_COUNT + direction;
        &self.indices[self.offsets[bucket] as usize..self.offsets[bucket + 1] as usize]
    }
}

fn for_each_adjacent_pair(placement: Bitboard, mut visit: impl FnMut(u32, usize)) {
    let mut targets = placement;
    while !targets.is_zero() {
        let target = targets.lowest_set_bit();
        targets.clear_bit(target);
        let target_mask = Bitboard::from_bit(target);
        for (direction, neighbor) in [
            target_mask.shr_stride(),
            target_mask.shl_stride(),
            target_mask.shr_1(),
            target_mask.shl_1(),
        ]
        .into_iter()
        .enumerate()
        {
            if !(placement & neighbor).is_zero() {
                visit(target, direction);
            }
        }
    }
}

impl TotalDeficitBound {
    pub(crate) fn precompute(
        pieces: &[Piece],
        piece_order: &[usize],
        placements: &[PiecePlacements],
        height: u8,
        width: u8,
    ) -> Self {
        let piece_count = pieces.len();
        let mut remaining_cells = vec![0u32; piece_count + 1];
        let mut remaining_single_cell_pieces = vec![0u32; piece_count + 1];
        let mut remaining_horizontal_dominoes = vec![0u32; piece_count + 1];
        let mut remaining_vertical_dominoes = vec![0u32; piece_count + 1];
        for piece_index in (0..piece_count).rev() {
            let piece = &pieces[piece_order[piece_index]];
            let cell_count = piece.cell_count();
            remaining_cells[piece_index] = remaining_cells[piece_index + 1] + cell_count;
            remaining_single_cell_pieces[piece_index] =
                remaining_single_cell_pieces[piece_index + 1] + u32::from(cell_count == 1);
            remaining_horizontal_dominoes[piece_index] = remaining_horizontal_dominoes
                [piece_index + 1]
                + u32::from(piece.shape() == (Bitboard::from_bit(0) | Bitboard::from_bit(1)));
            remaining_vertical_dominoes[piece_index] = remaining_vertical_dominoes[piece_index + 1]
                + u32::from(
                    piece.shape() == (Bitboard::from_bit(0) | Bitboard::from_bit(STRIDE as u32)),
                );
        }

        let placement_masks = placements
            .iter()
            .map(|placements| {
                placements
                    .iter()
                    .map(|&(_, _, mask)| mask)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let adjacent_placement_indices = placement_masks
            .iter()
            .map(|placements| AdjacentPlacementIndices::precompute(placements))
            .collect();
        let shape_piece_order_by_suffix = (0..=piece_count)
            .map(|piece_index| {
                let mut remaining = (piece_index..piece_count)
                    .filter(|&index| pieces[piece_order[index]].cell_count() > 1)
                    .collect::<Vec<_>>();
                remaining.sort_by_key(|&index| pieces[piece_order[index]].cell_count());
                remaining
                    .into_iter()
                    .map(|index| index as u8)
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .collect();

        let mut valid_mask = Bitboard::ZERO;
        for row in 0..height as usize {
            for column in 0..width as usize {
                valid_mask.set_bit((row * STRIDE + column) as u32);
            }
        }

        Self {
            remaining_cells,
            remaining_single_cell_pieces,
            remaining_horizontal_dominoes,
            remaining_vertical_dominoes,
            placement_masks,
            adjacent_placement_indices,
            shape_piece_order_by_suffix,
            pieces_are_connected: pieces.iter().all(|piece| shape_is_connected(piece.shape())),
            valid_mask,
        }
    }

    #[inline(always)]
    pub(crate) fn remaining_cells(&self, piece_index: usize) -> u32 {
        self.remaining_cells[piece_index]
    }

    #[inline(always)]
    pub(crate) fn allows<const MODULUS: usize>(&self, board: &Board, piece_index: usize) -> bool {
        let remaining_cells = self.remaining_cells[piece_index];
        let total_deficit = board.total_deficit();
        if remaining_cells < total_deficit {
            return false;
        }
        if !self.pieces_are_connected {
            return true;
        }

        let zero_hit_budget = (remaining_cells - total_deficit) / MODULUS as u32;
        let nonzero = self.valid_mask & !board.plane(0);
        let neighbors =
            nonzero.shl_1() | nonzero.shr_1() | nonzero.shl_stride() | nonzero.shr_stride();
        let isolated = nonzero & !neighbors;
        let isolated_count = isolated.count_ones();
        if isolated_count == 0 {
            return true;
        }

        let single_cell_pieces = self.remaining_single_cell_pieces[piece_index];
        let highest_deficit = MODULUS as u32 - 1;
        let guaranteed_one_zero_pieces = single_cell_pieces
            + self.remaining_horizontal_dominoes[piece_index]
                .max(self.remaining_vertical_dominoes[piece_index]);
        let shape_check_may_help = zero_hit_budget == 1
            && !(isolated & board.plane(highest_deficit as u8)).is_zero()
            && guaranteed_one_zero_pieces < highest_deficit;
        if !shape_check_may_help && zero_hit_budget >= isolated_count {
            return true;
        }

        let mut isolated_counts = [0u32; MAX_M];
        let mut isolated_deficit = 0;
        for (deficit, isolated_count) in
            isolated_counts.iter_mut().enumerate().take(MODULUS).skip(1)
        {
            let count = (isolated & board.plane(deficit as u8)).count_ones();
            *isolated_count = count;
            isolated_deficit += deficit as u32 * count;
        }

        if isolated_deficit <= single_cell_pieces {
            return true;
        }

        if zero_hit_budget < isolated_count {
            let required_neighbor_cover = isolated_deficit - single_cell_pieces;
            if zero_hit_budget
                * maximum_singleton_deficit_around_one_neighbor::<MODULUS>(&isolated_counts)
                < required_neighbor_cover
            {
                return false;
            }

            if zero_hit_budget
                < minimum_unsupported_singletons::<MODULUS>(&isolated_counts, single_cell_pieces)
                && singleton_neighbor_cover_upper_bound::<MODULUS>(
                    board,
                    isolated,
                    self.valid_mask,
                    zero_hit_budget,
                ) < required_neighbor_cover
            {
                return false;
            }
        }

        !shape_check_may_help
            || self.highest_deficit_singletons_are_coverable(
                board,
                isolated & board.plane(highest_deficit as u8),
                piece_index,
                highest_deficit - single_cell_pieces,
            )
    }

    fn highest_deficit_singletons_are_coverable(
        &self,
        board: &Board,
        mut isolated: Bitboard,
        piece_index: usize,
        required_multi_cell_pieces: u32,
    ) -> bool {
        let zero = board.plane(0);
        while !isolated.is_zero() {
            let cell = isolated.lowest_set_bit();
            isolated.clear_bit(cell);
            let cell_mask = Bitboard::from_bit(cell);
            let candidate_zeros = [
                zero & cell_mask.shr_stride(),
                zero & cell_mask.shl_stride(),
                zero & cell_mask.shr_1(),
                zero & cell_mask.shl_1(),
            ];
            let mut coverable = false;

            for (direction, zero_mask) in candidate_zeros.into_iter().enumerate() {
                if zero_mask.is_zero() {
                    continue;
                }
                let mut capable_pieces = 0;

                for &current_piece in &self.shape_piece_order_by_suffix[piece_index] {
                    let current_piece = current_piece as usize;
                    let placements = &self.placement_masks[current_piece];
                    for &placement_index in
                        self.adjacent_placement_indices[current_piece].get(cell, direction)
                    {
                        if (placements[placement_index as usize] & zero) == zero_mask {
                            capable_pieces += 1;
                            break;
                        }
                    }
                    if capable_pieces == required_multi_cell_pieces {
                        coverable = true;
                        break;
                    }
                }
                if coverable {
                    break;
                }
            }

            if !coverable {
                return false;
            }
        }
        true
    }
}

fn maximum_singleton_deficit_around_one_neighbor<const MODULUS: usize>(
    isolated_counts: &[u32; MAX_M],
) -> u32 {
    let mut remaining_neighbors = 4;
    let mut maximum_deficit = 0;
    for deficit in (1..MODULUS).rev() {
        let used = remaining_neighbors.min(isolated_counts[deficit]);
        maximum_deficit += used * deficit as u32;
        remaining_neighbors -= used;
    }
    maximum_deficit
}

fn minimum_unsupported_singletons<const MODULUS: usize>(
    isolated_counts: &[u32; MAX_M],
    single_cell_pieces: u32,
) -> u32 {
    let mut remaining_single_cell_pieces = single_cell_pieces;
    let mut unsupported: u32 = isolated_counts.iter().sum();
    for (deficit, &isolated_count) in isolated_counts.iter().enumerate().take(MODULUS).skip(1) {
        let supported = isolated_count.min(remaining_single_cell_pieces / deficit as u32);
        unsupported -= supported;
        remaining_single_cell_pieces -= supported * deficit as u32;
    }
    unsupported
}

fn shape_is_connected(shape: Bitboard) -> bool {
    let mut reached = Bitboard::from_bit(shape.lowest_set_bit());
    loop {
        let expanded = (reached
            | reached.shl_1()
            | reached.shr_1()
            | reached.shl_stride()
            | reached.shr_stride())
            & shape;
        if expanded == reached {
            return reached == shape;
        }
        reached = expanded;
    }
}

fn singleton_neighbor_cover_upper_bound<const MODULUS: usize>(
    board: &Board,
    isolated: Bitboard,
    valid_mask: Bitboard,
    zero_hit_budget: u32,
) -> u32 {
    const SCORE_BITS: usize = 5;
    const DEFICIT_BITS: usize = 3;
    const MAX_NEIGHBOR_DEFICIT: usize = 4 * (MAX_M - 1);

    let candidates =
        (isolated.shl_1() | isolated.shr_1() | isolated.shl_stride() | isolated.shr_stride())
            & valid_mask
            & board.plane(0);
    let mut deficit_bits = [Bitboard::ZERO; DEFICIT_BITS];
    for deficit in 1..MODULUS {
        let cells = isolated & board.plane(deficit as u8);
        for (bit, deficit_plane) in deficit_bits.iter_mut().enumerate() {
            if deficit & (1 << bit) != 0 {
                *deficit_plane |= cells;
            }
        }
    }

    let directional_deficits = [
        deficit_bits.map(|plane| plane.shl_1()),
        deficit_bits.map(|plane| plane.shr_1()),
        deficit_bits.map(|plane| plane.shl_stride()),
        deficit_bits.map(|plane| plane.shr_stride()),
    ];
    let mut score_bits = [Bitboard::ZERO; SCORE_BITS];
    for addend_bits in directional_deficits {
        let mut carry = Bitboard::ZERO;
        for (bit, accumulator) in score_bits.iter_mut().enumerate() {
            let addend = addend_bits.get(bit).copied().unwrap_or(Bitboard::ZERO);
            let previous = *accumulator;
            *accumulator = previous ^ addend ^ carry;
            carry = (previous & addend) | (previous & carry) | (addend & carry);
        }
        debug_assert!(carry.is_zero());
    }

    let mut remaining_budget = zero_hit_budget;
    let mut maximum_cover = 0;
    for weight in (1..=MAX_NEIGHBOR_DEFICIT).rev() {
        let mut matches = candidates;
        for (bit, score_plane) in score_bits.iter().enumerate() {
            matches &= if weight & (1 << bit) != 0 {
                *score_plane
            } else {
                !*score_plane
            };
        }
        let used = remaining_budget.min(matches.count_ones());
        maximum_cover += used * weight as u32;
        remaining_budget -= used;
        if remaining_budget == 0 {
            break;
        }
    }
    maximum_cover
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    fn precompute_bound(
        pieces: &[Piece],
        piece_order: &[usize],
        height: u8,
        width: u8,
    ) -> TotalDeficitBound {
        let placements = piece_order
            .iter()
            .map(|&piece_index| pieces[piece_index].placements(height, width))
            .collect::<Vec<_>>();
        TotalDeficitBound::precompute(pieces, piece_order, &placements, height, width)
    }

    fn bound(pieces: &[Piece], board: &Board) -> TotalDeficitBound {
        precompute_bound(
            pieces,
            &(0..pieces.len()).collect::<Vec<_>>(),
            board.height(),
            board.width(),
        )
    }

    #[test]
    fn precomputes_remaining_cells() {
        let single = Piece::from_grid(&[&[true]]);
        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![single, domino];
        let piece_order = vec![0, 1];

        let bound = precompute_bound(&pieces, &piece_order, 3, 3);

        assert_eq!(bound.remaining_cells(0), 3);
        assert_eq!(bound.remaining_cells(1), 2);
        assert_eq!(bound.remaining_cells(2), 0);
    }

    #[test]
    fn respects_solver_order() {
        let single = Piece::from_grid(&[&[true]]);
        let domino = Piece::from_grid(&[&[true, true]]);
        let tromino = Piece::from_grid(&[&[true, true, true]]);
        let pieces = vec![single, domino, tromino];
        let piece_order = vec![2, 0, 1];

        let bound = precompute_bound(&pieces, &piece_order, 3, 3);

        assert_eq!(bound.remaining_cells(0), 6);
        assert_eq!(bound.remaining_cells(1), 3);
        assert_eq!(bound.remaining_cells(2), 2);
    }

    #[test]
    fn allows_coverable_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 3);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino, domino];
        let piece_order = vec![0, 1];
        let bound = precompute_bound(&pieces, &piece_order, 3, 3);

        assert!(bound.allows::<2>(&board, 0));
    }

    #[test]
    fn rejects_uncoverable_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 5);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino];
        let piece_order = vec![0];
        let bound = precompute_bound(&pieces, &piece_order, 3, 3);

        assert!(!bound.allows::<2>(&board, 0));
    }

    #[test]
    fn allows_exact_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 2);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino];
        let piece_order = vec![0];
        let bound = precompute_bound(&pieces, &piece_order, 3, 3);

        assert!(bound.allows::<2>(&board, 0));
    }

    #[test]
    fn rejects_isolated_deficit_at_the_exact_boundary() {
        let board = Board::from_grid(&[&[0, 0, 0], &[0, 2, 0], &[0, 0, 0]], 3);
        let pieces = [Piece::from_grid(&[&[true, true]])];

        assert!(!bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn rejects_isolated_deficit_beyond_the_exact_boundary() {
        let board = Board::from_grid(
            &[
                &[2, 0, 0, 0, 2, 0, 0, 0, 0],
                &[0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[2, 0, 0, 0, 2, 0, 0, 0, 0],
            ],
            3,
        );
        let pieces = [
            Piece::from_grid(&[&[true, true, true, true, true]]),
            Piece::from_grid(&[&[true, true, true, true]]),
            Piece::from_grid(&[&[true, true]]),
        ];
        assert_eq!(board.total_deficit(), 8);

        assert!(!bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn rejects_pieces_that_cannot_share_the_only_zero_cell() {
        let board = Board::from_grid(&[&[0, 2, 0, 0, 0], &[0, 0, 0, 0, 0], &[0, 2, 0, 0, 0]], 3);
        let pieces = [
            Piece::from_grid(&[&[true, true, true, true, true]]),
            Piece::from_grid(&[&[true, true]]),
        ];
        assert_eq!(board.total_deficit(), 4);

        assert!(!bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn allows_nearby_isolated_cells_to_share_a_zero_cell() {
        let board = Board::from_grid(&[&[0, 0, 0], &[1, 0, 1], &[0, 0, 0]], 2);
        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = [domino, domino];

        assert!(bound(&pieces, &board).allows::<2>(&board, 0));
    }

    #[test]
    fn allows_one_placement_to_cover_nearby_isolated_cells() {
        let board = Board::from_grid(&[&[0, 0, 0], &[1, 0, 1], &[0, 0, 0]], 2);
        let pieces = [
            Piece::from_grid(&[&[true, true, true]]),
            Piece::from_grid(&[&[true]]),
        ];

        assert!(bound(&pieces, &board).allows::<2>(&board, 0));
    }

    #[test]
    fn leaves_disconnected_piece_states_to_other_bounds() {
        let board = Board::from_grid(&[&[1, 0, 0], &[0, 1, 0], &[0, 0, 0]], 3);
        let pieces = [Piece::from_grid(&[&[true, false], &[false, true]])];

        assert!(bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn sums_all_isolated_deficits_around_a_neighbor() {
        let board = Board::from_grid(
            &[
                &[0, 0, 0, 0, 0],
                &[0, 0, 1, 0, 0],
                &[0, 3, 0, 4, 0],
                &[0, 0, 2, 0, 0],
                &[0, 0, 0, 0, 0],
            ],
            5,
        );
        let isolated = board.valid_mask() & !board.plane(0);

        assert_eq!(
            singleton_neighbor_cover_upper_bound::<5>(&board, isolated, board.valid_mask(), 1),
            10
        );
    }

    fn assert_generated_states_are_allowed<const MODULUS: usize>(
        pieces: &[Piece],
        height: u8,
        width: u8,
    ) {
        fn visit<const MODULUS: usize>(
            bound: &TotalDeficitBound,
            placements: &[Vec<(usize, usize, Bitboard)>],
            board: Board,
            next_piece: usize,
            piece_index: usize,
        ) {
            if next_piece == placements.len() {
                assert!(
                    bound.allows::<MODULUS>(&board, piece_index),
                    "rejected generated solvable state: {board:?}, suffix={piece_index}"
                );
                return;
            }
            for &(_, _, mask) in &placements[next_piece] {
                let mut predecessor = board;
                predecessor.undo_piece(mask);
                visit::<MODULUS>(bound, placements, predecessor, next_piece + 1, piece_index);
            }
        }

        let placements = pieces
            .iter()
            .map(|piece| piece.placements(height, width))
            .collect::<Vec<_>>();
        let bound = precompute_bound(
            pieces,
            &(0..pieces.len()).collect::<Vec<_>>(),
            height,
            width,
        );
        for piece_index in 0..pieces.len() {
            visit::<MODULUS>(
                &bound,
                &placements,
                Board::new_solved(height, width, MODULUS as u8),
                piece_index,
                piece_index,
            );
        }
    }

    #[test]
    fn allows_exhaustively_generated_solvable_states() {
        let single = Piece::from_grid(&[&[true]]);
        let horizontal_domino = Piece::from_grid(&[&[true, true]]);
        let vertical_domino = Piece::from_grid(&[&[true], &[true]]);
        let corner = Piece::from_grid(&[&[true, true], &[true, false]]);
        let pieces = [corner, horizontal_domino, vertical_domino, single];

        assert_generated_states_are_allowed::<2>(&pieces, 3, 3);
        assert_generated_states_are_allowed::<3>(&pieces, 3, 3);
        assert_generated_states_are_allowed::<4>(&pieces, 3, 3);
        assert_generated_states_are_allowed::<5>(&pieces, 3, 3);
    }

    fn assert_sampled_states_are_allowed<const MODULUS: usize>(pieces: &[Piece]) {
        let height = 5;
        let width = 5;
        let placements = pieces
            .iter()
            .map(|piece| piece.placements(height, width))
            .collect::<Vec<_>>();
        let bound = precompute_bound(
            pieces,
            &(0..pieces.len()).collect::<Vec<_>>(),
            height,
            width,
        );

        let mut random_state = 0x4d59_5df4_d0f3_3173u64;
        for _ in 0..5_000 {
            random_state = random_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let piece_index = random_state as usize % pieces.len();
            let mut board = Board::new_solved(height, width, MODULUS as u8);

            for piece_placements in &placements[piece_index..] {
                random_state = random_state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let placement = piece_placements[random_state as usize % piece_placements.len()].2;
                board.undo_piece(placement);
            }

            assert!(
                bound.allows::<MODULUS>(&board, piece_index),
                "rejected sampled solvable state: {board:?}, suffix={piece_index}"
            );
        }
    }

    #[test]
    fn allows_sampled_solvable_states_with_larger_shapes() {
        let pieces = [
            Piece::from_grid(&[
                &[false, true, false],
                &[true, true, true],
                &[false, true, false],
            ]),
            Piece::from_grid(&[&[true, true, true, true, true]]),
            Piece::from_grid(&[&[true, true], &[true, false]]),
            Piece::from_grid(&[&[true, true], &[false, true]]),
            Piece::from_grid(&[&[true], &[true]]),
            Piece::from_grid(&[&[true, true]]),
            Piece::from_grid(&[&[true]]),
        ];

        assert_sampled_states_are_allowed::<2>(&pieces);
        assert_sampled_states_are_allowed::<3>(&pieces);
        assert_sampled_states_are_allowed::<4>(&pieces);
        assert_sampled_states_are_allowed::<5>(&pieces);
    }

    fn assert_shape_check_matches_brute_force<const MODULUS: usize>(pieces: &[Piece]) {
        let height = 5;
        let width = 5;
        let bound = precompute_bound(
            pieces,
            &(0..pieces.len()).collect::<Vec<_>>(),
            height,
            width,
        );
        let highest_deficit = MODULUS as u32 - 1;
        let mut random_state = 0xd1b5_4a32_d192_ed03u64;

        for _ in 0..1_000 {
            let mut grid = [[0u8; 5]; 5];
            for row in &mut grid {
                for cell in row {
                    random_state = random_state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    *cell = (random_state % MODULUS as u64) as u8;
                }
            }
            let rows = grid.iter().map(|row| row.as_slice()).collect::<Vec<_>>();
            let board = Board::from_grid(&rows, MODULUS as u8);
            let nonzero = board.valid_mask() & !board.plane(0);
            let neighbors =
                nonzero.shl_1() | nonzero.shr_1() | nonzero.shl_stride() | nonzero.shr_stride();
            let isolated = nonzero & !neighbors & board.plane(highest_deficit as u8);

            for piece_index in 0..pieces.len() {
                let single_cell_pieces = bound.remaining_single_cell_pieces[piece_index];
                if single_cell_pieces >= highest_deficit {
                    continue;
                }
                let required_multi_cell_pieces = highest_deficit - single_cell_pieces;
                let zero = board.plane(0);
                let mut expected = true;
                let mut cells = isolated;

                while !cells.is_zero() {
                    let cell = cells.lowest_set_bit();
                    cells.clear_bit(cell);
                    let cell_mask = Bitboard::from_bit(cell);
                    let mut candidate_zeros = zero
                        & (cell_mask.shl_1()
                            | cell_mask.shr_1()
                            | cell_mask.shl_stride()
                            | cell_mask.shr_stride());
                    let mut coverable = false;

                    while !candidate_zeros.is_zero() {
                        let zero_cell = candidate_zeros.lowest_set_bit();
                        candidate_zeros.clear_bit(zero_cell);
                        let zero_mask = Bitboard::from_bit(zero_cell);
                        let capable_pieces = bound.placement_masks[piece_index..]
                            .iter()
                            .filter(|placements| {
                                placements.iter().any(|placement| {
                                    !(*placement & cell_mask).is_zero()
                                        && (*placement & zero) == zero_mask
                                })
                            })
                            .count() as u32;
                        if capable_pieces >= required_multi_cell_pieces {
                            coverable = true;
                            break;
                        }
                    }
                    if !coverable {
                        expected = false;
                        break;
                    }
                }

                assert_eq!(
                    bound.highest_deficit_singletons_are_coverable(
                        &board,
                        isolated,
                        piece_index,
                        required_multi_cell_pieces,
                    ),
                    expected,
                    "mismatch: {board:?}, suffix={piece_index}"
                );
            }
        }
    }

    #[test]
    fn shape_check_matches_brute_force() {
        let pieces = [
            Piece::from_grid(&[
                &[false, true, false],
                &[true, true, true],
                &[false, true, false],
            ]),
            Piece::from_grid(&[&[true, true, true, true, true]]),
            Piece::from_grid(&[&[true, true], &[true, false]]),
            Piece::from_grid(&[&[true, true], &[false, true]]),
            Piece::from_grid(&[&[true], &[true]]),
            Piece::from_grid(&[&[true, true]]),
            Piece::from_grid(&[&[true]]),
        ];

        assert_shape_check_matches_brute_force::<2>(&pieces);
        assert_shape_check_matches_brute_force::<3>(&pieces);
        assert_shape_check_matches_brute_force::<4>(&pieces);
        assert_shape_check_matches_brute_force::<5>(&pieces);
    }
}
