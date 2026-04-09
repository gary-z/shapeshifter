use std::cell::Cell;

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

use super::prune::mc::HitCounter;
use super::pruning::*;
use super::{PruningConfig, SolverData};

/// Sort placements by: primary = fewer zero-deficit cells hit,
/// secondary = higher total deficit of covered cells.
/// Both computed cheaply via bitboard popcount.
pub(crate) fn sort_placements(
    board: &Board,
    m: u8,
    placements: &[(usize, usize, Bitboard)],
    order: &mut [u8; 196],
) {
    let pl_len = placements.len();
    let zero_plane = board.plane(0);
    let mut keys = [0u16; 196];
    for i in 0..pl_len {
        let mask = placements[i].2;
        let zeros = (mask & zero_plane).count_ones() as u16;
        let mut deficit_sum = 0u16;
        for d in 1..m {
            deficit_sum += d as u16 * (mask & board.plane(d)).count_ones() as u16;
        }
        keys[i] = zeros * 256 + (255 - deficit_sum.min(255));
    }
    let mut counts = [0u8; 26];
    for i in 0..pl_len { counts[(keys[i] >> 8) as usize] += 1; }
    let mut offsets = [0u8; 26];
    for i in 1..26 { offsets[i] = offsets[i - 1] + counts[i - 1]; }
    for i in 0..pl_len {
        let k = (keys[i] >> 8) as usize;
        order[offsets[k] as usize] = i as u8;
        offsets[k] += 1;
    }
    let mut start = 0usize;
    for b in 0..26 {
        let end = if b < 25 { offsets[b] as usize } else { pl_len };
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
        start = end;
    }
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
pub(crate) fn backtrack(
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

    if !prune_node(board, data, piece_idx, config) { return false; }

    let placements = &data.all_placements[piece_idx];
    let pl_len = placements.len();
    let mut order = [0u8; 196];
    sort_placements(board, data.m, placements, &mut order);
    let fs = filter_state(board, data, piece_idx);

    let mut found = false;

    for oi in 0..pl_len {
        let pl_idx = order[oi] as usize;
        let mask = placements[pl_idx].2;
        nodes.set(nodes.get() + 1);

        if !filter_placement(data, piece_idx, pl_idx, mask, prev_placement, &fs) {
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

        if backtrack(
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
            found = true;
        }

        solution.pop();
    }

    found
}
