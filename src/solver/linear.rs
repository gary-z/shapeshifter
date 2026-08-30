//! Fractional one-hot relaxation used to diversify parallel search.

use std::time::Duration;

use microlp::{ComparisonOp, OptimizationDirection, Problem};
use rand::{RngExt, SeedableRng};

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

/// Solve a randomized LP relaxation and return one score per placement.
///
/// Piece choices are fractional but still sum to one. Cell equations retain
/// the wrap variables, relaxed to real values. Different deterministic random
/// objectives select different extreme points of this global relaxation.
pub(crate) fn relaxation_hints(
    board: &Board,
    placements: &[Vec<(usize, usize, Bitboard)>],
    seed: u64,
) -> Option<Vec<Vec<f64>>> {
    let mut problem = Problem::new(OptimizationDirection::Minimize);
    problem.set_time_limit(Duration::from_secs(1));
    let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
    let choices = placements
        .iter()
        .map(|piece_placements| {
            piece_placements
                .iter()
                .map(|_| problem.add_var(rng.random::<f64>() - 0.5, (0.0, 1.0)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for piece_choices in &choices {
        problem.add_constraint(
            piece_choices.iter().map(|&variable| (variable, 1.0)),
            ComparisonOp::Eq,
            1.0,
        );
    }

    let max_wraps = placements.len() as f64 / board.m() as f64;
    for row in 0..board.height() as usize {
        for column in 0..board.width() as usize {
            let bit = (row * 15 + column) as u32;
            let mut terms = Vec::new();
            for (piece, piece_placements) in placements.iter().enumerate() {
                for (placement, &(_, _, mask)) in piece_placements.iter().enumerate() {
                    if mask.get_bit(bit) {
                        terms.push((choices[piece][placement], 1.0));
                    }
                }
            }
            let wraps = problem.add_var(0.0, (0.0, max_wraps));
            terms.push((wraps, -(board.m() as f64)));
            problem.add_constraint(terms, ComparisonOp::Eq, board.get(row, column) as f64);
        }
    }

    let outcome = problem.solve().ok()?;
    let solution = outcome.solution()?;
    Some(
        choices
            .iter()
            .map(|piece_choices| {
                piece_choices
                    .iter()
                    .map(|&variable| solution.var_value(variable))
                    .collect()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::Piece;

    #[test]
    fn relaxation_identifies_forced_placement() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let placements = vec![Piece::from_grid(&[&[true]]).placements(3, 3)];
        let hints = relaxation_hints(&board, &placements, 1).unwrap();
        assert!(hints[0][0] > 0.999);
    }
}
