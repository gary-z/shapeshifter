//! Approximate reverse likelihood for guiding the hard native search frontier.
//!
//! Each window table is the exact distribution induced by uniformly random
//! placements of the remaining pieces. Overlapping 3x3 window costs are
//! combined with their 3x2, 2x3, and 2x2 intersections (region-graph
//! inclusion-exclusion), which avoids repeatedly counting the same evidence.

use std::ops::RangeInclusive;

use rayon::prelude::*;

use super::PiecePlacements;
use crate::core::STRIDE;
use crate::core::board::Board;

const SCORE_SCALE: f64 = 256.0;

fn subtraction_table(modulus: usize, cell_count: usize) -> Vec<Vec<usize>> {
    let effect_count = 1 << cell_count;
    let mut table = vec![vec![0; effect_count]; modulus.pow(cell_count as u32)];
    for (state, effects) in table.iter_mut().enumerate() {
        effects[0] = state;
        let mut encoded = state;
        let mut multiplier = 1;
        for cell in 0..cell_count {
            let digit = encoded % modulus;
            encoded /= modulus;
            let previous = (digit + modulus - 1) % modulus;
            let delta = (previous as isize - digit as isize) * multiplier as isize;
            // Extend every effect on the preceding cells with this cell's
            // independent modular decrement. Decode each state only once.
            let half = 1 << cell;
            let (without, with) = effects[..2 * half].split_at_mut(half);
            for (previous_state, &state) in with.iter_mut().zip(without.iter()) {
                *previous_state = (state as isize + delta) as usize;
            }
            multiplier *= modulus;
        }
    }
    table
}

struct WindowLikelihood {
    cells: Vec<Vec<usize>>,
    costs: Vec<Vec<Vec<u16>>>,
    placement_effects: Vec<Vec<Vec<(u8, u16)>>>,
    modulus: usize,
}

impl WindowLikelihood {
    fn precompute(
        placements: &[PiecePlacements],
        modulus: usize,
        window_height: usize,
        window_width: usize,
        rows: RangeInclusive<usize>,
        columns: RangeInclusive<usize>,
    ) -> Self {
        let window_cell_count = window_height * window_width;
        let effect_count = 1 << window_cell_count;
        let cells = rows
            .flat_map(|row| {
                columns.clone().map(move |column| {
                    (0..window_cell_count)
                        .map(|index| {
                            let window_row = index / window_width;
                            let window_column = index % window_width;
                            (row + window_row) * STRIDE + column + window_column
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let state_count = modulus.pow(window_cell_count as u32);
        let mut probabilities =
            vec![vec![vec![0.0; state_count]; cells.len()]; placements.len() + 1];
        for window in &mut probabilities[placements.len()] {
            window[0] = 1.0;
        }

        let subtract = subtraction_table(modulus, window_cell_count);

        for piece_index in (0..placements.len()).rev() {
            let (through_current, after_current) = probabilities.split_at_mut(piece_index + 1);
            let current = &mut through_current[piece_index];
            let next = &after_current[0];
            current
                .par_iter_mut()
                .zip(next.par_iter())
                .zip(cells.par_iter())
                .for_each(|((current_window, next_window), window_cells)| {
                    let mut effect_counts = vec![0usize; effect_count];
                    for &(_, _, mask) in &placements[piece_index] {
                        let effect = window_cells.iter().enumerate().fold(
                            0,
                            |value, (cell_index, &cell)| {
                                value | (usize::from(mask.get_bit(cell as u32)) << cell_index)
                            },
                        );
                        effect_counts[effect] += 1;
                    }
                    let placement_count = placements[piece_index].len() as f64;
                    let effects = effect_counts
                        .into_iter()
                        .enumerate()
                        .filter(|&(_, count)| count != 0)
                        .map(|(effect, count)| (effect, count as f64 / placement_count))
                        .collect::<Vec<_>>();
                    for state in 0..state_count {
                        current_window[state] = effects
                            .iter()
                            .map(|&(effect, probability)| {
                                probability * next_window[subtract[state][effect]]
                            })
                            .sum();
                    }
                });
        }

        let costs = probabilities
            .into_iter()
            .map(|depth| {
                depth
                    .into_iter()
                    .map(|window| {
                        window
                            .into_iter()
                            .map(|probability| {
                                (-probability.max(1e-100).ln() * SCORE_SCALE)
                                    .round()
                                    .min(f64::from(u16::MAX)) as u16
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();

        let placement_effects = placements
            .iter()
            .map(|piece_placements| {
                piece_placements
                    .iter()
                    .map(|&(_, _, mask)| {
                        cells
                            .iter()
                            .enumerate()
                            .filter_map(|(window_index, window_cells)| {
                                let effect = window_cells.iter().enumerate().fold(
                                    0u16,
                                    |value, (cell_index, &cell)| {
                                        value | (u16::from(mask.get_bit(cell as u32)) << cell_index)
                                    },
                                );
                                (effect != 0).then_some((window_index as u8, effect))
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();

        Self {
            cells,
            costs,
            placement_effects,
            modulus,
        }
    }

    fn add_placement_scores(
        &self,
        board: &Board,
        piece_index: usize,
        placement_indices: &[u8],
        weight: i32,
        scores: &mut [i32; super::backtrack::MAX_PLACEMENTS],
    ) {
        let next_piece_index = piece_index + 1;
        let mut states = [0u32; 144];
        let mut base_score = 0i32;
        for (window_index, window_cells) in self.cells.iter().enumerate() {
            let mut state = 0usize;
            let mut multiplier = 1;
            for &cell in window_cells {
                state += usize::from(board.get(cell / STRIDE, cell % STRIDE)) * multiplier;
                multiplier *= self.modulus;
            }
            states[window_index] = state as u32;
            base_score += i32::from(self.costs[next_piece_index][window_index][state]);
        }

        for &placement_index in placement_indices {
            let mut score = base_score;
            for &(window_index, effect) in
                &self.placement_effects[piece_index][placement_index as usize]
            {
                let window_index = usize::from(window_index);
                let old_state = states[window_index] as usize;
                let mut encoded = old_state;
                let mut new_state = 0;
                let mut multiplier = 1;
                for cell_index in 0..self.cells[window_index].len() {
                    let digit = encoded % self.modulus;
                    encoded /= self.modulus;
                    let new_digit = if effect & (1 << cell_index) == 0 {
                        digit
                    } else {
                        (digit + self.modulus - 1) % self.modulus
                    };
                    new_state += new_digit * multiplier;
                    multiplier *= self.modulus;
                }
                score -= i32::from(self.costs[next_piece_index][window_index][old_state]);
                score += i32::from(self.costs[next_piece_index][window_index][new_state]);
            }
            scores[placement_index as usize] += weight * score;
        }
    }

    #[cfg(test)]
    fn board_score(&self, board: &Board, piece_index: usize) -> i32 {
        self.cells
            .iter()
            .enumerate()
            .map(|(window_index, window_cells)| {
                let mut state = 0usize;
                let mut multiplier = 1;
                for &cell in window_cells {
                    state += usize::from(board.get(cell / STRIDE, cell % STRIDE)) * multiplier;
                    multiplier *= self.modulus;
                }
                i32::from(self.costs[piece_index][window_index][state])
            })
            .sum()
    }
}

pub(super) struct ReverseLikelihood {
    regions: Vec<(i32, WindowLikelihood)>,
}

impl ReverseLikelihood {
    /// Bound the exponential state space of each 3x3 window independently of
    /// the board dimensions or number of pieces.
    pub(super) fn can_precompute(modulus: u8) -> bool {
        const MAX_WINDOW_STATES: usize = 1 << 15;
        usize::from(modulus).pow(9) <= MAX_WINDOW_STATES
    }

    pub(super) fn precompute(
        placements: &[PiecePlacements],
        height: u8,
        width: u8,
        modulus: u8,
    ) -> Self {
        let height = usize::from(height);
        let width = usize::from(width);
        let modulus = usize::from(modulus);
        let regions = vec![
            (
                1,
                WindowLikelihood::precompute(
                    placements,
                    modulus,
                    3,
                    3,
                    0..=height - 3,
                    0..=width - 3,
                ),
            ),
            (
                -1,
                WindowLikelihood::precompute(
                    placements,
                    modulus,
                    3,
                    2,
                    0..=height - 3,
                    1..=width - 3,
                ),
            ),
            (
                -1,
                WindowLikelihood::precompute(
                    placements,
                    modulus,
                    2,
                    3,
                    1..=height - 3,
                    0..=width - 3,
                ),
            ),
            (
                1,
                WindowLikelihood::precompute(
                    placements,
                    modulus,
                    2,
                    2,
                    1..=height - 3,
                    1..=width - 3,
                ),
            ),
        ];
        Self { regions }
    }

    pub(super) fn score_placements(
        &self,
        board: &Board,
        piece_index: usize,
        placement_indices: &[u8],
        scores: &mut [i32; super::backtrack::MAX_PLACEMENTS],
    ) {
        scores.fill(0);
        for (weight, region) in &self.regions {
            region.add_placement_scores(board, piece_index, placement_indices, *weight, scores);
        }
    }

    #[cfg(test)]
    fn board_score(&self, board: &Board, piece_index: usize) -> i32 {
        self.regions
            .iter()
            .map(|(weight, region)| weight * region.board_score(board, piece_index))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::Piece;

    #[test]
    fn subtraction_table_matches_digitwise_modular_subtraction() {
        for modulus in 2usize..=3 {
            for cell_count in 0..=9 {
                let table = subtraction_table(modulus, cell_count);
                for (state, effects) in table.iter().enumerate() {
                    for (effect, &actual) in effects.iter().enumerate() {
                        let mut encoded = state;
                        let mut expected = 0;
                        let mut multiplier = 1;
                        for cell in 0..cell_count {
                            let digit = encoded % modulus;
                            encoded /= modulus;
                            let hit = (effect >> cell) & 1;
                            expected += (digit + modulus - hit) % modulus * multiplier;
                            multiplier *= modulus;
                        }
                        assert_eq!(actual, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn window_costs_match_enumerated_suffix_placements() {
        let placements = vec![
            Piece::from_grid(&[&[true, true]]).placements(3, 3),
            Piece::from_grid(&[&[true], &[true]]).placements(3, 3),
        ];
        for modulus in 2usize..=3 {
            let window = WindowLikelihood::precompute(&placements, modulus, 3, 3, 0..=0, 0..=0);
            for depth in 0..=placements.len() {
                let mut assignments = vec![[0usize; 9]];
                for piece in &placements[depth..] {
                    assignments = assignments
                        .into_iter()
                        .flat_map(|cells| {
                            piece.iter().map(move |&(_, _, mask)| {
                                std::array::from_fn(|cell| {
                                    (cells[cell]
                                        + usize::from(
                                            mask.get_bit((cell / 3 * STRIDE + cell % 3) as u32),
                                        ))
                                        % modulus
                                })
                            })
                        })
                        .collect();
                }
                let total = assignments.len() as f64;
                let mut counts = vec![0; modulus.pow(9)];
                for cells in assignments {
                    let state = cells
                        .iter()
                        .rev()
                        .fold(0, |state, &digit| state * modulus + digit);
                    counts[state] += 1;
                }
                for (state, count) in counts.into_iter().enumerate() {
                    let probability = (count as f64 / total).max(1e-100);
                    let expected = (-probability.ln() * SCORE_SCALE)
                        .round()
                        .min(f64::from(u16::MAX)) as u16;
                    assert_eq!(window.costs[depth][0][state], expected);
                }
            }
        }
    }

    #[test]
    fn incremental_placement_scores_match_full_scores() {
        for modulus in 2..=3 {
            let grid: Vec<Vec<u8>> = (0..4)
                .map(|row| (0..4).map(|column| (row + column) % modulus).collect())
                .collect();
            let rows = grid.iter().map(Vec::as_slice).collect::<Vec<_>>();
            let board = Board::from_grid(&rows, modulus);
            let placements = vec![
                Piece::from_grid(&[&[true, true]]).placements(4, 4),
                Piece::from_grid(&[&[true], &[true]]).placements(4, 4),
            ];
            let likelihood = ReverseLikelihood::precompute(&placements, 4, 4, modulus);
            let indices = (0..placements[0].len())
                .map(|index| index as u8)
                .collect::<Vec<_>>();
            let mut scores = [0i32; super::super::backtrack::MAX_PLACEMENTS];
            likelihood.score_placements(&board, 0, &indices, &mut scores);

            for &placement_index in &indices {
                let mut child = board;
                child.apply_piece(placements[0][placement_index as usize].2);
                assert_eq!(
                    scores[placement_index as usize],
                    likelihood.board_score(&child, 1)
                );
            }
        }
    }
}
