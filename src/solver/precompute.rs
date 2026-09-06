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
    guided_frontier: bool,
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
    let small_component = super::pruning::SmallComponentBound::precompute(
        pieces,
        piece_order,
        height,
        width,
        modulus,
    );
    let cell_set_bound =
        super::pruning::CellSetBound::precompute(&placements, height, width, modulus);
    let anchor_placements = placements
        .iter()
        .map(|piece_placements| super::backtrack::AnchorPlacementData::precompute(piece_placements))
        .collect();

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

    let reverse_likelihood = guided_frontier.then(|| {
        super::likelihood::ReverseLikelihood::precompute(&placements, height, width, modulus)
    });

    SolverData {
        placements,
        total_deficit,
        jaggedness,
        partition_reachability,
        small_component,
        cell_set_bound,
        anchor_placements,
        reverse_likelihood,
        equivalent_pair_skips,
        single_cell_suffix_start,
        modulus,
        height,
        width,
        progress_weights,
    }
}
