use super::{PiecePlacements, SolverData};
use crate::core::board::Board;
use crate::core::piece::Piece;

pub(super) fn build_solver_data(
    board: &Board,
    pieces: &[Piece],
    piece_order: &[usize],
    placements: Vec<PiecePlacements>,
    equivalent_pair_skips: Vec<Option<Vec<bool>>>,
    single_cell_suffix_start: usize,
) -> SolverData {
    let height = board.height();
    let width = board.width();
    let modulus = board.m();
    let total_deficit =
        super::pruning::TotalDeficitBound::precompute(pieces, piece_order, height, width);
    let jaggedness =
        super::pruning::JaggednessBound::precompute(pieces, piece_order, height, width);
    let partition_reachability =
        super::pruning::PartitionReachability::precompute(pieces, piece_order, height, width);
    let cell_set_bound =
        super::pruning::CellSetBound::precompute(&placements, height, width, modulus);

    #[cfg(not(target_arch = "wasm32"))]
    let progress_weights: Vec<f64> = {
        let piece_count = pieces.len();
        let mut suffix_products = vec![1.0f64; piece_count + 1];
        for piece_index in (0..piece_count).rev() {
            suffix_products[piece_index] =
                suffix_products[piece_index + 1] * placements[piece_index].len() as f64;
        }
        let total_space = suffix_products[0];
        (0..piece_count)
            .map(|piece_index| {
                if total_space > 0.0 {
                    suffix_products[piece_index + 1] / total_space
                } else {
                    0.0
                }
            })
            .collect()
    };

    let monte_carlo = super::pruning::MonteCarloBounds::precompute(board, &placements, modulus);

    SolverData {
        placements,
        total_deficit,
        jaggedness,
        partition_reachability,
        cell_set_bound,
        monte_carlo,
        equivalent_pair_skips,
        single_cell_suffix_start,
        modulus,
        height,
        width,
        #[cfg(not(target_arch = "wasm32"))]
        progress_weights,
    }
}
