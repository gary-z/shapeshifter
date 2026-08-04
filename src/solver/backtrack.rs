use std::cell::Cell;

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

use super::prune::mc::HitCounter;
use super::pruning::*;
use super::{PruningConfig, SolverData};

/// Order the placements worth trying at this node, dropping the rest.
///
/// Primary key = number of zero-deficit cells hit (fewer first), secondary =
/// higher total deficit of covered cells. Placements hitting more than
/// `max_zeros` zero cells cannot lead to a solution — the child's deficit
/// would exceed what the remaining pieces can cover — so they are dropped
/// during bucketing rather than emitted and rejected later. That is where the
/// bulk of a node's work used to go: over 90% of placements fail this budget
/// on tight boards, and each one still paid for a sort key, an insertion-sort
/// slot, and a filter call.
///
/// Returns the number of surviving placements, written to `order[..returned]`.
pub(crate) fn sort_placements(
    board: &Board,
    m: u8,
    placements: &[(usize, usize, Bitboard)],
    max_zeros: u32,
    order: &mut [u8; 196],
) -> usize {
    let pl_len = placements.len();
    let zero_plane = board.plane(0);
    // A piece has at most 25 cells, so it can hit at most 25 zero cells.
    let cap = max_zeros.min(25) as usize;

    // Pass 1: primary key only (one popcount per placement) + bucket histogram.
    let mut zeros = [0u8; 196];
    let mut counts = [0u8; 26];
    for i in 0..pl_len {
        let z = (placements[i].2 & zero_plane).count_ones() as usize;
        zeros[i] = z as u8;
        if z <= cap {
            counts[z] += 1;
        }
    }

    let mut offsets = [0u8; 27];
    for b in 0..=cap {
        offsets[b + 1] = offsets[b] + counts[b];
    }
    let kept = offsets[cap + 1] as usize;
    if kept == 0 {
        return 0;
    }

    // Pass 2: secondary key and bucket placement, survivors only.
    let mut keys = [0u8; 196];
    let mut cursor = offsets;
    for i in 0..pl_len {
        let z = zeros[i] as usize;
        if z > cap {
            continue;
        }
        let mask = placements[i].2;
        let mut deficit_sum = 0u16;
        for d in 1..m {
            deficit_sum += d as u16 * (mask & board.plane(d)).count_ones() as u16;
        }
        keys[i] = (255 - deficit_sum.min(255)) as u8;
        order[cursor[z] as usize] = i as u8;
        cursor[z] += 1;
    }

    // Insertion sort within each bucket (zeros is constant there, so the
    // secondary key alone decides the order).
    for b in 0..=cap {
        let start = offsets[b] as usize;
        let end = offsets[b + 1] as usize;
        for i in start + 1..end {
            let val = order[i];
            let ki = keys[val as usize];
            let mut j = i;
            while j > start && keys[order[j - 1] as usize] > ki {
                order[j] = order[j - 1];
                j -= 1;
            }
            order[j] = val;
        }
    }

    kept
}

/// Try to solve remaining pieces when they're all 1x1.
pub(crate) fn solve_single_cells(
    board: &Board,
    m: u8,
    h: u8,
    w: u8,
    num_pieces: usize,
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    let mut needed = 0u32;
    for d in 1..m {
        needed += d as u32 * board.plane(d).count_ones();
    }
    let n = num_pieces as u32;
    if n < needed || (n - needed) % m as u32 != 0 {
        return false;
    }
    let extra_wraps = (n - needed) / m as u32;

    let base_len = solution.len();
    for r in 0..h as usize {
        for c in 0..w as usize {
            let deficit = board.get(r, c) as usize;
            if deficit != 0 {
                for _ in 0..deficit {
                    solution.push((r, c));
                }
            }
        }
    }

    for _ in 0..extra_wraps {
        for _ in 0..m {
            solution.push((0, 0));
        }
    }

    debug_assert_eq!(solution.len() - base_len, num_pieces);
    true
}

#[inline]
pub(crate) fn next_prev_placement(data: &SolverData, piece_idx: usize, pl_idx: usize) -> usize {
    let next = piece_idx + 1;
    if next < data.all_placements.len() && data.skip_tables[next].is_some() { pl_idx } else { usize::MAX }
}

/// Serial backtracker. Recursively tries all placements for each piece,
/// pruning infeasible branches via MC bounds and deterministic checks.
pub(crate) fn backtrack<const M: usize>(
    board: &Board,
    hits: HitCounter,
    data: &SolverData,
    piece_idx: usize,
    prev_placement: usize,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    config: &PruningConfig,
    exhaustive: bool,
) -> bool {
    if piece_idx == data.all_placements.len() {
        return board.is_solved();
    }

    if config.single_cell_endgame && piece_idx >= data.single_cell_start {
        let num_remaining = data.all_placements.len() - piece_idx;
        return solve_single_cells(board, data.m, data.h, data.w, num_remaining, solution);
    }

    if !prune_node::<M>(board, data, piece_idx, config) { return false; }

    let placements = &data.all_placements[piece_idx];
    let mut order = [0u8; 196];
    let max_zeros = max_zeros_hit::<M>(board, data, piece_idx);
    let kept = sort_placements(board, data.m, placements, max_zeros, &mut order);

    let mut found = false;
    // Depth of `solution` on entry. The single-cell endgame pushes one entry
    // per remaining piece, so unwinding truncates rather than pops.
    let base_len = solution.len();
    // Exhaustive mode keeps searching after a hit, so the first solution found
    // is stashed and restored on the way out.
    let mut found_solution: Option<Vec<(usize, usize)>> = None;

    for oi in 0..kept {
        let pl_idx = order[oi] as usize;
        let mask = placements[pl_idx].2;
        nodes.set(nodes.get() + 1);

        if !filter_placement(data, piece_idx, pl_idx, prev_placement) {
            continue;
        }

        let mut board = *board;
        board.apply_piece(mask);

        let mut new_hits = hits;
        new_hits.apply_piece(mask);
        if data.mc_prune.exceeds_hit_threshold(&new_hits, piece_idx + 1) {
            continue;
        }

        solution.push((placements[pl_idx].0, placements[pl_idx].1));

        let next_prev = next_prev_placement(data, piece_idx, pl_idx);

        if backtrack::<M>(
            &board,
            new_hits,
            data,
            piece_idx + 1,
            next_prev,
            solution,
            nodes,
            config,
            exhaustive,
        ) {
            if !exhaustive {
                return true;
            }
            if found_solution.is_none() {
                found_solution = Some(solution.clone());
            }
            found = true;
        }

        solution.truncate(base_len);
    }

    if let Some(sol) = found_solution {
        solution.clear();
        solution.extend_from_slice(&sol);
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::Piece;

    /// Zero cells hit and total covered deficit for a placement — the two
    /// sort keys, recomputed independently of `sort_placements`.
    fn keys_of(board: &Board, m: u8, mask: Bitboard) -> (u32, u32) {
        let zeros = (mask & board.plane(0)).count_ones();
        let mut deficit = 0u32;
        for d in 1..m {
            deficit += d as u32 * (mask & board.plane(d)).count_ones();
        }
        (zeros, deficit)
    }

    #[test]
    fn keeps_everything_when_budget_is_generous() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 1, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let pl = Piece::from_grid(&[&[true, true]]).placements(3, 3);

        let mut order = [0u8; 196];
        let kept = sort_placements(&board, 2, &pl, u32::MAX, &mut order);
        assert_eq!(kept, pl.len());

        let mut seen: Vec<u8> = order[..kept].to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), pl.len(), "every placement emitted exactly once");
    }

    #[test]
    fn emits_budget_prefix_in_sorted_order() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 1, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let pl = Piece::from_grid(&[&[true, true]]).placements(3, 3);

        for budget in 0..=2u32 {
            let mut order = [0u8; 196];
            let kept = sort_placements(&board, 2, &pl, budget, &mut order);

            // Exactly the placements the zero-hit budget allows, no more.
            let expected = pl
                .iter()
                .filter(|&&(_, _, mask)| keys_of(&board, 2, mask).0 <= budget)
                .count();
            assert_eq!(kept, expected, "budget {budget}");

            // Sorted by zeros ascending, then covered deficit descending.
            let mut prev = (0u32, u32::MAX);
            for &oi in &order[..kept] {
                let k = keys_of(&board, 2, pl[oi as usize].2);
                assert!(k.0 <= budget, "budget {budget}: over-budget placement emitted");
                assert!(
                    k.0 > prev.0 || (k.0 == prev.0 && k.1 <= prev.1),
                    "budget {budget}: order violated at {k:?} after {prev:?}"
                );
                prev = k;
            }
        }
    }

    #[test]
    fn zero_budget_on_a_full_board_keeps_nothing() {
        // Every cell is 0, so any placement wraps cells the budget forbids.
        let board = Board::new_solved(3, 3, 3);
        let pl = Piece::from_grid(&[&[true]]).placements(3, 3);
        let mut order = [0u8; 196];
        assert_eq!(sort_placements(&board, 3, &pl, 0, &mut order), 0);
    }
}
