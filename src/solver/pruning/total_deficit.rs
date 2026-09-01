//! Rejects states whose remaining pieces cannot cover the board's deficit.
//!
//! A connected multi-cell piece covering an isolated nonzero cell must also touch
//! a currently zero neighbor. Each distinct zero neighbor touched consumes at
//! least one of the `(remaining cells - deficit) / M` future zero-hit cycles.

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::{Board, MAX_M};
use crate::core::piece::Piece;

pub(crate) struct TotalDeficitBound {
    remaining_cells: Vec<u32>,
    remaining_single_cell_pieces: Vec<u32>,
    pieces_are_connected: bool,
    valid_mask: Bitboard,
}

impl TotalDeficitBound {
    pub(crate) fn precompute(
        pieces: &[Piece],
        piece_order: &[usize],
        height: u8,
        width: u8,
    ) -> Self {
        let piece_count = pieces.len();
        let mut remaining_cells = vec![0u32; piece_count + 1];
        let mut remaining_single_cell_pieces = vec![0u32; piece_count + 1];
        for piece_index in (0..piece_count).rev() {
            let piece = &pieces[piece_order[piece_index]];
            let cell_count = piece.cell_count();
            remaining_cells[piece_index] = remaining_cells[piece_index + 1] + cell_count;
            remaining_single_cell_pieces[piece_index] =
                remaining_single_cell_pieces[piece_index + 1] + u32::from(cell_count == 1);
        }

        let mut valid_mask = Bitboard::ZERO;
        for row in 0..height as usize {
            for column in 0..width as usize {
                valid_mask.set_bit((row * STRIDE + column) as u32);
            }
        }

        Self {
            remaining_cells,
            remaining_single_cell_pieces,
            pieces_are_connected: pieces.iter().all(|piece| shape_is_connected(piece.shape())),
            valid_mask,
        }
    }

    #[inline(always)]
    pub(crate) fn remaining_cells(&self, piece_index: usize) -> u32 {
        self.remaining_cells[piece_index]
    }

    #[inline(always)]
    pub(crate) fn allows<const MODULUS: usize>(&self, board: &Board, piece_index: usize) -> bool {
        let remaining_cells = self.remaining_cells[piece_index];
        let total_deficit = board.total_deficit();
        if remaining_cells < total_deficit {
            return false;
        }
        if !self.pieces_are_connected {
            return true;
        }

        let zero_hit_budget = (remaining_cells - total_deficit) / MODULUS as u32;
        let nonzero = self.valid_mask & !board.plane(0);
        let neighbors =
            nonzero.shl_1() | nonzero.shr_1() | nonzero.shl_stride() | nonzero.shr_stride();
        let isolated = nonzero & !neighbors;
        let isolated_count = isolated.count_ones();
        if isolated_count == 0 || zero_hit_budget >= isolated_count {
            return true;
        }

        let mut isolated_counts = [0u32; MAX_M];
        let mut isolated_deficit = 0;
        for (deficit, isolated_count) in
            isolated_counts.iter_mut().enumerate().take(MODULUS).skip(1)
        {
            let count = (isolated & board.plane(deficit as u8)).count_ones();
            *isolated_count = count;
            isolated_deficit += deficit as u32 * count;
        }

        let single_cell_pieces = self.remaining_single_cell_pieces[piece_index];
        if isolated_deficit <= single_cell_pieces {
            return true;
        }

        let required_neighbor_cover = isolated_deficit - single_cell_pieces;
        if zero_hit_budget
            * maximum_singleton_deficit_around_one_neighbor::<MODULUS>(&isolated_counts)
            < required_neighbor_cover
        {
            return false;
        }

        if zero_hit_budget
            >= minimum_unsupported_singletons::<MODULUS>(&isolated_counts, single_cell_pieces)
        {
            return true;
        }

        singleton_neighbor_cover_upper_bound::<MODULUS>(
            board,
            isolated,
            self.valid_mask,
            zero_hit_budget,
        ) >= required_neighbor_cover
    }
}

fn maximum_singleton_deficit_around_one_neighbor<const MODULUS: usize>(
    isolated_counts: &[u32; MAX_M],
) -> u32 {
    let mut remaining_neighbors = 4;
    let mut maximum_deficit = 0;
    for deficit in (1..MODULUS).rev() {
        let used = remaining_neighbors.min(isolated_counts[deficit]);
        maximum_deficit += used * deficit as u32;
        remaining_neighbors -= used;
    }
    maximum_deficit
}

fn minimum_unsupported_singletons<const MODULUS: usize>(
    isolated_counts: &[u32; MAX_M],
    single_cell_pieces: u32,
) -> u32 {
    let mut remaining_single_cell_pieces = single_cell_pieces;
    let mut unsupported: u32 = isolated_counts.iter().sum();
    for (deficit, &isolated_count) in isolated_counts.iter().enumerate().take(MODULUS).skip(1) {
        let supported = isolated_count.min(remaining_single_cell_pieces / deficit as u32);
        unsupported -= supported;
        remaining_single_cell_pieces -= supported * deficit as u32;
    }
    unsupported
}

fn shape_is_connected(shape: Bitboard) -> bool {
    let mut reached = Bitboard::from_bit(shape.lowest_set_bit());
    loop {
        let expanded = (reached
            | reached.shl_1()
            | reached.shr_1()
            | reached.shl_stride()
            | reached.shr_stride())
            & shape;
        if expanded == reached {
            return reached == shape;
        }
        reached = expanded;
    }
}

fn singleton_neighbor_cover_upper_bound<const MODULUS: usize>(
    board: &Board,
    isolated: Bitboard,
    valid_mask: Bitboard,
    zero_hit_budget: u32,
) -> u32 {
    const SCORE_BITS: usize = 5;
    const DEFICIT_BITS: usize = 3;
    const MAX_NEIGHBOR_DEFICIT: usize = 4 * (MAX_M - 1);

    let candidates =
        (isolated.shl_1() | isolated.shr_1() | isolated.shl_stride() | isolated.shr_stride())
            & valid_mask
            & board.plane(0);
    let mut deficit_bits = [Bitboard::ZERO; DEFICIT_BITS];
    for deficit in 1..MODULUS {
        let cells = isolated & board.plane(deficit as u8);
        for (bit, deficit_plane) in deficit_bits.iter_mut().enumerate() {
            if deficit & (1 << bit) != 0 {
                *deficit_plane |= cells;
            }
        }
    }

    let directional_deficits = [
        deficit_bits.map(|plane| plane.shl_1()),
        deficit_bits.map(|plane| plane.shr_1()),
        deficit_bits.map(|plane| plane.shl_stride()),
        deficit_bits.map(|plane| plane.shr_stride()),
    ];
    let mut score_bits = [Bitboard::ZERO; SCORE_BITS];
    for addend_bits in directional_deficits {
        let mut carry = Bitboard::ZERO;
        for (bit, accumulator) in score_bits.iter_mut().enumerate() {
            let addend = addend_bits.get(bit).copied().unwrap_or(Bitboard::ZERO);
            let previous = *accumulator;
            *accumulator = previous ^ addend ^ carry;
            carry = (previous & addend) | (previous & carry) | (addend & carry);
        }
        debug_assert!(carry.is_zero());
    }

    let mut remaining_budget = zero_hit_budget;
    let mut maximum_cover = 0;
    for weight in (1..=MAX_NEIGHBOR_DEFICIT).rev() {
        let mut matches = candidates;
        for (bit, score_plane) in score_bits.iter().enumerate() {
            matches &= if weight & (1 << bit) != 0 {
                *score_plane
            } else {
                !*score_plane
            };
        }
        let used = remaining_budget.min(matches.count_ones());
        maximum_cover += used * weight as u32;
        remaining_budget -= used;
        if remaining_budget == 0 {
            break;
        }
    }
    maximum_cover
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    fn bound(pieces: &[Piece], board: &Board) -> TotalDeficitBound {
        TotalDeficitBound::precompute(
            pieces,
            &(0..pieces.len()).collect::<Vec<_>>(),
            board.height(),
            board.width(),
        )
    }

    #[test]
    fn precomputes_remaining_cells() {
        let single = Piece::from_grid(&[&[true]]);
        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![single, domino];
        let piece_order = vec![0, 1];

        let bound = TotalDeficitBound::precompute(&pieces, &piece_order, 3, 3);

        assert_eq!(bound.remaining_cells(0), 3);
        assert_eq!(bound.remaining_cells(1), 2);
        assert_eq!(bound.remaining_cells(2), 0);
    }

    #[test]
    fn respects_solver_order() {
        let single = Piece::from_grid(&[&[true]]);
        let domino = Piece::from_grid(&[&[true, true]]);
        let tromino = Piece::from_grid(&[&[true, true, true]]);
        let pieces = vec![single, domino, tromino];
        let piece_order = vec![2, 0, 1];

        let bound = TotalDeficitBound::precompute(&pieces, &piece_order, 3, 3);

        assert_eq!(bound.remaining_cells(0), 6);
        assert_eq!(bound.remaining_cells(1), 3);
        assert_eq!(bound.remaining_cells(2), 2);
    }

    #[test]
    fn allows_coverable_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 3);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino, domino];
        let piece_order = vec![0, 1];
        let bound = TotalDeficitBound::precompute(&pieces, &piece_order, 3, 3);

        assert!(bound.allows::<2>(&board, 0));
    }

    #[test]
    fn rejects_uncoverable_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 5);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino];
        let piece_order = vec![0];
        let bound = TotalDeficitBound::precompute(&pieces, &piece_order, 3, 3);

        assert!(!bound.allows::<2>(&board, 0));
    }

    #[test]
    fn allows_exact_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 2);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino];
        let piece_order = vec![0];
        let bound = TotalDeficitBound::precompute(&pieces, &piece_order, 3, 3);

        assert!(bound.allows::<2>(&board, 0));
    }

    #[test]
    fn rejects_isolated_deficit_at_the_exact_boundary() {
        let board = Board::from_grid(&[&[0, 0, 0], &[0, 2, 0], &[0, 0, 0]], 3);
        let pieces = [Piece::from_grid(&[&[true, true]])];

        assert!(!bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn rejects_isolated_deficit_beyond_the_exact_boundary() {
        let board = Board::from_grid(
            &[
                &[2, 0, 0, 0, 2, 0, 0, 0, 0],
                &[0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[2, 0, 0, 0, 2, 0, 0, 0, 0],
            ],
            3,
        );
        let pieces = [
            Piece::from_grid(&[&[true, true, true, true, true]]),
            Piece::from_grid(&[&[true, true, true, true]]),
            Piece::from_grid(&[&[true, true]]),
        ];
        assert_eq!(board.total_deficit(), 8);

        assert!(!bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn allows_isolated_cells_to_share_a_touched_neighbor() {
        let board = Board::from_grid(&[&[0, 2, 0, 0, 0], &[0, 0, 0, 0, 0], &[0, 2, 0, 0, 0]], 3);
        let pieces = [
            Piece::from_grid(&[&[true, true, true, true, true]]),
            Piece::from_grid(&[&[true, true]]),
        ];
        assert_eq!(board.total_deficit(), 4);

        assert!(bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn leaves_disconnected_piece_states_to_other_bounds() {
        let board = Board::from_grid(&[&[1, 0, 0], &[0, 1, 0], &[0, 0, 0]], 3);
        let pieces = [Piece::from_grid(&[&[true, false], &[false, true]])];

        assert!(bound(&pieces, &board).allows::<3>(&board, 0));
    }

    #[test]
    fn sums_all_isolated_deficits_around_a_neighbor() {
        let board = Board::from_grid(
            &[
                &[0, 0, 0, 0, 0],
                &[0, 0, 1, 0, 0],
                &[0, 3, 0, 4, 0],
                &[0, 0, 2, 0, 0],
                &[0, 0, 0, 0, 0],
            ],
            5,
        );
        let isolated = board.valid_mask() & !board.plane(0);

        assert_eq!(
            singleton_neighbor_cover_upper_bound::<5>(&board, isolated, board.valid_mask(), 1),
            10
        );
    }

    fn assert_generated_states_are_allowed<const MODULUS: usize>(
        pieces: &[Piece],
        height: u8,
        width: u8,
    ) {
        fn visit<const MODULUS: usize>(
            bound: &TotalDeficitBound,
            placements: &[Vec<(usize, usize, Bitboard)>],
            board: Board,
            next_piece: usize,
            piece_index: usize,
        ) {
            if next_piece == placements.len() {
                assert!(
                    bound.allows::<MODULUS>(&board, piece_index),
                    "rejected generated solvable state: {board:?}, suffix={piece_index}"
                );
                return;
            }
            for &(_, _, mask) in &placements[next_piece] {
                let mut predecessor = board;
                predecessor.undo_piece(mask);
                visit::<MODULUS>(bound, placements, predecessor, next_piece + 1, piece_index);
            }
        }

        let placements = pieces
            .iter()
            .map(|piece| piece.placements(height, width))
            .collect::<Vec<_>>();
        let bound = TotalDeficitBound::precompute(
            pieces,
            &(0..pieces.len()).collect::<Vec<_>>(),
            height,
            width,
        );
        for piece_index in 0..pieces.len() {
            visit::<MODULUS>(
                &bound,
                &placements,
                Board::new_solved(height, width, MODULUS as u8),
                piece_index,
                piece_index,
            );
        }
    }

    #[test]
    fn allows_exhaustively_generated_solvable_states() {
        let single = Piece::from_grid(&[&[true]]);
        let horizontal_domino = Piece::from_grid(&[&[true, true]]);
        let vertical_domino = Piece::from_grid(&[&[true], &[true]]);
        let corner = Piece::from_grid(&[&[true, true], &[true, false]]);
        let pieces = [corner, horizontal_domino, vertical_domino, single];

        assert_generated_states_are_allowed::<2>(&pieces, 3, 3);
        assert_generated_states_are_allowed::<3>(&pieces, 3, 3);
        assert_generated_states_are_allowed::<4>(&pieces, 3, 3);
        assert_generated_states_are_allowed::<5>(&pieces, 3, 3);
    }
}
