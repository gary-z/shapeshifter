//! Pruning dispatcher.
//!
//! Pruning has four phases, ordered by when they fire in the search:
//!
//! 1. **precompute** — build static data from pieces/board (called once).
//!    Lives in each `solver::prune::*` module's `precompute()`.
//!
//! 2. **filter_placement** — per-placement check BEFORE apply_piece.
//!    Cheap filters that avoid the cost of applying + recursing + pruning.
//!    Called from `build_search_frame` to pre-filter candidate placements.
//!
//! 3. **check_placement** — per-placement check AFTER apply_piece.
//!    Updates incremental state (e.g., HitCounter) and checks thresholds.
//!    Returns false to skip this placement (still counts as a node visited).
//!
//! 4. **prune_node** — per-node feasibility check at start of recursion.
//!    Chains all prune techniques in cost-effectiveness order.
//!
//! TODO: Add depth-dependent total deficit upper bound (MC precompute).
//!       Currently only lower-bounded; MC can establish max typical deficit at each depth.
//! TODO: Add cumulative wrap count bound (MC precompute).
//!       Track total wraps during search; MC bounds wraps at each depth.

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use super::SolverData;
use super::prune::hit_count::HitCounter;

// Re-export types still needed by precompute.rs (subset/weight_tuple construction).
pub(crate) use super::prune::subset::SubsetReachability;
pub(crate) use super::prune::weight_tuple::WeightTupleReachability;

// ---------------------------------------------------------------------------
// Phase 2: filter_placement — before apply_piece
// ---------------------------------------------------------------------------

/// Check if a placement should be skipped before applying it.
/// Returns true if the placement is valid (keep it), false to skip.
/// Checks: cell locking, wrapping impossibility, skip tables.
#[inline(always)]
pub(crate) fn filter_placement(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    pl_idx: usize,
    mask: Bitboard,
    prev_placement: usize,
    locked_mask: Bitboard,
    zero_plane: Bitboard,
    no_wrap: bool,
) -> bool {
    // Cell locking: skip placements that hit locked zero cells.
    if !(mask & locked_mask).is_zero() { return false; }

    // Wrapping impossibility: when remaining_bits == deficit, hitting a zero
    // cell must wrap → deficit increases → guaranteed prune at next depth.
    if no_wrap && !(mask & zero_plane).is_zero() { return false; }

    // Skip table: deduplicate equivalent consecutive placement pairs.
    if prev_placement < usize::MAX {
        if let Some(ref table) = data.skip_tables[piece_idx] {
            let num_curr = data.all_placements[piece_idx].len();
            if table[prev_placement * num_curr + pl_idx] { return false; }
        }
    }

    true
}

/// Compute filter state that's constant across all placements at a node.
#[inline(always)]
pub(crate) fn filter_state(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    config: &super::PruningConfig,
) -> (Bitboard, Bitboard, bool) {
    let locked_mask = if config.cell_locking {
        board.plane(0) & !data.suffix_coverage[piece_idx].coverage_ge(data.m)
    } else {
        Bitboard::ZERO
    };
    let zero_plane = board.plane(0);
    let no_wrap = data.total_deficit_prune.remaining_bits(piece_idx) == board.total_deficit();
    (locked_mask, zero_plane, no_wrap)
}

// ---------------------------------------------------------------------------
// Phase 3: check_placement — after apply_piece
// ---------------------------------------------------------------------------

/// Update incremental state and check placement-level constraints.
/// Returns the updated HitCounter, or None to prune this placement.
/// Counts as a node visited either way.
#[inline(always)]
pub(crate) fn check_placement(
    hits: HitCounter,
    mask: Bitboard,
    data: &SolverData,
) -> Option<HitCounter> {
    let mut new_hits = hits;
    new_hits.apply_piece(mask);
    let t = data.hit_count_threshold.load(std::sync::atomic::Ordering::Relaxed);
    if t > 0 && new_hits.any_cell_gte(t) {
        return None;
    }
    Some(new_hits)
}

// ---------------------------------------------------------------------------
// Phase 4: prune_node — per-node feasibility check
// ---------------------------------------------------------------------------

/// Run all pruning checks for a given board state and piece index.
/// Returns true if the state is feasible (search should continue).
/// Ordered by cost-effectiveness: cheapest high-impact checks first.
pub(crate) fn prune_node(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    config: &super::PruningConfig,
) -> bool {
    let rb = data.total_deficit_prune.remaining_bits(piece_idx);

    if config.total_deficit_global && !data.total_deficit_prune.try_prune(board, piece_idx) { return false; }
    if config.jaggedness && !data.jaggedness_prune.try_prune(board, piece_idx, data.m) { return false; }
    if config.total_deficit_rowcol && !data.line_family_prune.try_prune_rowcol(board, piece_idx, data.m) { return false; }
    if config.total_deficit_diagonal && !data.line_family_prune.try_prune_diagonal(board, piece_idx, data.m) { return false; }
    if config.total_deficit_global && !data.parity_prune.try_prune(board, piece_idx, data.m, rb) { return false; }
    if config.total_deficit_global && !data.subset_prune.try_prune(board, piece_idx) { return false; }
    if config.total_deficit_global && !data.weight_tuple_prune.try_prune(board, piece_idx) { return false; }
    if config.subgame && !data.subgame_prune.try_prune(board, piece_idx, rb) { return false; }
    true
}
