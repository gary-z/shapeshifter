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
//! TODO: Add cumulative wrap count bound (MC precompute).
//!       Track total wraps during search; MC bounds wraps at each depth.
//!       (Equivalent to the deficit upper bound but tracked incrementally.)

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use super::SolverData;
use super::prune::hit_count::HitCounter;


// ---------------------------------------------------------------------------
// Phase 2: filter_placement — before apply_piece
// ---------------------------------------------------------------------------

/// Check if a placement should be skipped before applying it.
/// Returns true if the placement is valid (keep it), false to skip.
/// Checks: cell locking, zero-hit budget (generalizes wrapping filter), skip tables.
#[inline(always)]
pub(crate) fn filter_placement(
    data: &SolverData,
    piece_idx: usize,
    pl_idx: usize,
    mask: Bitboard,
    prev_placement: usize,
    fs: &FilterState,
) -> bool {
    // Zero-hit budget: if this placement hits more zero cells than the child
    // can afford (would make child_deficit > child_remaining_bits), skip.
    // Subsumes the wrapping filter (max_zeros_hit=0 when remaining_bits==deficit).
    if (mask & fs.zero_plane).count_ones() > fs.max_zeros_hit { return false; }

    // Skip table: deduplicate equivalent consecutive placement pairs.
    if prev_placement < usize::MAX {
        if let Some(ref table) = data.skip_tables[piece_idx] {
            let num_curr = data.all_placements[piece_idx].len();
            if table[prev_placement * num_curr + pl_idx] { return false; }
        }
    }

    true
}

/// Per-node filter state, constant across all placements at this node.
pub(crate) struct FilterState {
    pub zero_plane: Bitboard,
    /// Max zero cells a placement can hit without making the child's deficit
    /// exceed its remaining budget. Generalizes the wrapping filter:
    /// when max_zeros_hit=0, no zero cell can be touched at all.
    pub max_zeros_hit: u32,
}

/// Compute filter state that's constant across all placements at a node.
#[inline(always)]
pub(crate) fn filter_state(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
) -> FilterState {
    let zero_plane = board.plane(0);
    // Max zeros a placement can hit: (remaining_bits - deficit) / M.
    // Any placement exceeding this will fail the child's total_deficit check.
    let rb = data.total_deficit_prune.remaining_bits(piece_idx);
    let deficit = board.total_deficit();
    let max_zeros_hit = if rb >= deficit {
        (rb - deficit) / data.m as u32
    } else {
        0
    };
    FilterState { zero_plane, max_zeros_hit }
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
pub(crate) fn prune_node(
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
    // MC upper bound: deficit shouldn't be higher than random solutions at this depth.
    // Uses the current pipeline level's deficit bounds (tighter at lower percentiles).
    let mc_idx = data.mc_level_idx.load(std::sync::atomic::Ordering::Relaxed);
    if board.total_deficit() > data.mc_levels[mc_idx].max_deficit_at_depth[piece_idx] { return false; }
    // Reverse MC: bound deficit/jaggedness by what N-d pieces from solved can produce.
    {
        let remaining = data.all_placements.len() - piece_idx;
        let level = &data.mc_levels[mc_idx];
        if board.total_deficit() > level.rev_max_deficit[remaining] { return false; }
    }
    // MC jaggedness upper bound (progressive levels only — final level uses u32::MAX
    // because jaggedness is non-monotonic and MC can't guarantee coverage of all valid states).
    if config.jaggedness {
        let j = board.split_jaggedness(data.jaggedness_prune.h_mask(), data.jaggedness_prune.v_mask());
        let total_jagg = j.circular_h + j.circular_v;
        if total_jagg > data.mc_levels[mc_idx].max_jagg_at_depth[piece_idx] { return false; }
        let remaining = data.all_placements.len() - piece_idx;
        if total_jagg > data.mc_levels[mc_idx].rev_max_jagg[remaining] { return false; }
        // Deterministic jaggedness lower bound (existing, always sound).
        if !data.jaggedness_prune.try_prune_with_jagg(&j, piece_idx, data.m) { return false; }
    }
    if config.total_deficit_global && !data.parity_prune.try_prune(board, piece_idx, data.m, rb) { return false; }
    true
}
