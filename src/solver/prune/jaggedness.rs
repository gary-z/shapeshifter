//! Jaggedness pruning.
//!
//! Measures how "jagged" the board is — the count of adjacent cell pairs with
//! different deficit values. Each piece placement can reduce jaggedness by at
//! most its perimeter. If the remaining perimeter budget is insufficient to
//! smooth out the jaggedness, prune.
//!
//! Uses both circular (symmetric) and directional (asymmetric, M>=3) bounds.

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::piece::Piece;

/// Result of split jaggedness computation.
pub(crate) struct JaggednessResult {
    /// Circular distance h/v: sum of min(|a-b|, M-|a-b|) over adjacent pairs.
    pub circular_h: u32,
    pub circular_v: u32,
    /// Directional (forward) weight h/v: sum of (b-a) mod M over adjacent pairs.
    pub forward_h: u32,
    pub forward_v: u32,
    /// Directional (backward) weight h/v: sum of (a-b) mod M over adjacent pairs.
    pub backward_h: u32,
    pub backward_v: u32,
}

/// Compute jaggedness of a board split into horizontal and vertical components.
///
/// For each adjacent cell pair, computes the circular distance min(|a-b|, M-|a-b|)
/// and directional forward/backward distances. Masks determine which pairs to check.
#[inline(always)]
pub(crate) fn split_jaggedness<const M: usize>(board: &Board, h_mask: Bitboard, v_mask: Bitboard) -> JaggednessResult {
    let mut sh = [Bitboard::ZERO; 5]; // M <= 5
    let mut sv = [Bitboard::ZERO; 5];
    for d in 0..M {
        sh[d] = board.plane(d as u8).shr_1();
        sv[d] = board.plane(d as u8).shr_stride();
    }
    let mut circ_h = 0u32;
    let mut circ_v = 0u32;
    let mut fwd_h = 0u32;
    let mut fwd_v = 0u32;
    let mut bwd_h = 0u32;
    let mut bwd_v = 0u32;
    for d1 in 0..M {
        let p = board.plane(d1 as u8);
        for d2 in 0..M {
            if d1 == d2 { continue; }
            let h_count = (p & sh[d2] & h_mask).count_ones();
            let v_count = (p & sv[d2] & v_mask).count_ones();
            let diff = if d1 > d2 { d1 - d2 } else { d2 - d1 };
            let cw = diff.min(M - diff) as u32;
            circ_h += cw * h_count;
            circ_v += cw * v_count;
            let fw = ((d2 + M - d1) % M) as u32;
            let bw = ((d1 + M - d2) % M) as u32;
            fwd_h += fw * h_count;
            fwd_v += fw * v_count;
            bwd_h += bw * h_count;
            bwd_v += bw * v_count;
        }
    }
    JaggednessResult {
        circular_h: circ_h, circular_v: circ_v,
        forward_h: fwd_h, forward_v: fwd_v,
        backward_h: bwd_h, backward_v: bwd_v,
    }
}

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
                let bit = (r * STRIDE + c) as u32;
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
    pub fn try_prune(&self, j: &JaggednessResult, piece_idx: usize, m: u8) -> bool {
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

    /// Compute total jaggedness (circular H + V) for a board.
    fn jaggedness(board: &Board) -> u32 {
        let h = board.height() as usize;
        let w = board.width() as usize;
        let mut h_mask = Bitboard::ZERO;
        let mut v_mask = Bitboard::ZERO;
        for r in 0..h {
            for c in 0..w {
                let bit = (r * STRIDE + c) as u32;
                if c + 1 < w { h_mask.set_bit(bit); }
                if r + 1 < h { v_mask.set_bit(bit); }
            }
        }
        let result = match board.m() {
            2 => split_jaggedness::<2>(board, h_mask, v_mask),
            3 => split_jaggedness::<3>(board, h_mask, v_mask),
            4 => split_jaggedness::<4>(board, h_mask, v_mask),
            5 => split_jaggedness::<5>(board, h_mask, v_mask),
            _ => unreachable!(),
        };
        result.circular_h + result.circular_v
    }

    #[test]
    fn test_precompute_perimeters() {
        let h_dom = Piece::from_grid(&[&[true, true]]);
        let v_dom = Piece::from_grid(&[&[true], &[true]]);
        assert_eq!(h_dom.h_perimeter(), 2);
        assert_eq!(h_dom.v_perimeter(), 4);
        assert_eq!(v_dom.h_perimeter(), 4);
        assert_eq!(v_dom.v_perimeter(), 2);

        let pieces = vec![h_dom, v_dom];
        let order = vec![0, 1];
        let jp = JaggednessPrune::precompute(&pieces, &order, 3, 3);

        assert_eq!(jp.remaining_h_perimeter[0], 2 + 4);
        assert_eq!(jp.remaining_v_perimeter[0], 4 + 2);
        assert_eq!(jp.remaining_h_perimeter[1], 4);
        assert_eq!(jp.remaining_v_perimeter[1], 2);
        assert_eq!(jp.remaining_h_perimeter[2], 0);
    }

    #[test]
    fn test_try_prune_solved_board() {
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let jp = JaggednessPrune::precompute(&pieces, &order, 3, 3);
        let j = split_jaggedness::<2>(&board, jp.h_mask(), jp.v_mask());
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
        let j = split_jaggedness::<2>(&board, jp.h_mask(), jp.v_mask());
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
        let j = split_jaggedness::<2>(&board, jp.h_mask(), jp.v_mask());
        assert!(!jp.try_prune(&j, 0, 2));
    }

    // --- Jaggedness computation tests (moved from core/board.rs) ---

    #[test]
    fn test_jaggedness_solved_board() {
        let board = Board::new_solved(3, 3, 2);
        assert_eq!(jaggedness(&board), 0);
    }

    #[test]
    fn test_jaggedness_uniform_nonzero() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 0);
    }

    #[test]
    fn test_jaggedness_single_cell_different() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 2);
    }

    #[test]
    fn test_jaggedness_corner_cell() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 2);
    }

    #[test]
    fn test_jaggedness_center_cell() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 4);
    }

    #[test]
    fn test_jaggedness_horizontal_stripe() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn test_jaggedness_vertical_stripe() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[1, 0, 0], &[1, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn test_jaggedness_checkerboard() {
        let grid: &[&[u8]] = &[&[0, 1, 0], &[1, 0, 1], &[0, 1, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 12);
    }

    #[test]
    fn test_jaggedness_m3_two_values() {
        let grid: &[&[u8]] = &[&[1, 2, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        assert_eq!(jaggedness(&board), 5);
    }

    #[test]
    fn test_jaggedness_m3_all_different() {
        let grid: &[&[u8]] = &[&[0, 1, 2], &[1, 2, 0], &[2, 0, 1]];
        let board = Board::from_grid(grid, 3);
        assert_eq!(jaggedness(&board), 12);
    }

    #[test]
    fn test_jaggedness_rectangular_board() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn test_jaggedness_after_apply_piece() {
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
    fn test_jaggedness_two_isolated_cells() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 4);
    }

    #[test]
    fn test_jaggedness_m4_weighted() {
        let grid: &[&[u8]] = &[&[0, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 6);
    }

    #[test]
    fn test_jaggedness_m4_distance_1() {
        let grid: &[&[u8]] = &[&[0, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn test_jaggedness_m4_wrap_around() {
        let grid: &[&[u8]] = &[&[3, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 2);
    }
}
