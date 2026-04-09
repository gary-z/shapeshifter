use crate::core::bitboard::Bitboard;
use crate::core::STRIDE;

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
                    shape.set_bit((r * STRIDE + c) as u32);
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
        self.shape.count_ones_const()
    }

    /// Perimeter of the piece: count of edges between filled and unfilled cells
    /// (within the bounding box grid, using cardinal adjacency).
    /// This is a fixed property of the shape, independent of placement.
    pub fn perimeter(&self) -> u32 {
        self.h_perimeter() + self.v_perimeter()
    }

    /// Horizontal perimeter: boundary edges between horizontally adjacent cells.
    pub fn h_perimeter(&self) -> u32 {
        let s = self.shape;
        let h_internal = (s & s.shr_1()).count_ones();
        s.count_ones() * 2 - h_internal * 2
    }

    /// Vertical perimeter: boundary edges between vertically adjacent cells.
    pub fn v_perimeter(&self) -> u32 {
        let s = self.shape;
        let v_internal = (s & s.shr_stride()).count_ones();
        s.count_ones() * 2 - v_internal * 2
    }

    /// Return the piece's shape shifted to board position (row, col).
    pub fn placed_at(&self, row: usize, col: usize) -> Bitboard {
        let offset = (row * STRIDE + col) as u32;
        self.shape << offset
    }

    /// Iterate over all valid placement positions on a board of the given dimensions.
    /// Returns (row, col, shifted_shape) for each valid position.
    /// Uses incremental 1-cell shifts instead of absolute shifts for each position.
    pub fn placements(&self, board_height: u8, board_width: u8) -> Vec<(usize, usize, Bitboard)> {
        let max_row = board_height as usize - self.height as usize;
        let max_col = board_width as usize - self.width as usize;
        let mut result = Vec::with_capacity((max_row + 1) * (max_col + 1));
        let mut row_start = self.shape; // shape at (0, 0)
        for r in 0..=max_row {
            let mut mask = row_start;
            for c in 0..=max_col {
                result.push((r, c, mask));
                if c < max_col {
                    mask = mask.shl_1();
                }
            }
            if r < max_row {
                row_start = row_start.shl_stride();
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
                let index = (r * STRIDE + c) as u32;
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
