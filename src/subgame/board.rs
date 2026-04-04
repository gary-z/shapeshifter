use std::simd::{u16x16, cmp::SimdPartialEq, num::SimdUint};

use super::piece::MAX_CELLS;

/// A 1D subgame board: a row of up to 14 cells, each storing its deficit value.
///
/// Cell values are unreduced sums of per-cell deficits from the 2D board.
/// For a row subgame, `cells[r] = sum_{c} deficit(r, c)`.
/// Range: `[0, W * (M-1)]` for row subgame, `[0, H * (M-1)]` for column subgame.
///
/// Layout: `cells` is a `u16x16` SIMD vector for branchless operations.
/// The first `len` lanes hold actual values; the rest are 0.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SubgameBoard {
    /// Per-cell deficit values. Lanes beyond `len` are 0.
    cells: u16x16,
    /// Number of active cells (board dimension: H for row subgame, W for column).
    len: u8,
    /// Total deficit: sum of all cell values (cached for O(1) access).
    total_deficit: u32,
}

impl SubgameBoard {
    /// Create a subgame board from a slice of deficit values.
    ///
    /// # Panics
    /// - If `cells` is empty or longer than `MAX_CELLS`.
    pub fn from_cells(cells: &[u16]) -> Self {
        let len = cells.len();
        assert!(len >= 1 && len <= MAX_CELLS, "board length must be in [1, {MAX_CELLS}]");

        let mut arr = [0u16; MAX_CELLS];
        arr[..len].copy_from_slice(cells);
        let total_deficit: u32 = cells.iter().map(|&v| v as u32).sum();

        Self {
            cells: u16x16::from_array(arr),
            len: len as u8,
            total_deficit,
        }
    }

    /// Create a solved (all-zero) subgame board of the given length.
    pub fn new_solved(len: u8) -> Self {
        assert!(len >= 1 && len as usize <= MAX_CELLS);
        Self {
            cells: u16x16::splat(0),
            len,
            total_deficit: 0,
        }
    }

    /// The SIMD cell vector.
    #[inline(always)]
    pub fn cells(&self) -> u16x16 {
        self.cells
    }

    /// Number of cells.
    #[inline(always)]
    pub fn len(&self) -> u8 {
        self.len
    }

    /// Get the deficit value at cell `i`.
    #[inline(always)]
    pub fn get(&self, i: usize) -> u16 {
        self.cells.to_array()[i]
    }

    /// Total deficit across all cells (cached, O(1)).
    #[inline(always)]
    pub fn total_deficit(&self) -> u32 {
        self.total_deficit
    }

    /// True if every cell has deficit 0.
    #[inline(always)]
    pub fn is_solved(&self) -> bool {
        self.total_deficit == 0
    }

    /// Apply a piece placement: subtract the shifted profile from the board cells.
    /// Returns `true` if the placement is valid (no cell went below 0).
    /// Returns `false` if any cell would underflow — board state is left modified
    /// and the caller must undo.
    ///
    /// Uses SIMD: saturating subtract detects underflow in one comparison.
    #[inline(always)]
    pub fn apply_piece(&mut self, shifted_profile: u16x16) -> bool {
        // Saturating subtract: if cells[i] < profile[i], result saturates to 0.
        // Compare with regular subtract to detect underflow.
        let sat = self.cells.saturating_sub(shifted_profile);
        let regular = self.cells - shifted_profile; // wraps on underflow

        // If saturating != regular, some lane underflowed.
        if sat.simd_ne(regular).any() {
            return false;
        }

        self.cells = regular;
        // Update total deficit: subtract piece cell count.
        let piece_total: u32 = shifted_profile.reduce_sum() as u32;
        self.total_deficit -= piece_total;
        true
    }

    /// Undo a piece placement: add the shifted profile back to the board cells.
    #[inline(always)]
    pub fn undo_piece(&mut self, shifted_profile: u16x16) {
        self.cells += shifted_profile;
        let piece_total: u32 = shifted_profile.reduce_sum() as u32;
        self.total_deficit += piece_total;
    }

    /// Check if a placement would be valid without modifying the board.
    /// Uses SIMD saturating subtract for branchless underflow detection.
    #[inline(always)]
    pub fn can_apply(&self, shifted_profile: u16x16) -> bool {
        let sat = self.cells.saturating_sub(shifted_profile);
        let regular = self.cells - shifted_profile;
        sat.simd_eq(regular).all()
    }

    /// Return the cell values as a slice (up to `len`).
    pub fn as_slice(&self) -> Vec<u16> {
        let arr = self.cells.to_array();
        arr[..self.len as usize].to_vec()
    }
}

impl std::fmt::Debug for SubgameBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let arr = self.cells.to_array();
        write!(f, "SubgameBoard({:?}, deficit={})", &arr[..self.len as usize], self.total_deficit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::piece::SubgamePiece;

    #[test]
    fn test_from_cells() {
        let b = SubgameBoard::from_cells(&[3, 5, 0, 2]);
        assert_eq!(b.len(), 4);
        assert_eq!(b.get(0), 3);
        assert_eq!(b.get(1), 5);
        assert_eq!(b.get(2), 0);
        assert_eq!(b.get(3), 2);
        assert_eq!(b.total_deficit(), 10);
        assert!(!b.is_solved());
    }

    #[test]
    fn test_new_solved() {
        let b = SubgameBoard::new_solved(5);
        assert_eq!(b.len(), 5);
        assert_eq!(b.total_deficit(), 0);
        assert!(b.is_solved());
        for i in 0..5 {
            assert_eq!(b.get(i), 0);
        }
    }

    #[test]
    fn test_apply_piece_valid() {
        let mut b = SubgameBoard::from_cells(&[3, 5, 2]);
        let p = SubgamePiece::from_profile(&[2, 3]);
        let pls = p.placements(3);
        // Place at position 0: subtract [2, 3, 0] from [3, 5, 2]
        let ok = b.apply_piece(pls[0].1);
        assert!(ok);
        assert_eq!(b.get(0), 1);
        assert_eq!(b.get(1), 2);
        assert_eq!(b.get(2), 2);
        assert_eq!(b.total_deficit(), 5);
    }

    #[test]
    fn test_apply_piece_underflow() {
        let mut b = SubgameBoard::from_cells(&[1, 5, 2]);
        let p = SubgamePiece::from_profile(&[2, 3]);
        let pls = p.placements(3);
        // Place at position 0: subtract [2, 3, 0] from [1, 5, 2] -> cell 0 underflows
        let ok = b.apply_piece(pls[0].1);
        assert!(!ok);
    }

    #[test]
    fn test_apply_undo_roundtrip() {
        let original = SubgameBoard::from_cells(&[4, 3, 2, 1]);
        let mut b = original;
        let p = SubgamePiece::from_profile(&[1, 2]);
        let pls = p.placements(4);
        let shifted = pls[1].1; // position 1

        let ok = b.apply_piece(shifted);
        assert!(ok);
        assert_ne!(b, original);

        b.undo_piece(shifted);
        assert_eq!(b, original);
    }

    #[test]
    fn test_can_apply() {
        let b = SubgameBoard::from_cells(&[3, 5, 2]);
        let p = SubgamePiece::from_profile(&[2, 3]);
        let pls = p.placements(3);
        assert!(b.can_apply(pls[0].1));   // [2,3,0] from [3,5,2] OK
        assert!(!b.can_apply(pls[1].1));  // [0,2,3] from [3,5,2] -> cell 2 underflows

        let b2 = SubgameBoard::from_cells(&[3, 5, 4]);
        assert!(b2.can_apply(pls[1].1));  // [0,2,3] from [3,5,4] OK
    }

    #[test]
    fn test_solve_simple() {
        // Board [2, 1], piece with profile [2, 1] at position 0 -> solved
        let mut b = SubgameBoard::from_cells(&[2, 1]);
        let p = SubgamePiece::from_profile(&[2, 1]);
        let pls = p.placements(2);
        let ok = b.apply_piece(pls[0].1);
        assert!(ok);
        assert!(b.is_solved());
    }

    #[test]
    fn test_multiple_applications() {
        let mut b = SubgameBoard::from_cells(&[4, 4, 4]);
        let p = SubgamePiece::from_profile(&[2, 2]);
        let pls = p.placements(3);

        // Place at pos 0: [4,4,4] - [2,2,0] = [2,2,4]
        assert!(b.apply_piece(pls[0].1));
        assert_eq!(b.as_slice(), vec![2, 2, 4]);

        // Place at pos 1: [2,2,4] - [0,2,2] = [2,0,2]
        assert!(b.apply_piece(pls[1].1));
        assert_eq!(b.as_slice(), vec![2, 0, 2]);
    }

    #[test]
    fn test_total_deficit_tracking() {
        let mut b = SubgameBoard::from_cells(&[10, 5, 3]);
        assert_eq!(b.total_deficit(), 18);

        let p = SubgamePiece::from_profile(&[3, 2]);
        let pls = p.placements(3);
        b.apply_piece(pls[0].1); // subtract 5
        assert_eq!(b.total_deficit(), 13);

        b.undo_piece(pls[0].1); // restore
        assert_eq!(b.total_deficit(), 18);
    }

    #[test]
    fn test_as_slice() {
        let b = SubgameBoard::from_cells(&[1, 2, 3]);
        assert_eq!(b.as_slice(), vec![1, 2, 3]);
    }

    #[test]
    fn test_debug_output() {
        let b = SubgameBoard::from_cells(&[3, 1]);
        let s = format!("{:?}", b);
        assert!(s.contains("[3, 1]"));
        assert!(s.contains("deficit=4"));
    }

    #[test]
    #[should_panic(expected = "board length")]
    fn test_empty_board() {
        SubgameBoard::from_cells(&[]);
    }
}
