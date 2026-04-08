use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::coverage::precompute_suffix_coverage;
use crate::core::piece::Piece;
use super::SolverData;

/// Build all precomputed data needed by the backtracking solver.
///
/// This includes: suffix sums, jaggedness masks, parity partitions,
/// suffix coverage, and hit-count MC levels.
pub(crate) fn build_solver_data(
    board: &Board,
    pieces: &[Piece],
    order: &[usize],
    all_placements: Vec<Vec<(usize, usize, Bitboard)>>,
    skip_tables: Vec<Option<Vec<bool>>>,
    single_cell_start: usize,
    h: u8,
    w: u8,
    m: u8,
) -> SolverData {
    let n = pieces.len();

    // Precompute suffix sums/maxes of piece properties.
    let total_deficit_prune = super::prune::total_deficit::TotalDeficitPrune::precompute(pieces, order);
    let jaggedness_prune = super::prune::jaggedness::JaggednessPrune::precompute(pieces, order, h, w);

    // Precompute per-piece reach: union of all placement masks.
    let reaches: Vec<Bitboard> = all_placements
        .iter()
        .map(|placements| {
            let mut reach = Bitboard::ZERO;
            for &(_, _, mask) in placements {
                reach |= mask;
            }
            reach
        })
        .collect();

    // Precompute suffix coverage in binary bitboard layers.
    let suffix_coverage = precompute_suffix_coverage(&reaches);

    let parity_prune = super::prune::parity::ParityPrune::precompute(pieces, order, h, w, m);

    // Compute progress weights: fraction of naive search space per placement at each depth.
    let mut suffix_products = vec![1.0f64; n + 1];
    for d in (0..n).rev() {
        suffix_products[d] = suffix_products[d + 1] * all_placements[d].len() as f64;
    }
    let total_space = suffix_products[0];
    let progress_weights: Vec<f64> = (0..n)
        .map(|d| if total_space > 0.0 { suffix_products[d + 1] / total_space } else { 0.0 })
        .collect();

    let mc_levels = super::prune::hit_count::precompute_mc(board, &all_placements, m);
    let num_levels = mc_levels.len();

    SolverData {
        all_placements,
        total_deficit_prune,
        jaggedness_prune,
        parity_prune,
        mc_levels,
        mc_level_idx: std::sync::atomic::AtomicUsize::new(num_levels.saturating_sub(1)),
        suffix_coverage,
        skip_tables,
        single_cell_start,
        m,
        h,
        w,
        progress_weights,
    }
}
