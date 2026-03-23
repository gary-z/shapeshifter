use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bitboard::Bitboard;
use crate::board::Board;

use super::pruning::*;
use super::{PruningConfig, SolverData};

/// Try to solve remaining pieces when they're all 1x1.
/// Each cell at value d needs (M-d)%M hits. Total hits must equal number of pieces.
/// Returns true and fills solution if solvable.
pub(crate) fn solve_single_cells(
    board: &Board,
    m: u8,
    h: u8,
    w: u8,
    num_pieces: usize,
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    // Count total hits needed and verify it matches available pieces.
    let mut needed = 0u32;
    for d in 1..m {
        needed += (m - d) as u32 * board.plane(d).count_ones();
    }
    if needed as usize != num_pieces {
        return false;
    }

    // Assign pieces to cells: for each non-zero cell, emit (M-d) placements.
    // Process cells in row-major order.
    let base_len = solution.len();
    for r in 0..h as usize {
        for c in 0..w as usize {
            let val = board.get(r, c);
            if val != 0 {
                let hits = (m - val) as usize;
                for _ in 0..hits {
                    solution.push((r, c));
                }
            }
        }
    }

    debug_assert_eq!(solution.len() - base_len, num_pieces);
    true
}

/// Generate backtrack functions with and without abort support.
/// This macro avoids code duplication while keeping the serial path
/// free of any abort-related overhead (no extra parameter, no branch).
macro_rules! define_backtrack {
    ($name:ident $(, abort: $abort_param:ident : $abort_ty:ty)?) => {
        pub(crate) fn $name(
            board: &Board,
            data: &SolverData,
            piece_idx: usize,
            min_placement: usize,
            prev_dup_placement: usize,
            solution: &mut Vec<(usize, usize)>,
            nodes: &Cell<u64>,
            config: &PruningConfig,
            $($abort_param: $abort_ty,)?
        ) -> bool {
            nodes.set(nodes.get() + 1);

            $(
                // Check abort every 1024 nodes.
                if nodes.get() & 1023 == 0 && $abort_param.load(Ordering::Relaxed) {
                    return false;
                }
            )?

            if piece_idx == data.all_placements.len() {
                return board.is_solved();
            }

            // If all remaining pieces are 1x1, solve directly.
            if config.single_cell_endgame && piece_idx >= data.single_cell_start {
                let num_remaining = data.all_placements.len() - piece_idx;
                return solve_single_cells(board, data.m, data.h, data.w, num_remaining, solution);
            }

            let remaining = data.all_placements.len() - piece_idx;
            let branching = data.all_placements[piece_idx].len();

            if config.active_planes && !prune_active_planes(board, remaining) { return false; }
            if config.min_flips_global && !prune_min_flips_global(board, data, piece_idx) { return false; }
            if config.min_flips_rowcol && !prune_line_families_rowcol(board, data, piece_idx) { return false; }
            if config.min_flips_diagonal && !prune_line_families_diagonal(board, data, piece_idx) { return false; }
            if config.min_flips_rowcol && branching >= 6 && !prune_subgrid(board, data, piece_idx, remaining) { return false; }
            if config.coverage && !prune_coverage(board, data, piece_idx) { return false; }
            if config.jaggedness && !prune_jaggedness(board, data, piece_idx) { return false; }
            if config.min_flips_global && !prune_parity_partitions(board, data, piece_idx) { return false; }
            if config.min_flips_global && !prune_subset_reachability(board, data, piece_idx) { return false; }
            if config.min_flips_global && !prune_weight_tuples(board, data, piece_idx) { return false; }

            // Compute locked mask: cells at 0 where remaining coverage < M.
            let locked_mask = if config.cell_locking {
                board.plane(0) & !data.suffix_coverage[piece_idx].coverage_ge(data.m)
            } else {
                Bitboard::ZERO
            };

            // Prune: per-component checks (jaggedness, min_flips).
            // Run when branching factor justifies flood-fill cost.
            // Component checks (flood-fill + per-component jaggedness/min_flips) disabled:
            // profiling shows 12% of instructions but <0.1% node reduction on real puzzles.
            // The per-component bounds are too loose with large pieces on medium boards.
            // Keeping the code for potential future use on larger boards.
            if false && config.component_checks {
                if !check_components(board, locked_mask, data, piece_idx) {
                    return false;
                }
            }

            let placements = &data.all_placements[piece_idx];

            // Order placements by zeros hit ascending using counting sort.
            // Keys are small (0..=max_piece_area), so O(n) bucket sort beats O(n^2) insertion sort.
            let zero_plane = board.plane(0);
            let pl_len = placements.len();
            let mut order = [0u8; 196];
            let mut keys = [0u8; 196];
            for i in 0..pl_len {
                keys[i] = (placements[i].2 & zero_plane).count_ones() as u8;
            }
            // Counting sort: count occurrences, then build order from buckets.
            let mut counts = [0u8; 26]; // max piece area is 25 (5x5)
            for i in 0..pl_len { counts[keys[i] as usize] += 1; }
            let mut offsets = [0u8; 26];
            for i in 1..26 { offsets[i] = offsets[i - 1] + counts[i - 1]; }
            for i in 0..pl_len {
                let k = keys[i] as usize;
                order[offsets[k] as usize] = i as u8;
                offsets[k] += 1;
            }

            let mut board = board.clone();
            for oi in 0..pl_len {
                let pl_idx = order[oi] as usize;
                let (row, col, mask) = placements[pl_idx];
                // Duplicate symmetry breaking.
                if config.duplicate_pruning && pl_idx < min_placement {
                    continue;
                }

                // Skip placements that touch locked cells.
                if !(mask & locked_mask).is_zero() {
                    continue;
                }

                // Skip pair combos with same net effect as a previously tried combo.
                if prev_dup_placement < usize::MAX {
                    if let Some(ref table) = data.skip_tables[piece_idx] {
                        let num_curr = placements.len();
                        if table[prev_dup_placement * num_curr + pl_idx] {
                            continue;
                        }
                    }
                }

                board.apply_piece(mask);
                solution.push((row, col));

                let is_next_dup = config.duplicate_pruning
                    && piece_idx + 1 < data.all_placements.len()
                    && data.is_dup_of_prev[piece_idx + 1];

                let next_min = if is_next_dup { pl_idx } else { 0 };
                // Always pass placement for skip table lookup (works for any consecutive pair).
                let next_prev_dup = if piece_idx + 1 < data.all_placements.len()
                    && data.skip_tables[piece_idx + 1].is_some()
                {
                    pl_idx
                } else {
                    usize::MAX
                };

                if $name(
                    &board,
                    data,
                    piece_idx + 1,
                    next_min,
                    next_prev_dup,
                    solution,
                    nodes,
                    config,
                    $($abort_param,)?
                ) {
                    return true;
                }

                solution.pop();
                board.undo_piece(mask);
            }

            false
        }
    };
}

// Serial backtrack: no abort parameter, no overhead.
define_backtrack!(backtrack);

// Abortable backtrack: checks abort flag every 1024 nodes.
define_backtrack!(backtrack_abortable, abort: abort: &AtomicBool);
