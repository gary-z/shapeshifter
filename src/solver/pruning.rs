//! Pruning dispatcher.
//!
//! Individual pruning techniques live in `solver::prune::*`.
//! This module provides `prune_node` which chains them in cost-effectiveness order.

use crate::core::board::Board;
use super::SolverData;

// Re-export types still needed by precompute.rs (subset/weight_tuple construction).
pub(crate) use super::prune::subset::SubsetReachability;
pub(crate) use super::prune::weight_tuple::WeightTupleReachability;

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
    if config.total_deficit_global && !data.region_budget_prune.try_prune(board, piece_idx) { return false; }
    if config.subgame && !data.subgame_prune.try_prune(board, piece_idx, rb) { return false; }
    true
}
