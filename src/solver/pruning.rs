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

use crate::core::board::Board;
use super::SolverData;


// ---------------------------------------------------------------------------
// Phase 2: filter_placement — before apply_piece
// ---------------------------------------------------------------------------

/// Check if a placement should be skipped before applying it.
/// Returns true if the placement is valid (keep it), false to skip.
///
/// The zero-hit budget is *not* checked here: it is the primary sort key, so
/// `sort_placements` drops over-budget placements while bucketing and never
/// emits them. Only the pair skip table remains.
#[inline(always)]
pub(crate) fn filter_placement(
    data: &SolverData,
    piece_idx: usize,
    pl_idx: usize,
    prev_placement: usize,
) -> bool {
    // Skip table: deduplicate equivalent consecutive placement pairs.
    if prev_placement < usize::MAX {
        if let Some(ref table) = data.skip_tables[piece_idx] {
            let num_curr = data.all_placements[piece_idx].len();
            if table[prev_placement * num_curr + pl_idx] { return false; }
        }
    }

    true
}

/// Max zero cells a placement can hit without making the child's deficit
/// exceed its remaining budget. Generalizes the wrapping filter: when the
/// result is 0, no zero cell can be touched at all.
#[inline(always)]
pub(crate) fn max_zeros_hit<const M: usize>(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
) -> u32 {
    // (remaining_bits - deficit) / M. Any placement exceeding this will fail
    // the child's total_deficit check.
    let rb = data.total_deficit_prune.remaining_bits(piece_idx);
    let deficit = board.total_deficit();
    if rb >= deficit { (rb - deficit) / M as u32 } else { 0 }
}

// ---------------------------------------------------------------------------
// Phase 3: check_placement — after apply_piece
// ---------------------------------------------------------------------------

// Phase 3 (check_placement) is inlined in the backtracker for performance.
// It updates the HitCounter and checks the depth-aware hit count threshold
// from mc_levels[mc_level_idx].max_hits_at_depth[piece_idx + 1].

// ---------------------------------------------------------------------------
// Phase 4: prune_node — per-node feasibility check
// ---------------------------------------------------------------------------

/// Run all pruning checks for a given board state and piece index.
/// Returns true if the state is feasible (search should continue).
/// Ordered by cost-effectiveness: cheapest high-impact checks first.
#[inline(always)]
pub(crate) fn prune_node<const M: usize>(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    config: &super::PruningConfig,
) -> bool {
    let rb = data.total_deficit_prune.remaining_bits(piece_idx);

    // Note: total_deficit lower bound is also enforced by filter_placement's zero-hit
    // budget (which guarantees the child won't exceed remaining_bits). This check is
    // redundant for children of filtered parents but serves as a safety net.
    if config.total_deficit_global && !data.total_deficit_prune.try_prune(board, piece_idx) { return false; }
    // MC bounds (forward + reverse) + deterministic jaggedness (all share one jagg computation).
    if !data.mc_prune.try_prune::<M>(board, piece_idx, &data.jaggedness_prune) { return false; }
    if config.total_deficit_global && !data.parity_prune.try_prune(board, piece_idx, M as u8, rb) { return false; }
    true
}
