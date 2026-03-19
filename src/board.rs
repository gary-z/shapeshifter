use crate::bitboard::Bitboard;

/// Maximum value of M (number of digit states per cell).
pub const MAX_M: usize = 5;

/// The board state represented as M bitboards, one per digit value.
/// `planes[d]` has bit (r, c) set iff cell (r, c) has value `d`.
/// The planes are mutually exclusive — each cell appears in exactly one plane.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Board {
    /// One bitboard per digit value 0..M.
    planes: [Bitboard; MAX_M],
    /// Number of digit states. Cells hold values in [0, M).
    m: u8,
    /// Board height.
    height: u8,
    /// Board width.
    width: u8,
}

impl Board {
    /// Create a board from a 2D array of cell values.
    /// Panics if any value >= m, or dimensions are out of range.
    pub fn from_grid(grid: &[&[u8]], m: u8) -> Self {
        let height = grid.len();
        let width = if height > 0 { grid[0].len() } else { 0 };
        assert!(height >= 3 && height <= 14, "height must be in [3, 14]");
        assert!(width >= 3 && width <= 14, "width must be in [3, 14]");
        assert!(m >= 2 && m as usize <= MAX_M, "m must be in [2, {MAX_M}]");

        let mut planes = [Bitboard::ZERO; MAX_M];
        for (r, row) in grid.iter().enumerate() {
            assert_eq!(row.len(), width, "all rows must have the same width");
            for (c, &val) in row.iter().enumerate() {
                assert!(val < m, "cell value {val} >= m ({m})");
                let index = (r * 15 + c) as u32;
                planes[val as usize].set_bit(index);
            }
        }

        Self {
            planes,
            m,
            height: height as u8,
            width: width as u8,
        }
    }

    /// Create a board where all cells are 0.
    pub fn new_solved(height: u8, width: u8, m: u8) -> Self {
        assert!(height >= 3 && height <= 14);
        assert!(width >= 3 && width <= 14);
        assert!(m >= 2 && m as usize <= MAX_M);

        let mut mask = Bitboard::ZERO;
        for r in 0..height as usize {
            for c in 0..width as usize {
                mask.set_bit((r * 15 + c) as u32);
            }
        }

        let mut planes = [Bitboard::ZERO; MAX_M];
        planes[0] = mask;

        Self {
            planes,
            m,
            height,
            width,
        }
    }

    pub const fn m(&self) -> u8 {
        self.m
    }

    pub const fn height(&self) -> u8 {
        self.height
    }

    pub const fn width(&self) -> u8 {
        self.width
    }

    /// Get the value at cell (row, col).
    pub fn get(&self, row: usize, col: usize) -> u8 {
        let index = (row * 15 + col) as u32;
        for d in 0..self.m as usize {
            if self.planes[d].get_bit(index) {
                return d as u8;
            }
        }
        unreachable!("cell ({row}, {col}) not found in any plane");
    }

    /// Get the bitboard plane for digit `d`.
    pub const fn plane(&self, d: u8) -> Bitboard {
        self.planes[d as usize]
    }

    /// Returns true if every cell is 0 (the board is solved).
    pub fn is_solved(&self) -> bool {
        for d in 1..self.m as usize {
            if !self.planes[d].is_zero() {
                return false;
            }
        }
        true
    }

    /// Apply a piece placement: increment all cells under `piece_mask` by 1 (mod M).
    /// Each plane rotates: cells at digit d move to digit (d+1) % M.
    pub fn apply_piece(&mut self, piece_mask: Bitboard) {
        let m = self.m as usize;
        // Cells at value (M-1) that are under the piece wrap to 0.
        let top = self.planes[m - 1] & piece_mask;
        // Shift each plane up: plane[d] loses bits going to d+1, gains bits from d-1.
        // Work backwards to avoid overwriting.
        let mut i = m - 1;
        while i > 0 {
            // Cells in plane[i-1] under the mask move to plane[i].
            let moving = self.planes[i - 1] & piece_mask;
            // plane[i] keeps cells NOT under the mask, gains cells from plane[i-1].
            self.planes[i] = (self.planes[i] & !piece_mask) | moving;
            i -= 1;
        }
        // plane[0] keeps cells NOT under the mask, gains wrapped cells from plane[M-1].
        self.planes[0] = (self.planes[0] & !piece_mask) | top;
    }

    /// Undo a piece placement: decrement all cells under `piece_mask` by 1 (mod M).
    pub fn undo_piece(&mut self, piece_mask: Bitboard) {
        let m = self.m as usize;
        // Cells at value 0 under the piece wrap to M-1.
        let bottom = self.planes[0] & piece_mask;
        // Shift each plane down.
        for i in 0..m - 1 {
            let moving = self.planes[i + 1] & piece_mask;
            self.planes[i] = (self.planes[i] & !piece_mask) | moving;
        }
        self.planes[m - 1] = (self.planes[m - 1] & !piece_mask) | bottom;
    }

    /// Bitboard mask of all valid cells on this board.
    pub fn valid_mask(&self) -> Bitboard {
        let mut mask = Bitboard::ZERO;
        for d in 0..self.m as usize {
            mask |= self.planes[d];
        }
        mask
    }
}

impl std::fmt::Debug for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Board({}x{}, m={})", self.height, self.width, self.m)?;
        for r in 0..self.height as usize {
            for c in 0..self.width as usize {
                if c > 0 {
                    write!(f, " ")?;
                }
                write!(f, "{}", self.get(r, c))?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_grid() -> Board {
        // 3x3 board, m=3
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 1, 0], &[1, 0, 2]];
        Board::from_grid(grid, 3)
    }

    #[test]
    fn test_from_grid_and_get() {
        let board = sample_grid();
        assert_eq!(board.get(0, 0), 0);
        assert_eq!(board.get(0, 1), 1);
        assert_eq!(board.get(0, 2), 2);
        assert_eq!(board.get(1, 0), 2);
        assert_eq!(board.get(1, 1), 1);
        assert_eq!(board.get(1, 2), 0);
        assert_eq!(board.get(2, 0), 1);
        assert_eq!(board.get(2, 1), 0);
        assert_eq!(board.get(2, 2), 2);
    }

    #[test]
    fn test_dimensions() {
        let board = sample_grid();
        assert_eq!(board.height(), 3);
        assert_eq!(board.width(), 3);
        assert_eq!(board.m(), 3);
    }

    #[test]
    fn test_new_solved() {
        let board = Board::new_solved(3, 3, 3);
        assert!(board.is_solved());
        for r in 0..3 {
            for c in 0..3 {
                assert_eq!(board.get(r, c), 0);
            }
        }
    }

    #[test]
    fn test_is_solved() {
        let board = sample_grid();
        assert!(!board.is_solved());

        let solved = Board::new_solved(4, 4, 2);
        assert!(solved.is_solved());
    }

    #[test]
    fn test_apply_piece_single_cell() {
        // 3x3, m=3, all zeros
        let mut board = Board::new_solved(3, 3, 3);
        // Piece covering only (0,0)
        let piece = Bitboard::from_bit(0);

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 1);
        assert_eq!(board.get(0, 1), 0); // untouched

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 2);

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 0); // wrapped around
    }

    #[test]
    fn test_apply_piece_multi_cell() {
        // 3x3, m=2, all zeros
        let mut board = Board::new_solved(3, 3, 2);
        // Piece covering (0,0) and (0,1)
        let mut piece = Bitboard::ZERO;
        piece.set_bit(0);  // (0,0)
        piece.set_bit(1);  // (0,1)

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 1);
        assert_eq!(board.get(0, 1), 1);
        assert_eq!(board.get(0, 2), 0);
    }

    #[test]
    fn test_undo_piece() {
        let mut board = Board::new_solved(3, 3, 3);
        let piece = Bitboard::from_bit(0);

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 1);

        board.undo_piece(piece);
        assert_eq!(board.get(0, 0), 0);
    }

    #[test]
    fn test_undo_piece_wraps() {
        // m=3, cell at 0 -> undo -> should become 2
        let mut board = Board::new_solved(3, 3, 3);
        let piece = Bitboard::from_bit(0);

        board.undo_piece(piece);
        assert_eq!(board.get(0, 0), 2);

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 0);
    }

    #[test]
    fn test_apply_undo_roundtrip() {
        let original = sample_grid();
        let mut board = original;

        let mut piece = Bitboard::ZERO;
        piece.set_bit(0 * 15 + 0);
        piece.set_bit(0 * 15 + 1);
        piece.set_bit(1 * 15 + 0);

        board.apply_piece(piece);
        assert_ne!(board, original);

        board.undo_piece(piece);
        assert_eq!(board, original);
    }

    #[test]
    fn test_valid_mask() {
        let board = Board::new_solved(3, 4, 2);
        let mask = board.valid_mask();
        // 3 rows, 4 cols
        for r in 0..3 {
            for c in 0..4 {
                assert!(mask.get_bit((r * 15 + c) as u32));
            }
        }
        // Outside should be unset
        assert!(!mask.get_bit(4));  // col 4 in row 0
        assert!(!mask.get_bit(14)); // col 14 in row 0
    }

    #[test]
    fn test_plane() {
        let board = sample_grid();
        // plane(0) should have bits set where value is 0
        let p0 = board.plane(0);
        assert!(p0.get_bit(0 * 15 + 0)); // (0,0) = 0
        assert!(p0.get_bit(1 * 15 + 2)); // (1,2) = 0
        assert!(p0.get_bit(2 * 15 + 1)); // (2,1) = 0
        assert!(!p0.get_bit(0 * 15 + 1)); // (0,1) = 1, not 0
    }

    #[test]
    fn test_m5_board() {
        let grid: &[&[u8]] = &[&[0, 1, 2], &[3, 4, 0], &[1, 2, 3]];
        let board = Board::from_grid(grid, 5);
        assert_eq!(board.get(1, 1), 4);

        let mut b = board;
        let piece = Bitboard::from_bit(1 * 15 + 1); // (1,1)
        b.apply_piece(piece);
        assert_eq!(b.get(1, 1), 0); // 4 + 1 = 5 -> 0 mod 5
    }

    #[test]
    fn test_debug_output() {
        let board = Board::new_solved(3, 3, 2);
        let s = format!("{:?}", board);
        assert!(s.contains("Board(3x3, m=2)"));
    }

    #[test]
    #[should_panic(expected = "cell value")]
    fn test_invalid_cell_value() {
        let grid: &[&[u8]] = &[&[0, 1, 3], &[0, 0, 0], &[0, 0, 0]];
        Board::from_grid(grid, 3); // value 3 >= m=3
    }

    #[test]
    #[should_panic(expected = "height")]
    fn test_invalid_height() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0]]; // height 2
        Board::from_grid(grid, 2);
    }
}
