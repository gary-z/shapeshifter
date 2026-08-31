//! Rejects states whose remaining pieces cannot cover the board's total deficit.

use crate::core::board::Board;
use crate::core::piece::Piece;

pub(crate) struct TotalDeficitBound {
    remaining_cells: Vec<u32>,
}

impl TotalDeficitBound {
    pub(crate) fn precompute(pieces: &[Piece], piece_order: &[usize]) -> Self {
        let piece_count = pieces.len();
        let mut remaining_cells = vec![0u32; piece_count + 1];
        for piece_index in (0..piece_count).rev() {
            remaining_cells[piece_index] =
                remaining_cells[piece_index + 1] + pieces[piece_order[piece_index]].cell_count();
        }
        Self { remaining_cells }
    }

    #[inline(always)]
    pub(crate) fn remaining_cells(&self, piece_index: usize) -> u32 {
        self.remaining_cells[piece_index]
    }

    #[inline(always)]
    pub(crate) fn allows(&self, board: &Board, piece_index: usize) -> bool {
        self.remaining_cells[piece_index] >= board.total_deficit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    #[test]
    fn precomputes_remaining_cells() {
        let single = Piece::from_grid(&[&[true]]);
        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![single, domino];
        let piece_order = vec![0, 1];

        let bound = TotalDeficitBound::precompute(&pieces, &piece_order);

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

        let bound = TotalDeficitBound::precompute(&pieces, &piece_order);

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
        let bound = TotalDeficitBound::precompute(&pieces, &piece_order);

        assert!(bound.allows(&board, 0));
    }

    #[test]
    fn rejects_uncoverable_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 5);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino];
        let piece_order = vec![0];
        let bound = TotalDeficitBound::precompute(&pieces, &piece_order);

        assert!(!bound.allows(&board, 0));
    }

    #[test]
    fn allows_exact_deficit() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 2);

        let domino = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![domino];
        let piece_order = vec![0];
        let bound = TotalDeficitBound::precompute(&pieces, &piece_order);

        assert!(bound.allows(&board, 0));
    }
}
