use crate::core::bitboard::Bitboard;

/// Maximum bits needed to count up to 36 pieces.
const COUNT_BITS: usize = 6;

/// Per-cell coverage counter stored as binary bitboard layers.
/// Layer `i` holds bit `i` of the count at each cell position.
/// Supports O(1) parallel threshold checks across all cells.
#[derive(Clone, Copy)]
pub struct CoverageCounter {
    layers: [Bitboard; COUNT_BITS],
}

impl CoverageCounter {
    pub const ZERO: CoverageCounter = CoverageCounter {
        layers: [Bitboard::ZERO; COUNT_BITS],
    };

    /// Add a reach bitboard (each bit adds 1 to that cell's count).
    /// This is binary addition with carry across layers.
    pub fn add(&mut self, reach: Bitboard) {
        let mut carry = reach;
        for layer in &mut self.layers {
            let new = *layer ^ carry;
            carry = *layer & carry;
            *layer = new;
            if carry.is_zero() {
                break;
            }
        }
    }

    /// Subtract a reach bitboard (each bit subtracts 1 from that cell's count).
    /// Binary subtraction with borrow across layers.
    /// Caller must ensure counts don't go negative.
    pub fn subtract(&mut self, reach: Bitboard) {
        let mut borrow = reach;
        for layer in &mut self.layers {
            let new = *layer ^ borrow;
            borrow = !*layer & borrow;
            *layer = new;
            if borrow.is_zero() {
                break;
            }
        }
    }

    /// Returns a bitboard where bit is set iff that cell's count >= k.
    pub fn coverage_ge(&self, k: u8) -> Bitboard {
        match k {
            0 => !Bitboard::ZERO, // all bits set
            // >= 1: any bit set
            1 => self.layers[0]
                | self.layers[1]
                | self.layers[2]
                | self.layers[3]
                | self.layers[4]
                | self.layers[5],
            // >= 2: bit1+ set (value is at least 2 = 0b10)
            2 => self.layers[1]
                | self.layers[2]
                | self.layers[3]
                | self.layers[4]
                | self.layers[5],
            // >= 3: (bit0 & bit1) | bit2+ (value 3 = 0b11, or 4+ which has bit2+)
            3 => {
                (self.layers[0] & self.layers[1])
                    | self.layers[2]
                    | self.layers[3]
                    | self.layers[4]
                    | self.layers[5]
            }
            // >= 4: bit2+ set (value is at least 4 = 0b100)
            4 => self.layers[2] | self.layers[3] | self.layers[4] | self.layers[5],
            // >= 5: (bit0 & bit2) | (bit1 & bit2) | bit3+ (5=101, 6=110, 7=111, 8+=bit3+)
            5 => {
                ((self.layers[0] | self.layers[1]) & self.layers[2])
                    | self.layers[3]
                    | self.layers[4]
                    | self.layers[5]
            }
            // >= 6: (bit1 & bit2) | bit3+
            6 => {
                (self.layers[1] & self.layers[2])
                    | self.layers[3]
                    | self.layers[4]
                    | self.layers[5]
            }
            // >= 7: (bit0 & bit1 & bit2) | bit3+
            7 => {
                (self.layers[0] & self.layers[1] & self.layers[2])
                    | self.layers[3]
                    | self.layers[4]
                    | self.layers[5]
            }
            // >= 8: bit3+
            8 => self.layers[3] | self.layers[4] | self.layers[5],
            // Higher thresholds: unlikely to matter but handle gracefully.
            _ => Bitboard::ZERO,
        }
    }
}

/// Precompute suffix coverage counters.
/// `result[i]` = binary count of reaches[i] | reaches[i+1] | ... | reaches[n-1].
pub fn precompute_suffix_coverage(reaches: &[Bitboard]) -> Vec<CoverageCounter> {
    let n = reaches.len();
    let mut suffix = vec![CoverageCounter::ZERO; n + 1];
    for i in (0..n).rev() {
        suffix[i] = suffix[i + 1];
        suffix[i].add(reaches[i]);
    }
    suffix
}

/// Check if the board is potentially solvable given remaining coverage.
/// Returns false (prune) if any non-zero cell can't be hit enough times.
pub fn has_sufficient_coverage(
    board: &crate::core::board::Board,
    coverage: &CoverageCounter,
    m: u8,
) -> bool {
    for d in 1..m as usize {
        let plane = board.plane(d as u8);
        if plane.is_zero() {
            continue;
        }
        let needed = m - d as u8;
        let ge = coverage.coverage_ge(needed);
        // Any cell in this plane NOT covered enough times?
        if !(plane & !ge).is_zero() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_counter() {
        let c = CoverageCounter::ZERO;
        assert!(c.coverage_ge(1).is_zero()); // nothing covered
    }

    #[test]
    fn test_add_single() {
        let mut c = CoverageCounter::ZERO;
        let reach = Bitboard::from_bit(5);
        c.add(reach);
        assert!(c.coverage_ge(1).get_bit(5));
        assert!(!c.coverage_ge(2).get_bit(5));
    }

    #[test]
    fn test_add_multiple_same_cell() {
        let mut c = CoverageCounter::ZERO;
        let reach = Bitboard::from_bit(10);
        c.add(reach);
        c.add(reach);
        c.add(reach);
        assert!(c.coverage_ge(1).get_bit(10));
        assert!(c.coverage_ge(2).get_bit(10));
        assert!(c.coverage_ge(3).get_bit(10));
        assert!(!c.coverage_ge(4).get_bit(10));
    }

    #[test]
    fn test_add_different_cells() {
        let mut c = CoverageCounter::ZERO;
        c.add(Bitboard::from_bit(0));
        c.add(Bitboard::from_bit(1));
        assert!(c.coverage_ge(1).get_bit(0));
        assert!(c.coverage_ge(1).get_bit(1));
        assert!(!c.coverage_ge(2).get_bit(0));
        assert!(!c.coverage_ge(2).get_bit(1));
    }

    #[test]
    fn test_coverage_ge_4() {
        let mut c = CoverageCounter::ZERO;
        let reach = Bitboard::from_bit(7);
        for _ in 0..4 {
            c.add(reach);
        }
        assert!(c.coverage_ge(4).get_bit(7));

        let mut c2 = CoverageCounter::ZERO;
        for _ in 0..3 {
            c2.add(reach);
        }
        assert!(!c2.coverage_ge(4).get_bit(7));
    }

    #[test]
    fn test_suffix_coverage() {
        let reaches = vec![
            Bitboard::from_bit(0),
            Bitboard::from_bit(0),
            Bitboard::from_bit(1),
        ];
        let suffix = precompute_suffix_coverage(&reaches);

        // suffix[0]: all 3 pieces. Cell 0 covered 2x, cell 1 covered 1x.
        assert!(suffix[0].coverage_ge(2).get_bit(0));
        assert!(suffix[0].coverage_ge(1).get_bit(1));
        assert!(!suffix[0].coverage_ge(2).get_bit(1));

        // suffix[2]: only piece 2. Cell 1 covered 1x.
        assert!(!suffix[2].coverage_ge(1).get_bit(0));
        assert!(suffix[2].coverage_ge(1).get_bit(1));

        // suffix[3]: no pieces.
        assert!(suffix[3].coverage_ge(1).is_zero());
    }
}
