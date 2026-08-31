//! Bounds differences between horizontally and vertically adjacent cells.
//!
//! A piece cannot smooth more boundaries than its perimeter, which gives a
//! deterministic lower bound on the work needed to solve a jagged board.

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::piece::Piece;

pub(crate) struct Jaggedness {
    /// Sum of min(|a-b|, M-|a-b|) over adjacent pairs.
    pub horizontal_circular_distance: u32,
    pub vertical_circular_distance: u32,
    /// Sum of (b-a) mod M over adjacent pairs.
    pub horizontal_forward_distance: u32,
    pub vertical_forward_distance: u32,
    /// Sum of (a-b) mod M over adjacent pairs.
    pub horizontal_backward_distance: u32,
    pub vertical_backward_distance: u32,
}

#[inline(always)]
pub(crate) fn measure<const MODULUS: usize>(
    board: &Board,
    horizontal_mask: Bitboard,
    vertical_mask: Bitboard,
) -> Jaggedness {
    let mut shifted_horizontal_planes = [Bitboard::ZERO; 5];
    let mut shifted_vertical_planes = [Bitboard::ZERO; 5];
    for value in 0..MODULUS {
        shifted_horizontal_planes[value] = board.plane(value as u8).shr_1();
        shifted_vertical_planes[value] = board.plane(value as u8).shr_stride();
    }
    let mut horizontal_circular_distance = 0u32;
    let mut vertical_circular_distance = 0u32;
    let mut horizontal_forward_distance = 0u32;
    let mut vertical_forward_distance = 0u32;
    let mut horizontal_backward_distance = 0u32;
    let mut vertical_backward_distance = 0u32;
    for first_value in 0..MODULUS {
        let first_value_plane = board.plane(first_value as u8);
        for second_value in 0..MODULUS {
            if first_value == second_value {
                continue;
            }
            let horizontal_pair_count =
                (first_value_plane & shifted_horizontal_planes[second_value] & horizontal_mask)
                    .count_ones();
            let vertical_pair_count =
                (first_value_plane & shifted_vertical_planes[second_value] & vertical_mask)
                    .count_ones();
            let absolute_difference = first_value.abs_diff(second_value);
            let circular_distance = absolute_difference.min(MODULUS - absolute_difference) as u32;
            horizontal_circular_distance += circular_distance * horizontal_pair_count;
            vertical_circular_distance += circular_distance * vertical_pair_count;

            let forward_distance = ((second_value + MODULUS - first_value) % MODULUS) as u32;
            let backward_distance = ((first_value + MODULUS - second_value) % MODULUS) as u32;
            horizontal_forward_distance += forward_distance * horizontal_pair_count;
            vertical_forward_distance += forward_distance * vertical_pair_count;
            horizontal_backward_distance += backward_distance * horizontal_pair_count;
            vertical_backward_distance += backward_distance * vertical_pair_count;
        }
    }
    Jaggedness {
        horizontal_circular_distance,
        vertical_circular_distance,
        horizontal_forward_distance,
        vertical_forward_distance,
        horizontal_backward_distance,
        vertical_backward_distance,
    }
}

pub(crate) struct JaggednessBound {
    horizontal_mask: Bitboard,
    vertical_mask: Bitboard,
    remaining_horizontal_perimeter: Vec<u32>,
    remaining_vertical_perimeter: Vec<u32>,
}

impl JaggednessBound {
    pub(crate) fn precompute(
        pieces: &[Piece],
        piece_order: &[usize],
        height: u8,
        width: u8,
    ) -> Self {
        let board_height = height as usize;
        let board_width = width as usize;
        let piece_count = pieces.len();

        let mut horizontal_mask = Bitboard::ZERO;
        let mut vertical_mask = Bitboard::ZERO;
        for row in 0..board_height {
            for column in 0..board_width {
                let bit = (row * STRIDE + column) as u32;
                if column + 1 < board_width {
                    horizontal_mask.set_bit(bit);
                }
                if row + 1 < board_height {
                    vertical_mask.set_bit(bit);
                }
            }
        }

        let mut remaining_horizontal_perimeter = vec![0u32; piece_count + 1];
        let mut remaining_vertical_perimeter = vec![0u32; piece_count + 1];
        for piece_index in (0..piece_count).rev() {
            remaining_horizontal_perimeter[piece_index] = remaining_horizontal_perimeter
                [piece_index + 1]
                + pieces[piece_order[piece_index]].h_perimeter();
            remaining_vertical_perimeter[piece_index] = remaining_vertical_perimeter
                [piece_index + 1]
                + pieces[piece_order[piece_index]].v_perimeter();
        }

        Self {
            horizontal_mask,
            vertical_mask,
            remaining_horizontal_perimeter,
            remaining_vertical_perimeter,
        }
    }

    pub(crate) fn horizontal_mask(&self) -> Bitboard {
        self.horizontal_mask
    }
    pub(crate) fn vertical_mask(&self) -> Bitboard {
        self.vertical_mask
    }

    #[inline(always)]
    pub(crate) fn allows(&self, jaggedness: &Jaggedness, piece_index: usize, modulus: u8) -> bool {
        let horizontal_perimeter = self.remaining_horizontal_perimeter[piece_index];
        let vertical_perimeter = self.remaining_vertical_perimeter[piece_index];

        if jaggedness.horizontal_circular_distance > horizontal_perimeter
            || jaggedness.vertical_circular_distance > vertical_perimeter
        {
            return false;
        }

        if modulus >= 3 {
            let modulus = modulus as u32;
            if jaggedness.horizontal_forward_distance * 2 > modulus * horizontal_perimeter
                || jaggedness.horizontal_backward_distance * 2 > modulus * horizontal_perimeter
            {
                return false;
            }
            if jaggedness.vertical_forward_distance * 2 > modulus * vertical_perimeter
                || jaggedness.vertical_backward_distance * 2 > modulus * vertical_perimeter
            {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    fn jaggedness(board: &Board) -> u32 {
        let height = board.height() as usize;
        let width = board.width() as usize;
        let mut horizontal_mask = Bitboard::ZERO;
        let mut vertical_mask = Bitboard::ZERO;
        for row in 0..height {
            for column in 0..width {
                let bit = (row * STRIDE + column) as u32;
                if column + 1 < width {
                    horizontal_mask.set_bit(bit);
                }
                if row + 1 < height {
                    vertical_mask.set_bit(bit);
                }
            }
        }
        let result = match board.m() {
            2 => measure::<2>(board, horizontal_mask, vertical_mask),
            3 => measure::<3>(board, horizontal_mask, vertical_mask),
            4 => measure::<4>(board, horizontal_mask, vertical_mask),
            5 => measure::<5>(board, horizontal_mask, vertical_mask),
            _ => unreachable!(),
        };
        result.horizontal_circular_distance + result.vertical_circular_distance
    }

    #[test]
    fn precomputes_perimeters() {
        let horizontal_domino = Piece::from_grid(&[&[true, true]]);
        let vertical_domino = Piece::from_grid(&[&[true], &[true]]);
        assert_eq!(horizontal_domino.h_perimeter(), 2);
        assert_eq!(horizontal_domino.v_perimeter(), 4);
        assert_eq!(vertical_domino.h_perimeter(), 4);
        assert_eq!(vertical_domino.v_perimeter(), 2);

        let pieces = vec![horizontal_domino, vertical_domino];
        let piece_order = vec![0, 1];
        let bound = JaggednessBound::precompute(&pieces, &piece_order, 3, 3);

        assert_eq!(bound.remaining_horizontal_perimeter[0], 2 + 4);
        assert_eq!(bound.remaining_vertical_perimeter[0], 4 + 2);
        assert_eq!(bound.remaining_horizontal_perimeter[1], 4);
        assert_eq!(bound.remaining_vertical_perimeter[1], 2);
        assert_eq!(bound.remaining_horizontal_perimeter[2], 0);
    }

    #[test]
    fn allows_solved_board() {
        let board = Board::new_solved(3, 3, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let pieces = vec![piece];
        let piece_order = vec![0];
        let bound = JaggednessBound::precompute(&pieces, &piece_order, 3, 3);
        let jaggedness = measure::<2>(&board, bound.horizontal_mask(), bound.vertical_mask());
        assert!(bound.allows(&jaggedness, 0, 2));
    }

    #[test]
    fn allows_uniform_nonzero_board() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let pieces = vec![piece];
        let piece_order = vec![0];
        let bound = JaggednessBound::precompute(&pieces, &piece_order, 3, 3);
        let jaggedness = measure::<2>(&board, bound.horizontal_mask(), bound.vertical_mask());
        assert!(bound.allows(&jaggedness, 0, 2));
    }

    #[test]
    fn rejects_checkerboard_with_insufficient_perimeter() {
        let grid: &[&[u8]] = &[&[0, 1, 0], &[1, 0, 1], &[0, 1, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let pieces = vec![piece];
        let piece_order = vec![0];
        let bound = JaggednessBound::precompute(&pieces, &piece_order, 3, 3);
        let jaggedness = measure::<2>(&board, bound.horizontal_mask(), bound.vertical_mask());
        assert!(!bound.allows(&jaggedness, 0, 2));
    }

    #[test]
    fn solved_board_has_no_jaggedness() {
        let board = Board::new_solved(3, 3, 2);
        assert_eq!(jaggedness(&board), 0);
    }

    #[test]
    fn uniform_nonzero_board_has_no_jaggedness() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 0);
    }

    #[test]
    fn measures_different_single_cell() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 2);
    }

    #[test]
    fn measures_different_corner_cell() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 2);
    }

    #[test]
    fn measures_different_center_cell() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 4);
    }

    #[test]
    fn measures_horizontal_stripe() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn measures_vertical_stripe() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[1, 0, 0], &[1, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn measures_checkerboard() {
        let grid: &[&[u8]] = &[&[0, 1, 0], &[1, 0, 1], &[0, 1, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 12);
    }

    #[test]
    fn measures_two_values_with_modulus_three() {
        let grid: &[&[u8]] = &[&[1, 2, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        assert_eq!(jaggedness(&board), 5);
    }

    #[test]
    fn measures_three_values_with_modulus_three() {
        let grid: &[&[u8]] = &[&[0, 1, 2], &[1, 2, 0], &[2, 0, 1]];
        let board = Board::from_grid(grid, 3);
        assert_eq!(jaggedness(&board), 12);
    }

    #[test]
    fn measures_rectangular_board() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn measures_board_after_applying_piece() {
        let mut board = Board::new_solved(3, 3, 2);
        assert_eq!(jaggedness(&board), 0);

        let mut piece = Bitboard::ZERO;
        piece.set_bit(0);
        piece.set_bit(1);
        piece.set_bit(15);
        piece.set_bit(16);
        board.apply_piece(piece);
        assert_eq!(jaggedness(&board), 4);
    }

    #[test]
    fn measures_two_isolated_cells() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 4);
    }

    #[test]
    fn weights_distance_with_modulus_four() {
        let grid: &[&[u8]] = &[&[0, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 6);
    }

    #[test]
    fn measures_unit_distance_with_modulus_four() {
        let grid: &[&[u8]] = &[&[0, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn measures_wraparound_with_modulus_four() {
        let grid: &[&[u8]] = &[&[3, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 2);
    }
}
