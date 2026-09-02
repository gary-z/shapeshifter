mod cell_set;
mod jaggedness;
mod monte_carlo;
mod partition;
mod total_deficit;

pub(super) use cell_set::CellSetBound;
pub(super) use jaggedness::JaggednessBound;
pub(super) use monte_carlo::{HitCounter, MonteCarloBounds};
pub(super) use partition::PartitionReachability;
pub(super) use total_deficit::TotalDeficitBound;

use crate::core::board::Board;

use super::SolverData;

#[inline(always)]
pub(super) fn is_canonical_placement_pair(
    data: &SolverData,
    piece_index: usize,
    placement_index: usize,
    previous_placement: usize,
) -> bool {
    if previous_placement == usize::MAX {
        return true;
    }

    data.equivalent_pair_skips[piece_index]
        .as_ref()
        .is_none_or(|table| {
            let placement_count = data.placements[piece_index].len();
            !table[previous_placement * placement_count + placement_index]
        })
}

#[inline(always)]
pub(super) fn max_zero_cells_allowed<const MODULUS: usize>(
    board: &Board,
    data: &SolverData,
    piece_index: usize,
) -> u32 {
    let remaining_cells = data.total_deficit.remaining_cells(piece_index);
    let deficit = board.total_deficit();
    remaining_cells.saturating_sub(deficit) / MODULUS as u32
}

#[inline(always)]
pub(super) fn state_is_feasible<const MODULUS: usize>(
    board: &Board,
    data: &SolverData,
    piece_index: usize,
) -> bool {
    let remaining_cells = data.total_deficit.remaining_cells(piece_index);

    data.total_deficit.allows::<MODULUS>(board, piece_index)
        && data
            .monte_carlo
            .allows_state::<MODULUS>(board, piece_index, &data.jaggedness)
        && data
            .partition_reachability
            .allows(board, piece_index, MODULUS as u8, remaining_cells)
        && data
            .cell_set_bound
            .allows::<MODULUS>(board, piece_index, remaining_cells)
}
