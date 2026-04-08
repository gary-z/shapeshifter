//! Jaggedness pruning.
//!
//! Measures how "jagged" the board is — the count of adjacent cell pairs with
//! different deficit values. Each piece placement can reduce jaggedness by at
//! most its perimeter. If the remaining perimeter budget is insufficient to
//! smooth out the jaggedness, prune.
//!
//! Uses both circular (symmetric) and directional (asymmetric, M>=3) bounds.

use crate::core::bitboard::Bitboard;
use crate::core::piece::Piece;

/// Precomputed data for jaggedness pruning.
pub(crate) struct JaggednessPrune {
    /// Mask of cells that have a horizontal neighbor to the right.
    jagg_h_mask: Bitboard,
    /// Mask of cells that have a vertical neighbor below.
    jagg_v_mask: Bitboard,
    /// remaining_h_perimeter[i] = suffix sum of h_perimeter for pieces [i..n].
    remaining_h_perimeter: Vec<u32>,
    /// remaining_v_perimeter[i] = suffix sum of v_perimeter for pieces [i..n].
    remaining_v_perimeter: Vec<u32>,
}

impl JaggednessPrune {
    /// Build from pieces in solver order and board dimensions.
    pub fn precompute(pieces: &[Piece], order: &[usize], h: u8, w: u8) -> Self {
        let bh = h as usize;
        let bw = w as usize;
        let n = pieces.len();

        let mut jagg_h_mask = Bitboard::ZERO;
        let mut jagg_v_mask = Bitboard::ZERO;
        for r in 0..bh {
            for c in 0..bw {
                let bit = (r * 15 + c) as u32;
                if c + 1 < bw { jagg_h_mask.set_bit(bit); }
                if r + 1 < bh { jagg_v_mask.set_bit(bit); }
            }
        }

        let mut remaining_h_perimeter = vec![0u32; n + 1];
        let mut remaining_v_perimeter = vec![0u32; n + 1];
        for i in (0..n).rev() {
            remaining_h_perimeter[i] = remaining_h_perimeter[i + 1] + pieces[order[i]].h_perimeter();
            remaining_v_perimeter[i] = remaining_v_perimeter[i + 1] + pieces[order[i]].v_perimeter();
        }

        Self { jagg_h_mask, jagg_v_mask, remaining_h_perimeter, remaining_v_perimeter }
    }

    pub fn h_mask(&self) -> Bitboard { self.jagg_h_mask }
    pub fn v_mask(&self) -> Bitboard { self.jagg_v_mask }

    /// Returns false (prune) if remaining perimeter can't smooth out the jaggedness.
    /// Takes pre-computed jaggedness result (shared with MC jaggedness check).
    #[inline(always)]
    pub fn try_prune(&self, j: &crate::core::board::JaggednessResult, piece_idx: usize, m: u8) -> bool {
        let rem_h = self.remaining_h_perimeter[piece_idx];
        let rem_v = self.remaining_v_perimeter[piece_idx];

        if j.circular_h > rem_h || j.circular_v > rem_v {
            return false;
        }

        if m >= 3 {
            let m32 = m as u32;
            if j.forward_h * 2 > m32 * rem_h || j.backward_h * 2 > m32 * rem_h {
                return false;
            }
            if j.forward_v * 2 > m32 * rem_v || j.backward_v * 2 > m32 * rem_v {
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

    #[test]
    fn test_precompute_perimeters() {
        // Horizontal domino [true, true]: 2 cells, 1 h_internal
        //   h_perimeter = 2*2 - 1*2 = 2, v_perimeter = 2*2 - 0*2 = 4
        // Vertical domino: 2 cells, 1 v_internal
        //   h_perimeter = 2*2 - 0*2 = 4, v_perimeter = 2*2 - 1*2 = 2
        let h_dom = Piece::from_grid(&[&[true, true]]);
        let v_dom = Piece::from_grid(&[&[true], &[true]]);
        assert_eq!(h_dom.h_perimeter(), 2);
        assert_eq!(h_dom.v_perimeter(), 4);
        assert_eq!(v_dom.h_perimeter(), 4);
        assert_eq!(v_dom.v_perimeter(), 2);

        let pieces = vec![h_dom, v_dom];
        let order = vec![0, 1];
        let jp = JaggednessPrune::precompute(&pieces, &order, 3, 3);

        assert_eq!(jp.remaining_h_perimeter[0], 2 + 4); // h_dom + v_dom
        assert_eq!(jp.remaining_v_perimeter[0], 4 + 2);
        assert_eq!(jp.remaining_h_perimeter[1], 4); // just v_dom
        assert_eq!(jp.remaining_v_perimeter[1], 2);
        assert_eq!(jp.remaining_h_perimeter[2], 0);
    }

    #[test]
    fn test_try_prune_solved_board() {
        // All-zero board has zero jaggedness → always feasible.
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let jp = JaggednessPrune::precompute(&pieces, &order, 3, 3);

        let j = board.split_jaggedness(jp.h_mask(), jp.v_mask());
        assert!(jp.try_prune(&j, 0, 2));
    }

    #[test]
    fn test_try_prune_uniform_nonzero() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let jp = JaggednessPrune::precompute(&pieces, &order, 3, 3);

        let j = board.split_jaggedness(jp.h_mask(), jp.v_mask());
        assert!(jp.try_prune(&j, 0, 2));
    }

    #[test]
    fn test_try_prune_checkerboard_infeasible() {
        let grid: &[&[u8]] = &[&[0, 1, 0], &[1, 0, 1], &[0, 1, 0]];
        let board = Board::from_grid(grid, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let jp = JaggednessPrune::precompute(&pieces, &order, 3, 3);

        let j = board.split_jaggedness(jp.h_mask(), jp.v_mask());
        assert!(!jp.try_prune(&j, 0, 2));
    }
}
