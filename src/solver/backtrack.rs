use std::cell::Cell;

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

use super::SolverData;
use super::pruning::HitCounter;
use super::pruning::{is_canonical_placement_pair, max_zero_cells_allowed, state_is_feasible};

pub(super) const MAX_PLACEMENTS: usize = 196;
const MAX_PIECE_CELLS: usize = 25;

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
    if available_pieces < required_hits || (available_pieces - required_hits) % modulus as u32 != 0
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
    board: &Board,
    hits: HitCounter,
    data: &SolverData,
    piece_index: usize,
    previous_placement: usize,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    exhaustive: bool,
) -> bool {
    if piece_index == data.placements.len() {
        return board.is_solved();
    }

    if piece_index >= data.single_cell_suffix_start {
        let remaining_pieces = data.placements.len() - piece_index;
        return solve_single_cell_suffix(
            board,
            data.modulus,
            data.height,
            data.width,
            remaining_pieces,
            solution,
        );
    }

    if !state_is_feasible::<MODULUS>(board, data, piece_index) {
        return false;
    }

    let placements = &data.placements[piece_index];
    let mut ranked_indices = [0u8; MAX_PLACEMENTS];
    let max_zero_cells = max_zero_cells_allowed::<MODULUS>(board, data, piece_index);
    let candidate_count = rank_placements(
        board,
        data.modulus,
        placements,
        max_zero_cells,
        &mut ranked_indices,
    );

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

        let mut board = *board;
        board.apply_piece(mask);

        let mut hits_after_placement = hits;
        hits_after_placement.apply_piece(mask);
        if data
            .monte_carlo
            .exceeds_hit_threshold(&hits_after_placement, piece_index + 1)
        {
            continue;
        }

        solution.push((placements[placement_index].0, placements[placement_index].1));

        let previous_placement = next_previous_placement(data, piece_index, placement_index);

        if backtrack::<MODULUS>(
            &board,
            hits_after_placement,
            data,
            piece_index + 1,
            previous_placement,
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
}
