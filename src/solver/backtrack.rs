use std::cell::Cell;

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

use super::SolverData;
use super::pruning::{is_canonical_placement_pair, max_zero_cells_allowed, state_is_feasible};

pub(super) const MAX_PLACEMENTS: usize = 196;
const MAX_PIECE_CELLS: usize = 25;

pub(super) struct AnchorPlacementData {
    cell_offsets: Box<[u8]>,
    anchor_mask: Bitboard,
    placements_per_row: usize,
}

impl AnchorPlacementData {
    pub(super) fn precompute(placements: &[(usize, usize, Bitboard)]) -> Self {
        let mut anchor_mask = Bitboard::ZERO;
        for &(row, column, _) in placements {
            anchor_mask.set_bit((row * crate::core::STRIDE + column) as u32);
        }

        let placements_per_row = placements
            .iter()
            .take_while(|&&(row, _, _)| row == 0)
            .count();
        let mut cell_offsets = Vec::new();
        if let Some(&(_, _, first_mask)) = placements.first() {
            let mut cells = first_mask;
            while !cells.is_zero() {
                let cell = cells.lowest_set_bit();
                cells.clear_bit(cell);
                cell_offsets.push(cell as u8);
            }
        }

        Self {
            cell_offsets: cell_offsets.into_boxed_slice(),
            anchor_mask,
            placements_per_row,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct SearchPosition {
    pub(super) board: Board,
    pub(super) piece_index: usize,
    pub(super) previous_placement: usize,
}

/// Rank placements by zero cells hit, then by covered deficit.
/// Placements outside the zero-cell budget are omitted.
pub(super) fn rank_placements(
    board: &Board,
    modulus: u8,
    placements: &[(usize, usize, Bitboard)],
    max_zero_cells: u32,
    ranked_indices: &mut [u8; MAX_PLACEMENTS],
) -> usize {
    let placement_count = placements.len();
    let zero_plane = board.plane(0);
    let cap = max_zero_cells.min(MAX_PIECE_CELLS as u32) as usize;

    let mut zero_counts = [0u8; MAX_PLACEMENTS];
    let mut bucket_counts = [0u8; MAX_PIECE_CELLS + 1];
    for placement_index in 0..placement_count {
        let zero_count = (placements[placement_index].2 & zero_plane).count_ones() as usize;
        zero_counts[placement_index] = zero_count as u8;
        if zero_count <= cap {
            bucket_counts[zero_count] += 1;
        }
    }

    let mut offsets = [0u8; MAX_PIECE_CELLS + 2];
    for bucket in 0..=cap {
        offsets[bucket + 1] = offsets[bucket] + bucket_counts[bucket];
    }
    let ranked_count = offsets[cap + 1] as usize;
    if ranked_count == 0 {
        return 0;
    }

    let mut deficit_keys = [0u8; MAX_PLACEMENTS];
    let mut cursor = offsets;
    for placement_index in 0..placement_count {
        let zero_count = zero_counts[placement_index] as usize;
        if zero_count > cap {
            continue;
        }
        let mask = placements[placement_index].2;
        let mut deficit_sum = 0u16;
        for deficit in 1..modulus {
            deficit_sum += deficit as u16 * (mask & board.plane(deficit)).count_ones() as u16;
        }
        deficit_keys[placement_index] = (255 - deficit_sum.min(255)) as u8;
        ranked_indices[cursor[zero_count] as usize] = placement_index as u8;
        cursor[zero_count] += 1;
    }

    for bucket in 0..=cap {
        let start = offsets[bucket] as usize;
        let end = offsets[bucket + 1] as usize;
        for unsorted_index in start + 1..end {
            let placement_index = ranked_indices[unsorted_index];
            let key = deficit_keys[placement_index as usize];
            let mut insertion_index = unsorted_index;
            while insertion_index > start
                && deficit_keys[ranked_indices[insertion_index - 1] as usize] > key
            {
                ranked_indices[insertion_index] = ranked_indices[insertion_index - 1];
                insertion_index -= 1;
            }
            ranked_indices[insertion_index] = placement_index;
        }
    }

    ranked_count
}

/// Rank placements by deriving zero-overlap buckets for all top-left anchors
/// together. This avoids scanning and popcounting every placement while the
/// global zero-cell budget is small.
pub(super) fn rank_low_zero_hit_placements(
    board: &Board,
    modulus: u8,
    placements: &[(usize, usize, Bitboard)],
    anchors: &AnchorPlacementData,
    max_zero_cells: u32,
    ranked_indices: &mut [u8; MAX_PLACEMENTS],
) -> usize {
    debug_assert!(max_zero_cells <= 2);
    if anchors.placements_per_row == 0 {
        return 0;
    }

    let cap = max_zero_cells as usize;
    if cap == 0 {
        let nonzero = !board.plane(0);
        let mut valid_anchors = anchors.anchor_mask;
        for &cell_offset in &anchors.cell_offsets {
            valid_anchors &= nonzero.shr_shape_offset(u32::from(cell_offset));
            if valid_anchors.is_zero() {
                return 0;
            }
        }

        let mut deficit_keys = [0u8; MAX_PLACEMENTS];
        let mut ranked_count = 0usize;
        while !valid_anchors.is_zero() {
            let anchor = valid_anchors.lowest_set_bit();
            valid_anchors.clear_bit(anchor);
            let row = anchor as usize / crate::core::STRIDE;
            let column = anchor as usize % crate::core::STRIDE;
            let placement_index = row * anchors.placements_per_row + column;
            let mask = placements[placement_index].2;

            let mut deficit_sum = 0u16;
            for deficit in 1..modulus {
                deficit_sum += deficit as u16 * (mask & board.plane(deficit)).count_ones() as u16;
            }
            deficit_keys[placement_index] = (255 - deficit_sum.min(255)) as u8;
            ranked_indices[ranked_count] = placement_index as u8;
            ranked_count += 1;
        }

        sort_anchor_bucket(ranked_indices, &deficit_keys, 0, ranked_count);
        return ranked_count;
    }

    let zero = board.plane(0);
    let mut at_least_one = Bitboard::ZERO;
    let mut at_least_two = Bitboard::ZERO;
    let mut at_least_three = Bitboard::ZERO;
    for &cell_offset in &anchors.cell_offsets {
        let hit = zero.shr_shape_offset(u32::from(cell_offset));
        if cap == 2 {
            at_least_three |= at_least_two & hit;
        }
        at_least_two |= at_least_one & hit;
        at_least_one |= hit;

        let over_budget = if cap == 1 {
            at_least_two
        } else {
            at_least_three
        };
        if (anchors.anchor_mask & !over_budget).is_zero() {
            return 0;
        }
    }

    let exact = [
        anchors.anchor_mask & !at_least_one,
        anchors.anchor_mask & at_least_one & !at_least_two,
        anchors.anchor_mask & at_least_two & !at_least_three,
    ];
    let mut offsets = [0u8; 4];
    for bucket in 0..=cap {
        offsets[bucket + 1] = offsets[bucket] + exact[bucket].count_ones() as u8;
    }
    let ranked_count = offsets[cap + 1] as usize;
    if ranked_count == 0 {
        return 0;
    }

    let mut deficit_keys = [0u8; MAX_PLACEMENTS];
    let mut cursor = offsets;
    for bucket in 0..=cap {
        let mut valid_anchors = exact[bucket];
        while !valid_anchors.is_zero() {
            let anchor = valid_anchors.lowest_set_bit();
            valid_anchors.clear_bit(anchor);
            let row = anchor as usize / crate::core::STRIDE;
            let column = anchor as usize % crate::core::STRIDE;
            let placement_index = row * anchors.placements_per_row + column;
            let mask = placements[placement_index].2;

            let mut deficit_sum = 0u16;
            for deficit in 1..modulus {
                deficit_sum += deficit as u16 * (mask & board.plane(deficit)).count_ones() as u16;
            }
            deficit_keys[placement_index] = (255 - deficit_sum.min(255)) as u8;
            ranked_indices[cursor[bucket] as usize] = placement_index as u8;
            cursor[bucket] += 1;
        }
    }

    for bucket in 0..=cap {
        sort_anchor_bucket(
            ranked_indices,
            &deficit_keys,
            offsets[bucket] as usize,
            offsets[bucket + 1] as usize,
        );
    }
    ranked_count
}

#[inline]
fn sort_anchor_bucket(
    ranked_indices: &mut [u8; MAX_PLACEMENTS],
    deficit_keys: &[u8; MAX_PLACEMENTS],
    start: usize,
    end: usize,
) {
    for unsorted_index in start + 1..end {
        let placement_index = ranked_indices[unsorted_index];
        let key = deficit_keys[placement_index as usize];
        let mut insertion_index = unsorted_index;
        while insertion_index > start
            && deficit_keys[ranked_indices[insertion_index - 1] as usize] > key
        {
            ranked_indices[insertion_index] = ranked_indices[insertion_index - 1];
            insertion_index -= 1;
        }
        ranked_indices[insertion_index] = placement_index;
    }
}

pub(super) fn solve_single_cell_suffix(
    board: &Board,
    modulus: u8,
    height: u8,
    width: u8,
    available_pieces: usize,
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    let mut required_hits = 0u32;
    for deficit in 1..modulus {
        required_hits += deficit as u32 * board.plane(deficit).count_ones();
    }
    let available_pieces = available_pieces as u32;
    if available_pieces < required_hits
        || !(available_pieces - required_hits).is_multiple_of(modulus as u32)
    {
        return false;
    }
    let extra_wraps = (available_pieces - required_hits) / modulus as u32;

    let initial_solution_length = solution.len();
    for row in 0..height as usize {
        for column in 0..width as usize {
            let deficit = board.get(row, column) as usize;
            if deficit != 0 {
                for _ in 0..deficit {
                    solution.push((row, column));
                }
            }
        }
    }

    for _ in 0..extra_wraps {
        for _ in 0..modulus {
            solution.push((0, 0));
        }
    }

    debug_assert_eq!(
        solution.len() - initial_solution_length,
        available_pieces as usize
    );
    true
}

#[inline]
pub(super) fn next_previous_placement(
    data: &SolverData,
    piece_index: usize,
    placement_index: usize,
) -> usize {
    let next_piece_index = piece_index + 1;
    if next_piece_index < data.placements.len()
        && data.equivalent_pair_skips[next_piece_index].is_some()
    {
        placement_index
    } else {
        usize::MAX
    }
}

pub(super) fn backtrack<const MODULUS: usize>(
    position: SearchPosition,
    data: &SolverData,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    exhaustive: bool,
) -> bool {
    let SearchPosition {
        board,
        piece_index,
        previous_placement,
    } = position;

    if piece_index == data.placements.len() {
        return board.is_solved();
    }

    if piece_index >= data.single_cell_suffix_start {
        let remaining_pieces = data.placements.len() - piece_index;
        return solve_single_cell_suffix(
            &board,
            data.modulus,
            data.height,
            data.width,
            remaining_pieces,
            solution,
        );
    }

    if !state_is_feasible::<MODULUS>(&board, data, piece_index) {
        return false;
    }

    let placements = &data.placements[piece_index];
    let mut ranked_indices = [0u8; MAX_PLACEMENTS];
    let max_zero_cells = max_zero_cells_allowed::<MODULUS>(&board, data, piece_index);
    let candidate_count = if max_zero_cells <= 2 {
        rank_low_zero_hit_placements(
            &board,
            data.modulus,
            placements,
            &data.anchor_placements[piece_index],
            max_zero_cells,
            &mut ranked_indices,
        )
    } else {
        rank_placements(
            &board,
            data.modulus,
            placements,
            max_zero_cells,
            &mut ranked_indices,
        )
    };

    let mut found = false;
    let solution_length_on_entry = solution.len();
    let mut first_solution: Option<Vec<(usize, usize)>> = None;

    for &ranked_index in &ranked_indices[..candidate_count] {
        let placement_index = ranked_index as usize;
        let mask = placements[placement_index].2;
        nodes.set(nodes.get() + 1);

        if !is_canonical_placement_pair(data, piece_index, placement_index, previous_placement) {
            continue;
        }

        let mut board_after_placement = board;
        board_after_placement.apply_piece(mask);

        solution.push((placements[placement_index].0, placements[placement_index].1));

        let previous_placement = next_previous_placement(data, piece_index, placement_index);

        if backtrack::<MODULUS>(
            SearchPosition {
                board: board_after_placement,
                piece_index: piece_index + 1,
                previous_placement,
            },
            data,
            solution,
            nodes,
            exhaustive,
        ) {
            if !exhaustive {
                return true;
            }
            if first_solution.is_none() {
                first_solution = Some(solution.clone());
            }
            found = true;
        }

        solution.truncate(solution_length_on_entry);
    }

    if let Some(first_solution) = first_solution {
        solution.clear();
        solution.extend_from_slice(&first_solution);
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::Piece;

    fn placement_keys(board: &Board, modulus: u8, mask: Bitboard) -> (u32, u32) {
        let zero_count = (mask & board.plane(0)).count_ones();
        let mut deficit = 0u32;
        for cell_deficit in 1..modulus {
            deficit += cell_deficit as u32 * (mask & board.plane(cell_deficit)).count_ones();
        }
        (zero_count, deficit)
    }

    #[test]
    fn keeps_everything_when_budget_is_generous() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 1, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let placements = Piece::from_grid(&[&[true, true]]).placements(3, 3);

        let mut ranked = [0u8; MAX_PLACEMENTS];
        let ranked_count = rank_placements(&board, 2, &placements, u32::MAX, &mut ranked);
        assert_eq!(ranked_count, placements.len());

        let mut seen: Vec<u8> = ranked[..ranked_count].to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            placements.len(),
            "every placement emitted exactly once"
        );
    }

    #[test]
    fn emits_budget_prefix_in_sorted_order() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 1, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let placements = Piece::from_grid(&[&[true, true]]).placements(3, 3);

        for budget in 0..=2u32 {
            let mut ranked = [0u8; MAX_PLACEMENTS];
            let ranked_count = rank_placements(&board, 2, &placements, budget, &mut ranked);

            let expected = placements
                .iter()
                .filter(|&&(_, _, mask)| placement_keys(&board, 2, mask).0 <= budget)
                .count();
            assert_eq!(ranked_count, expected, "budget {budget}");

            let mut previous_key = (0u32, u32::MAX);
            for &placement_index in &ranked[..ranked_count] {
                let key = placement_keys(&board, 2, placements[placement_index as usize].2);
                assert!(
                    key.0 <= budget,
                    "budget {budget}: over-budget placement emitted"
                );
                assert!(
                    key.0 > previous_key.0 || (key.0 == previous_key.0 && key.1 <= previous_key.1),
                    "budget {budget}: order violated at {key:?} after {previous_key:?}"
                );
                previous_key = key;
            }
        }
    }

    #[test]
    fn zero_budget_on_a_solved_board_keeps_nothing() {
        let board = Board::new_solved(3, 3, 3);
        let placements = Piece::from_grid(&[&[true]]).placements(3, 3);
        let mut ranked = [0u8; MAX_PLACEMENTS];
        assert_eq!(rank_placements(&board, 3, &placements, 0, &mut ranked), 0);
    }

    #[test]
    fn low_budget_anchor_rank_matches_generic_rank() {
        let pieces = [
            Piece::from_grid(&[
                &[false, true, false],
                &[true, true, true],
                &[false, true, false],
            ]),
            Piece::from_grid(&[&[true, true, true, true, true]]),
            Piece::from_grid(&[&[true, true], &[true, false]]),
            Piece::from_grid(&[&[true], &[true]]),
        ];
        let mut random_state = 0x243f_6a88_85a3_08d3u64;
        for modulus in 2..=5u8 {
            for _ in 0..1_000 {
                let mut grid = [[0u8; 5]; 5];
                for row in &mut grid {
                    for cell in row {
                        random_state = random_state
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1_442_695_040_888_963_407);
                        *cell = (random_state % u64::from(modulus)) as u8;
                    }
                }
                let rows = grid.iter().map(|row| row.as_slice()).collect::<Vec<_>>();
                let board = Board::from_grid(&rows, modulus);

                for piece in &pieces {
                    let placements = piece.placements(5, 5);
                    let anchors = AnchorPlacementData::precompute(&placements);
                    for budget in 0..=2 {
                        let mut expected = [0u8; MAX_PLACEMENTS];
                        let expected_count =
                            rank_placements(&board, modulus, &placements, budget, &mut expected);
                        let mut actual = [0u8; MAX_PLACEMENTS];
                        let actual_count = rank_low_zero_hit_placements(
                            &board,
                            modulus,
                            &placements,
                            &anchors,
                            budget,
                            &mut actual,
                        );
                        assert_eq!(
                            &actual[..actual_count],
                            &expected[..expected_count],
                            "mismatch for M={modulus}, budget={budget}, piece={piece:?}"
                        );
                    }
                }
            }
        }
    }
}
