use std::simd::u16x16;

/// Maximum number of cells in a subgame (max board dimension).
pub const MAX_CELLS: usize = 16;

/// A 1D subgame piece: the projection of a 2D piece onto one axis.
///
/// The profile stores how many 2D cells the piece covers in each row (or column).
/// For example, an L-shaped piece `##` / `#.` has row profile `[2, 1]` and
/// column profile `[2, 1]`.
///
/// Layout: `profile` is a `u16x16` SIMD vector, padded with zeros beyond `len`.
/// This allows SIMD subtraction of the profile from a subgame board in one op.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SubgamePiece {
    /// Per-position contribution. `profile[j]` = number of 2D cells at offset j.
    /// Entries beyond `len` are 0.
    profile: u16x16,
    /// Number of positions in the profile (1..=5 for pieces up to 5 tall/wide).
    len: u8,
    /// Total cell count: sum of all profile entries.
    cell_count: u16,
}

impl SubgamePiece {
    /// Create a subgame piece from a slice of per-position cell counts.
    ///
    /// # Panics
    /// - If `profile` is empty or longer than `MAX_CELLS`.
    /// - If any entry is 0 (projections of valid pieces are always >= 1).
    pub fn from_profile(profile: &[u16]) -> Self {
        let len = profile.len();
        assert!(len >= 1 && len <= MAX_CELLS, "profile length must be in [1, {MAX_CELLS}]");
        assert!(profile.iter().all(|&v| v >= 1), "profile entries must be >= 1");

        let mut arr = [0u16; MAX_CELLS];
        arr[..len].copy_from_slice(profile);
        let cell_count: u16 = profile.iter().sum();

        Self {
            profile: u16x16::from_array(arr),
            len: len as u8,
            cell_count,
        }
    }

    /// The SIMD profile vector (zero-padded to 16 lanes).
    #[inline(always)]
    pub fn profile(&self) -> u16x16 {
        self.profile
    }

    /// Number of positions in the profile.
    #[inline(always)]
    pub fn len(&self) -> u8 {
        self.len
    }

    /// Total cell count (sum of profile entries).
    #[inline(always)]
    pub fn cell_count(&self) -> u16 {
        self.cell_count
    }

    /// Get the profile value at index `i`.
    #[inline(always)]
    pub fn get(&self, i: usize) -> u16 {
        self.profile.to_array()[i]
    }

    /// Generate all valid placements on a subgame board of length `board_len`.
    /// Returns a vec of `(position, shifted_profile)` where shifted_profile is
    /// the profile SIMD vector shifted to start at `position`.
    pub fn placements(&self, board_len: u8) -> Vec<(usize, u16x16)> {
        if self.len > board_len {
            return Vec::new();
        }
        let max_pos = (board_len - self.len) as usize;
        let profile_arr = self.profile.to_array();
        let mut result = Vec::with_capacity(max_pos + 1);
        for pos in 0..=max_pos {
            let mut shifted = [0u16; MAX_CELLS];
            shifted[pos..pos + self.len as usize]
                .copy_from_slice(&profile_arr[..self.len as usize]);
            result.push((pos, u16x16::from_array(shifted)));
        }
        result
    }
}

impl std::fmt::Debug for SubgamePiece {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let arr = self.profile.to_array();
        write!(f, "SubgamePiece({:?})", &arr[..self.len as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_cell() {
        let p = SubgamePiece::from_profile(&[1]);
        assert_eq!(p.len(), 1);
        assert_eq!(p.cell_count(), 1);
        assert_eq!(p.get(0), 1);
        assert_eq!(p.get(1), 0); // padding
    }

    #[test]
    fn test_multi_entry_profile() {
        let p = SubgamePiece::from_profile(&[2, 1, 3]);
        assert_eq!(p.len(), 3);
        assert_eq!(p.cell_count(), 6);
        assert_eq!(p.get(0), 2);
        assert_eq!(p.get(1), 1);
        assert_eq!(p.get(2), 3);
    }

    #[test]
    fn test_placements_exact_fit() {
        // Profile [2, 1] on a board of length 2 -> 1 placement at pos 0
        let p = SubgamePiece::from_profile(&[2, 1]);
        let pls = p.placements(2);
        assert_eq!(pls.len(), 1);
        assert_eq!(pls[0].0, 0);
        let arr = pls[0].1.to_array();
        assert_eq!(arr[0], 2);
        assert_eq!(arr[1], 1);
    }

    #[test]
    fn test_placements_multiple() {
        // Profile [1] on board of length 5 -> 5 placements
        let p = SubgamePiece::from_profile(&[1]);
        let pls = p.placements(5);
        assert_eq!(pls.len(), 5);
        for (i, (pos, shifted)) in pls.iter().enumerate() {
            assert_eq!(*pos, i);
            let arr = shifted.to_array();
            assert_eq!(arr[i], 1);
            // Other entries should be 0
            for j in 0..5 {
                if j != i {
                    assert_eq!(arr[j], 0);
                }
            }
        }
    }

    #[test]
    fn test_placements_shifted() {
        let p = SubgamePiece::from_profile(&[3, 2]);
        let pls = p.placements(4);
        assert_eq!(pls.len(), 3); // positions 0, 1, 2

        // Position 2: profile at indices 2, 3
        let arr = pls[2].1.to_array();
        assert_eq!(arr[0], 0);
        assert_eq!(arr[1], 0);
        assert_eq!(arr[2], 3);
        assert_eq!(arr[3], 2);
    }

    #[test]
    fn test_placements_too_large() {
        let p = SubgamePiece::from_profile(&[1, 1, 1]);
        let pls = p.placements(2);
        assert_eq!(pls.len(), 0);
    }

    #[test]
    fn test_max_profile() {
        // Maximum: 5 entries (piece height/width up to 5)
        let p = SubgamePiece::from_profile(&[5, 4, 3, 2, 1]);
        assert_eq!(p.len(), 5);
        assert_eq!(p.cell_count(), 15);
    }

    #[test]
    fn test_debug_output() {
        let p = SubgamePiece::from_profile(&[2, 1]);
        let s = format!("{:?}", p);
        assert!(s.contains("[2, 1]"));
    }

    #[test]
    #[should_panic(expected = "profile length")]
    fn test_empty_profile() {
        SubgamePiece::from_profile(&[]);
    }

    #[test]
    #[should_panic(expected = "profile entries must be >= 1")]
    fn test_zero_entry() {
        SubgamePiece::from_profile(&[1, 0, 2]);
    }

    #[test]
    fn test_equality() {
        let a = SubgamePiece::from_profile(&[2, 1]);
        let b = SubgamePiece::from_profile(&[2, 1]);
        let c = SubgamePiece::from_profile(&[1, 2]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
