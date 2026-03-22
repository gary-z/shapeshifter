use crate::bitboard::Bitboard;

/// A puzzle piece defined by its filled cells, anchored at (0, 0).
/// The shape is stored as a Bitboard using the 15-column stride layout.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Piece {
    /// Bitboard with bits set at the piece's filled cells, anchored at (0, 0).
    shape: Bitboard,
    /// Height of the piece's bounding box.
    height: u8,
    /// Width of the piece's bounding box.
    width: u8,
}

impl Piece {
    /// Create a piece from a 2D grid of booleans (true = filled).
    /// The grid must be non-empty, fit within 5x5, and be tight (no empty border rows/cols).
    pub fn from_grid(grid: &[&[bool]]) -> Self {
        let height = grid.len();
        assert!(height >= 1 && height <= 5, "piece height must be in [1, 5]");
        let width = grid[0].len();
        assert!(width >= 1 && width <= 5, "piece width must be in [1, 5]");

        let mut shape = Bitboard::ZERO;
        for (r, row) in grid.iter().enumerate() {
            assert_eq!(row.len(), width, "all rows must have the same width");
            for (c, &filled) in row.iter().enumerate() {
                if filled {
                    shape.set_bit((r * 15 + c) as u32);
                }
            }
        }

        assert!(!shape.is_zero(), "piece must have at least one filled cell");

        Self {
            shape,
            height: height as u8,
            width: width as u8,
        }
    }

    pub const fn shape(&self) -> Bitboard {
        self.shape
    }

    pub const fn height(&self) -> u8 {
        self.height
    }

    pub const fn width(&self) -> u8 {
        self.width
    }

    /// Number of filled cells in the piece.
    pub const fn cell_count(&self) -> u32 {
        self.shape.count_ones()
    }

    /// Max filled cells in any single row of the piece.
    pub fn max_row_thickness(&self) -> u32 {
        let mut max = 0u32;
        for r in 0..self.height as usize {
            let row_bits = (self.shape >> (r as u32 * 15)).limbs[0] & ((1u64 << self.width) - 1);
            let count = row_bits.count_ones();
            if count > max {
                max = count;
            }
        }
        max
    }

    /// Max filled cells in any single column of the piece.
    pub fn max_col_thickness(&self) -> u32 {
        let mut max = 0u32;
        for c in 0..self.width as usize {
            let mut count = 0u32;
            for r in 0..self.height as usize {
                if self.shape.get_bit((r * 15 + c) as u32) {
                    count += 1;
                }
            }
            if count > max {
                max = count;
            }
        }
        max
    }

    /// Max filled cells on any single main diagonal (d = row - col) within the piece.
    pub fn max_diag_thickness(&self) -> u32 {
        let mut counts = [0u32; 9]; // d = pr - pc ranges from -(w-1) to h-1, offset by 4
        for r in 0..self.height as usize {
            for c in 0..self.width as usize {
                if self.shape.get_bit((r * 15 + c) as u32) {
                    let d = (r as i32 - c as i32 + 4) as usize;
                    counts[d] += 1;
                }
            }
        }
        *counts.iter().max().unwrap()
    }

    /// Max filled cells on any single anti-diagonal (d = row + col) within the piece.
    pub fn max_antidiag_thickness(&self) -> u32 {
        let mut counts = [0u32; 9]; // d = pr + pc ranges from 0 to h+w-2
        for r in 0..self.height as usize {
            for c in 0..self.width as usize {
                if self.shape.get_bit((r * 15 + c) as u32) {
                    counts[r + c] += 1;
                }
            }
        }
        *counts.iter().max().unwrap()
    }

    /// The diagonal span of this piece: how many consecutive diagonals it covers.
    /// Equal to height + width - 1.
    pub fn diag_span(&self) -> u8 {
        self.height + self.width - 1
    }

    /// Max filled cells on any single right-leaning zig-zag band, over all placement
    /// positions. A right-leaning band b covers board cells where c/2 == b and r%2 == c%2.
    ///
    /// For a piece at (r0, c0), cell (pr, pc) lands on a right-leaning band iff
    /// (r0+pr)%2 == (c0+pc)%2, i.e. pr%2 ^ pc%2 == r0%2 ^ c0%2.
    /// The board band is (c0+pc)/2. Two eligible cells land on the same band iff
    /// (c0+pc1)/2 == (c0+pc2)/2.
    ///
    /// We try all 4 combinations of (r0%2, c0%2). For each, group eligible cells
    /// by (pc + c0%2) / 2 (the relative band index). Max group size = thickness.
    pub fn max_zigzag_r_thickness(&self) -> u32 {
        let mut max = 0u32;
        for r0_parity in 0u32..2 {
            for c0_parity in 0u32..2 {
                let elig_parity = r0_parity ^ c0_parity;
                let mut band_counts = [0u32; 3]; // max bands: ceil(5/2) = 3
                for pr in 0..self.height as usize {
                    for pc in 0..self.width as usize {
                        if self.shape.get_bit((pr * 15 + pc) as u32) {
                            let cell_parity = (pr as u32 % 2) ^ (pc as u32 % 2);
                            if cell_parity == elig_parity {
                                let band = (pc as u32 + c0_parity) / 2;
                                if (band as usize) < band_counts.len() {
                                    band_counts[band as usize] += 1;
                                }
                            }
                        }
                    }
                }
                for &cnt in band_counts.iter() {
                    if cnt > max {
                        max = cnt;
                    }
                }
            }
        }
        max
    }

    /// Max filled cells on any single left-leaning zig-zag band.
    /// Left-leaning uses the opposite eligibility parity, but since we try all 4
    /// parity combos in the right-leaning version, the max is the same.
    pub fn max_zigzag_l_thickness(&self) -> u32 {
        self.max_zigzag_r_thickness()
    }

    /// Zig-zag span: a piece of width W spans at most floor(W/2) + 1 zig-zag bands.
    pub fn zigzag_span(&self) -> u8 {
        self.width / 2 + 1
    }

    /// Perimeter of the piece: count of edges between filled and unfilled cells
    /// (within the bounding box grid, using cardinal adjacency).
    /// This is a fixed property of the shape, independent of placement.
    pub fn perimeter(&self) -> u32 {
        let s = self.shape;
        // Count internal edges (shared between two filled cells) in each direction.
        // Each shift + AND finds adjacent filled pairs; multiply by 2 since each
        // internal edge removes 2 from the perimeter (one from each side).
        let h_internal = (s & (s >> 1)).count_ones();  // horizontal pairs
        let v_internal = (s & (s >> 15)).count_ones(); // vertical pairs
        s.count_ones() * 4 - (h_internal + v_internal) * 2
    }

    /// Return the piece's shape shifted to board position (row, col).
    pub fn placed_at(&self, row: usize, col: usize) -> Bitboard {
        let offset = (row * 15 + col) as u32;
        self.shape << offset
    }

    /// Iterate over all valid placement positions on a board of the given dimensions.
    /// Returns (row, col, shifted_shape) for each valid position.
    pub fn placements(&self, board_height: u8, board_width: u8) -> Vec<(usize, usize, Bitboard)> {
        let max_row = board_height as usize - self.height as usize;
        let max_col = board_width as usize - self.width as usize;
        let mut result = Vec::new();
        for r in 0..=max_row {
            for c in 0..=max_col {
                result.push((r, c, self.placed_at(r, c)));
            }
        }
        result
    }
}

impl std::fmt::Debug for Piece {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Piece({}x{})", self.height, self.width)?;
        for r in 0..self.height as usize {
            for c in 0..self.width as usize {
                let index = (r * 15 + c) as u32;
                if self.shape.get_bit(index) {
                    write!(f, "#")?;
                } else {
                    write!(f, ".")?;
                }
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_cell_piece() {
        let piece = Piece::from_grid(&[&[true]]);
        assert_eq!(piece.height(), 1);
        assert_eq!(piece.width(), 1);
        assert_eq!(piece.cell_count(), 1);
        assert!(piece.shape().get_bit(0));
    }

    #[test]
    fn test_l_shaped_piece() {
        // ##
        // #.
        let piece = Piece::from_grid(&[&[true, true], &[true, false]]);
        assert_eq!(piece.height(), 2);
        assert_eq!(piece.width(), 2);
        assert_eq!(piece.cell_count(), 3);
        assert!(piece.shape().get_bit(0));        // (0,0)
        assert!(piece.shape().get_bit(1));        // (0,1)
        assert!(piece.shape().get_bit(15));       // (1,0)
        assert!(!piece.shape().get_bit(16));      // (1,1) empty
    }

    #[test]
    fn test_placed_at() {
        let piece = Piece::from_grid(&[&[true, true], &[true, false]]);
        let placed = piece.placed_at(2, 3);
        assert!(placed.get_bit(2 * 15 + 3));     // (2,3)
        assert!(placed.get_bit(2 * 15 + 4));     // (2,4)
        assert!(placed.get_bit(3 * 15 + 3));     // (3,3)
        assert!(!placed.get_bit(3 * 15 + 4));    // (3,4) empty
        assert_eq!(placed.count_ones(), 3);
    }

    #[test]
    fn test_placements_count() {
        // 1x1 piece on a 3x3 board -> 9 placements
        let piece = Piece::from_grid(&[&[true]]);
        let placements = piece.placements(3, 3);
        assert_eq!(placements.len(), 9);
    }

    #[test]
    fn test_placements_2x2_on_3x3() {
        // 2x2 piece on 3x3 board -> 2*2 = 4 placements
        let piece = Piece::from_grid(&[&[true, true], &[true, true]]);
        let placements = piece.placements(3, 3);
        assert_eq!(placements.len(), 4);
    }

    #[test]
    fn test_placements_exact_fit() {
        // 3x3 piece on 3x3 board -> 1 placement
        let piece = Piece::from_grid(&[
            &[true, true, true],
            &[true, true, true],
            &[true, true, true],
        ]);
        let placements = piece.placements(3, 3);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].0, 0);
        assert_eq!(placements[0].1, 0);
    }

    #[test]
    fn test_placements_positions_correct() {
        let piece = Piece::from_grid(&[&[true]]);
        let placements = piece.placements(3, 3);
        // Verify corners
        assert!(placements.iter().any(|&(r, c, _)| r == 0 && c == 0));
        assert!(placements.iter().any(|&(r, c, _)| r == 0 && c == 2));
        assert!(placements.iter().any(|&(r, c, _)| r == 2 && c == 0));
        assert!(placements.iter().any(|&(r, c, _)| r == 2 && c == 2));
    }

    #[test]
    fn test_t_shaped_piece() {
        // ###
        // .#.
        let piece = Piece::from_grid(&[&[true, true, true], &[false, true, false]]);
        assert_eq!(piece.height(), 2);
        assert_eq!(piece.width(), 3);
        assert_eq!(piece.cell_count(), 4);
    }

    #[test]
    fn test_5x5_piece() {
        let row = &[true, true, true, true, true];
        let piece = Piece::from_grid(&[row, row, row, row, row]);
        assert_eq!(piece.height(), 5);
        assert_eq!(piece.width(), 5);
        assert_eq!(piece.cell_count(), 25);
    }

    #[test]
    fn test_debug_output() {
        let piece = Piece::from_grid(&[&[true, false], &[true, true]]);
        let s = format!("{:?}", piece);
        assert!(s.contains("Piece(2x2)"));
        assert!(s.contains("#."));
        assert!(s.contains("##"));
    }

    #[test]
    #[should_panic(expected = "at least one filled cell")]
    fn test_empty_piece() {
        Piece::from_grid(&[&[false, false], &[false, false]]);
    }

    #[test]
    #[should_panic(expected = "piece height")]
    fn test_too_tall() {
        let row = &[true];
        Piece::from_grid(&[row, row, row, row, row, row]);
    }

    // --- Perimeter tests ---

    #[test]
    fn test_perimeter_single_cell() {
        // #  -> perimeter = 4
        let piece = Piece::from_grid(&[&[true]]);
        assert_eq!(piece.perimeter(), 4);
    }

    #[test]
    fn test_perimeter_domino_horizontal() {
        // ##  -> perimeter = 6 (4+4 - 2 shared edges)
        let piece = Piece::from_grid(&[&[true, true]]);
        assert_eq!(piece.perimeter(), 6);
    }

    #[test]
    fn test_perimeter_domino_vertical() {
        // #
        // #  -> perimeter = 6
        let piece = Piece::from_grid(&[&[true], &[true]]);
        assert_eq!(piece.perimeter(), 6);
    }

    #[test]
    fn test_perimeter_2x2_square() {
        // ##
        // ##  -> perimeter = 8
        let piece = Piece::from_grid(&[&[true, true], &[true, true]]);
        assert_eq!(piece.perimeter(), 8);
    }

    #[test]
    fn test_perimeter_l_shape() {
        // ##
        // #.  -> perimeter = 8 (3 cells, 2 internal edges, 3*4 - 2*2 = 8)
        let piece = Piece::from_grid(&[&[true, true], &[true, false]]);
        assert_eq!(piece.perimeter(), 8);
    }

    #[test]
    fn test_perimeter_t_shape() {
        // ###
        // .#.  -> 4 cells, 3 internal edges. 4*4 - 3*2 = 10
        let piece = Piece::from_grid(&[&[true, true, true], &[false, true, false]]);
        assert_eq!(piece.perimeter(), 10);
    }

    #[test]
    fn test_perimeter_straight_line_5() {
        // #####  -> 5 cells, 4 internal edges. 5*4 - 4*2 = 12
        let piece = Piece::from_grid(&[&[true, true, true, true, true]]);
        assert_eq!(piece.perimeter(), 12);
    }

    #[test]
    fn test_perimeter_3x3_square() {
        // ###
        // ###
        // ###  -> 9 cells, 12 internal edges. 9*4 - 12*2 = 12
        let row = &[true, true, true];
        let piece = Piece::from_grid(&[row, row, row]);
        assert_eq!(piece.perimeter(), 12);
    }

    #[test]
    fn test_perimeter_plus_shape() {
        // .#.
        // ###
        // .#.  -> 5 cells, 4 internal edges. 5*4 - 4*2 = 12
        let piece = Piece::from_grid(&[
            &[false, true, false],
            &[true, true, true],
            &[false, true, false],
        ]);
        assert_eq!(piece.perimeter(), 12);
    }

    #[test]
    fn test_perimeter_5x5_full() {
        // 25 cells, 40 internal edges. 25*4 - 40*2 = 20
        let row = &[true, true, true, true, true];
        let piece = Piece::from_grid(&[row, row, row, row, row]);
        assert_eq!(piece.perimeter(), 20);
    }
}
