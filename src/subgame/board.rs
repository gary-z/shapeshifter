use std::simd::{u16x16, num::SimdUint};

use super::piece::MAX_CELLS;

/// A 1D subgame board: a row of up to 14 cells, each storing its deficit value.
///
/// Cell values are unreduced sums of per-cell deficits from the 2D board.
/// Values are always non-negative and can exceed M. When a cell at 0 is
/// decremented, it wraps to M-1 (mirroring the full game's modular arithmetic).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SubgameBoard {
    cells: u16x16,
    len: u8,
    m: u8,
    total_deficit: u32,
}

impl SubgameBoard {
    /// Create a subgame board from deficit values and the modulus M.
    pub fn from_cells(cells: &[u16], m: u8) -> Self {
        let len = cells.len();
        assert!(len >= 1 && len <= MAX_CELLS, "board length must be in [1, {MAX_CELLS}]");
        assert!(m >= 2, "M must be >= 2");

        let mut arr = [0u16; MAX_CELLS];
        arr[..len].copy_from_slice(cells);
        let total_deficit: u32 = cells.iter().map(|&v| v as u32).sum();

        Self {
            cells: u16x16::from_array(arr),
            len: len as u8,
            m,
            total_deficit,
        }
    }

    /// Create a solved (all-zero) subgame board.
    pub fn new_solved(len: u8, m: u8) -> Self {
        assert!(len >= 1 && len as usize <= MAX_CELLS);
        Self { cells: u16x16::splat(0), len, m, total_deficit: 0 }
    }

    #[inline(always)]
    pub fn cells(&self) -> u16x16 { self.cells }

    #[inline(always)]
    pub fn len(&self) -> u8 { self.len }

    #[inline(always)]
    pub fn m(&self) -> u8 { self.m }

    #[inline(always)]
    pub fn get(&self, i: usize) -> u16 { self.cells.to_array()[i] }

    #[inline(always)]
    pub fn total_deficit(&self) -> u32 { self.total_deficit }

    #[inline(always)]
    pub fn is_solved(&self) -> bool { self.total_deficit == 0 }

    /// Apply a piece placement with wrapping.
    ///
    /// Subtracts the shifted profile from cells. Any cell that would go below
    /// zero wraps by adding enough multiples of M to stay non-negative.
    /// Returns the wrap_add vector (needed by `undo_piece`).
    #[inline(always)]
    pub fn apply_piece(&mut self, shifted_profile: u16x16) -> u16x16 {
        let m = self.m as u16;
        let m_vec = u16x16::splat(m);

        // shortfall = max(0, profile - cells)
        let shortfall = shifted_profile.saturating_sub(self.cells);

        // wrap_add = ceil(shortfall / M) * M
        let m_minus_1 = u16x16::splat(m - 1);
        let numer = shortfall + m_minus_1;
        let wrap_add = (numer / m_vec) * m_vec;

        self.cells = self.cells + wrap_add - shifted_profile;

        let piece_total: u32 = shifted_profile.reduce_sum() as u32;
        let wrap_total: u32 = wrap_add.reduce_sum() as u32;
        // Add wrap_total first to avoid underflow when piece_total > total_deficit.
        self.total_deficit = self.total_deficit + wrap_total - piece_total;

        wrap_add
    }

    /// Undo a piece placement. `wrap_add` must be the value returned by the
    /// corresponding `apply_piece` call.
    #[inline(always)]
    pub fn undo_piece(&mut self, shifted_profile: u16x16, wrap_add: u16x16) {
        self.cells = self.cells + shifted_profile - wrap_add;

        let piece_total: u32 = shifted_profile.reduce_sum() as u32;
        let wrap_total: u32 = wrap_add.reduce_sum() as u32;
        self.total_deficit = self.total_deficit + piece_total - wrap_total;
    }

    /// Return the cell values as a slice.
    pub fn as_slice(&self) -> Vec<u16> {
        let arr = self.cells.to_array();
        arr[..self.len as usize].to_vec()
    }
}

impl std::fmt::Debug for SubgameBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let arr = self.cells.to_array();
        write!(f, "SubgameBoard({:?}, m={}, deficit={})", &arr[..self.len as usize], self.m, self.total_deficit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::simd::u16x16;
    use super::super::piece::SubgamePiece;

    #[test]
    fn test_from_cells() {
        let b = SubgameBoard::from_cells(&[3, 5, 0, 2], 3);
        assert_eq!(b.len(), 4);
        assert_eq!(b.m(), 3);
        assert_eq!(b.get(0), 3);
        assert_eq!(b.get(1), 5);
        assert_eq!(b.get(2), 0);
        assert_eq!(b.get(3), 2);
        assert_eq!(b.total_deficit(), 10);
        assert!(!b.is_solved());
    }

    #[test]
    fn test_new_solved() {
        let b = SubgameBoard::new_solved(5, 3);
        assert_eq!(b.len(), 5);
        assert_eq!(b.total_deficit(), 0);
        assert!(b.is_solved());
    }

    #[test]
    fn test_apply_piece_no_wrap() {
        let mut b = SubgameBoard::from_cells(&[3, 5, 2], 3);
        let p = SubgamePiece::from_profile(&[2, 3]);
        let pls = p.placements(3);
        let wrap = b.apply_piece(pls[0].1);
        assert_eq!(wrap, u16x16::splat(0)); // no wrapping
        assert_eq!(b.get(0), 1);
        assert_eq!(b.get(1), 2);
        assert_eq!(b.get(2), 2);
        assert_eq!(b.total_deficit(), 5);
    }

    #[test]
    fn test_apply_piece_with_wrap() {
        // Cell at 0, M=3: decrementing by 1 wraps to M-1=2.
        let mut b = SubgameBoard::from_cells(&[0, 5, 2], 3);
        let p = SubgamePiece::from_profile(&[1, 1]);
        let pls = p.placements(3);
        let wrap = b.apply_piece(pls[0].1);
        assert_eq!(b.get(0), 2); // wrapped: 0 + 3 - 1 = 2
        assert_eq!(b.get(1), 4);
        assert_eq!(b.get(2), 2);
        // total_deficit: was 7, subtract 2 (piece cells), add 3 (one wrap) = 8
        assert_eq!(b.total_deficit(), 8);
        assert_eq!(wrap.to_array()[0], 3); // one wrap of M=3
    }

    #[test]
    fn test_apply_piece_wrap_m5() {
        // M=5, cell at 1, hit by 2 → 1 - 2 underflows → 1 + 5 - 2 = 4
        let mut b = SubgameBoard::from_cells(&[1, 10], 5);
        let p = SubgamePiece::from_profile(&[2, 1]);
        let pls = p.placements(2);
        b.apply_piece(pls[0].1);
        assert_eq!(b.get(0), 4); // 1 + 5 - 2 = 4
        assert_eq!(b.get(1), 9); // 10 - 1 = 9
        // deficit: was 11, -3 (piece) +5 (wrap) = 13
        assert_eq!(b.total_deficit(), 13);
    }

    #[test]
    fn test_apply_undo_roundtrip() {
        let original = SubgameBoard::from_cells(&[4, 3, 2, 1], 3);
        let mut b = original;
        let p = SubgamePiece::from_profile(&[1, 2]);
        let pls = p.placements(4);
        let shifted = pls[1].1; // position 1

        let wrap = b.apply_piece(shifted);
        assert_ne!(b, original);

        b.undo_piece(shifted, wrap);
        assert_eq!(b, original);
    }

    #[test]
    fn test_apply_undo_with_wrap() {
        let original = SubgameBoard::from_cells(&[0, 5, 2], 3);
        let mut b = original;
        let p = SubgamePiece::from_profile(&[1]);
        let pls = p.placements(3);

        let wrap = b.apply_piece(pls[0].1); // cell 0: 0 → 2 (wrap)
        assert_eq!(b.get(0), 2);

        b.undo_piece(pls[0].1, wrap); // restore
        assert_eq!(b, original);
    }

    #[test]
    fn test_solve_simple() {
        let mut b = SubgameBoard::from_cells(&[2, 1], 3);
        let p = SubgamePiece::from_profile(&[2, 1]);
        let pls = p.placements(2);
        b.apply_piece(pls[0].1);
        assert!(b.is_solved());
    }

    #[test]
    #[should_panic(expected = "board length")]
    fn test_empty_board() {
        SubgameBoard::from_cells(&[], 2);
    }
}
