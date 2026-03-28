use crate::core::bitboard::Bitboard;

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
    /// Cached minimum total cell-increments needed to solve.
    /// = sum_{d=1}^{M-1} (M - d) * popcount(planes[d])
    min_flips: u32,
    /// Number of non-zero planes (planes[d] for d>0 that have any bits set).
    active_planes: u8,
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

        let mut min_flips = 0u32;
        let mut active_planes = 0u8;
        for d in 1..m as usize {
            let cnt = planes[d].count_ones();
            min_flips += (m as u32 - d as u32) * cnt;
            if cnt > 0 {
                active_planes += 1;
            }
        }

        Self {
            planes,
            m,
            height: height as u8,
            width: width as u8,
            min_flips,
            active_planes,
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
            min_flips: 0,
            active_planes: 0,
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
    #[inline(always)]
    pub const fn plane(&self, d: u8) -> Bitboard {
        self.planes[d as usize]
    }

    /// Returns true if every cell is 0 (the board is solved).
    #[inline(always)]
    pub fn is_solved(&self) -> bool {
        self.min_flips == 0
    }

    /// Apply a piece placement: increment all cells under `piece_mask` by 1 (mod M).
    /// Each plane rotates: cells at digit d move to digit (d+1) % M.
    #[inline(always)]
    pub fn apply_piece(&mut self, piece_mask: Bitboard) {
        let m = self.m as u32;
        // Incremental min_flips update:
        // Cells at 0 go to 1 (cost 0 → M-1), all others decrease by 1.
        // delta = M * popcount(plane[0] & mask) - popcount(mask)
        let zeros_hit = (self.planes[0] & piece_mask).count_ones();
        self.min_flips = self.min_flips + m * zeros_hit - piece_mask.count_ones();

        let m = m as usize;
        // Hoist the NOT outside the loop: compute the keep-mask once.
        let keep_mask = !piece_mask;
        let top = self.planes[m - 1] & piece_mask;
        let mut i = m - 1;
        while i > 0 {
            let moving = self.planes[i - 1] & piece_mask;
            self.planes[i] = (self.planes[i] & keep_mask) | moving;
            i -= 1;
        }
        self.planes[0] = (self.planes[0] & keep_mask) | top;
    }

    /// Undo a piece placement: decrement all cells under `piece_mask` by 1 (mod M).
    /// Equivalent to applying the piece M-1 more times (since M increments wraps to identity).
    #[inline(always)]
    pub fn undo_piece(&mut self, piece_mask: Bitboard) {
        for _ in 1..self.m {
            self.apply_piece(piece_mask);
        }
    }

    /// Number of non-zero planes (planes with d > 0 that have any bits set).
    /// Each piece placement can reduce this by at most 1.
    pub fn active_planes(&self) -> u8 {
        self.active_planes
    }

    /// Minimum total cell-increments needed to solve (cached, O(1)).
    /// = sum_{d=1}^{M-1} (M - d) * popcount(planes[d])
    #[inline(always)]
    pub fn min_flips_needed(&self) -> u32 {
        self.min_flips
    }

    /// Split jaggedness into (horizontal, vertical) components.
    /// Horizontal = mismatching (r,c)-(r,c+1) pairs. Vertical = (r,c)-(r+1,c) pairs.
    /// Takes precomputed masks to avoid rebuilding them per call.
    #[inline(always)]
    pub fn split_jaggedness(&self, h_mask: Bitboard, h_total: u32, v_mask: Bitboard, v_total: u32) -> (u32, u32) {
        let mut h_matching = 0u32;
        let mut v_matching = 0u32;
        for d in 0..self.m as usize {
            let p = self.planes[d];
            h_matching += (p & (p >> 1) & h_mask).count_ones();
            v_matching += (p & (p >> 15) & v_mask).count_ones();
        }
        (h_total - h_matching, v_total - v_matching)
    }

    /// Count of adjacent cell pairs (horizontal + vertical) with different values.
    /// A solved board has jaggedness = 0.
    pub fn jaggedness(&self) -> u32 {
        let h = self.height as usize;
        let w = self.width as usize;

        // Build masks for valid horizontal and vertical pair origins.
        // Horizontal: cell (r, c) paired with (r, c+1), so c < w-1.
        // Vertical: cell (r, c) paired with (r+1, c), so r < h-1.
        let mut h_mask = Bitboard::ZERO;
        let mut v_mask = Bitboard::ZERO;
        for r in 0..h {
            for c in 0..w {
                let bit = (r * 15 + c) as u32;
                if c + 1 < w {
                    h_mask.set_bit(bit);
                }
                if r + 1 < h {
                    v_mask.set_bit(bit);
                }
            }
        }

        // Count matching pairs: adjacent cells in the same plane.
        let mut matching = 0u32;
        for d in 0..self.m as usize {
            let p = self.planes[d];
            // Horizontal: p & (p >> 1), masked to valid pair origins.
            matching += (p & (p >> 1) & h_mask).count_ones();
            // Vertical: p & (p >> 15), masked to valid pair origins.
            matching += (p & (p >> 15) & v_mask).count_ones();
        }

        // Total valid pairs - matching = jaggedness.
        let total_h = h_mask.count_ones();
        let total_v = v_mask.count_ones();
        (total_h + total_v) - matching
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

    // --- Jaggedness tests ---

    #[test]
    fn test_jaggedness_solved_board() {
        // All zeros: every adjacent pair matches. Jaggedness = 0.
        let board = Board::new_solved(3, 3, 2);
        assert_eq!(board.jaggedness(), 0);
    }

    #[test]
    fn test_jaggedness_uniform_nonzero() {
        // All 1s: every adjacent pair matches. Jaggedness = 0.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 0);
    }

    #[test]
    fn test_jaggedness_single_cell_different() {
        // 3x3, m=2. Only (0,0)=1, rest=0.
        // (0,0) differs from (0,1) horizontally and (1,0) vertically. Jaggedness = 2.
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 2);
    }

    #[test]
    fn test_jaggedness_corner_cell() {
        // 3x3, m=2. Only (2,2)=1 (bottom-right corner).
        // Neighbors: (2,1) horizontal, (1,2) vertical. Both are 0. Jaggedness = 2.
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 2);
    }

    #[test]
    fn test_jaggedness_center_cell() {
        // 3x3, m=2. Only (1,1)=1 (center).
        // 4 neighbors: (0,1), (2,1), (1,0), (1,2) all 0. Jaggedness = 4.
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 4);
    }

    #[test]
    fn test_jaggedness_horizontal_stripe() {
        // 3x3, m=2. Top row all 1, rest 0.
        // Horizontal pairs in row 0: (0,0)-(0,1) match, (0,1)-(0,2) match. +0
        // Vertical pairs from row 0 to 1: 3 pairs, all differ. +3
        // Rest: rows 1-2 all 0, all match. +0
        // Jaggedness = 3.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 3);
    }

    #[test]
    fn test_jaggedness_vertical_stripe() {
        // 3x3, m=2. Left column all 1, rest 0.
        // Vertical: col 0 pairs match. +0
        // Horizontal: row 0: (0,0)=1 vs (0,1)=0 differ. Same for rows 1,2. +3
        // Jaggedness = 3.
        let grid: &[&[u8]] = &[&[1, 0, 0], &[1, 0, 0], &[1, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 3);
    }

    #[test]
    fn test_jaggedness_checkerboard() {
        // 3x3, m=2. Checkerboard pattern: every adjacent pair differs.
        // 0 1 0
        // 1 0 1
        // 0 1 0
        // Horizontal pairs: 3 rows × 2 pairs = 6, all differ. +6
        // Vertical pairs: 2 row-gaps × 3 cols = 6, all differ. +6
        // Jaggedness = 12.
        let grid: &[&[u8]] = &[&[0, 1, 0], &[1, 0, 1], &[0, 1, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 12);
    }

    #[test]
    fn test_jaggedness_m3_two_values() {
        // 3x3, m=3. Two different nonzero values are still "jagged."
        // 1 2 1
        // 0 0 0
        // 0 0 0
        // Row 0 horizontal: (1,2) differ, (2,1) differ. +2
        // Row 0-1 vertical: (1,0), (2,0), (1,0) all differ. +3
        // Rows 1-2: all 0, match. +0
        // Rows 1 horizontal: all 0. +0
        // Jaggedness = 5.
        let grid: &[&[u8]] = &[&[1, 2, 1], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        assert_eq!(board.jaggedness(), 5);
    }

    #[test]
    fn test_jaggedness_m3_all_different() {
        // 3x3, m=3. Each cell a different value (cycling).
        // 0 1 2
        // 1 2 0
        // 2 0 1
        // Horizontal: (0,1)≠(0,2)≠, (1,2)≠, etc. Let me count:
        // Row 0: 0-1 differ, 1-2 differ. +2
        // Row 1: 1-2 differ, 2-0 differ. +2
        // Row 2: 2-0 differ, 0-1 differ. +2
        // Vertical:
        // Col 0: 0-1 differ, 1-2 differ. +2
        // Col 1: 1-2 differ, 2-0 differ. +2
        // Col 2: 2-0 differ, 0-1 differ. +2
        // Jaggedness = 12.
        let grid: &[&[u8]] = &[&[0, 1, 2], &[1, 2, 0], &[2, 0, 1]];
        let board = Board::from_grid(grid, 3);
        assert_eq!(board.jaggedness(), 12);
    }

    #[test]
    fn test_jaggedness_rectangular_board() {
        // 4x3 board, m=2. All 1s except bottom row.
        // 1 1 1
        // 1 1 1
        // 1 1 1
        // 0 0 0
        // Horizontal: all pairs in same rows match. +0
        // Vertical: rows 0-1, 1-2 all match. Row 2-3: 3 pairs differ. +3
        // Jaggedness = 3.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 3);
    }

    #[test]
    fn test_jaggedness_after_apply_piece() {
        // Start solved (jaggedness=0), apply a piece, check jaggedness increases.
        let mut board = Board::new_solved(3, 3, 2);
        assert_eq!(board.jaggedness(), 0);

        // Place a 2x2 piece at (0,0). Board becomes:
        // 1 1 0
        // 1 1 0
        // 0 0 0
        let mut piece = Bitboard::ZERO;
        piece.set_bit(0);
        piece.set_bit(1);
        piece.set_bit(15);
        piece.set_bit(16);
        board.apply_piece(piece);

        // Horizontal: (0,1)-(0,2) differ, (1,1)-(1,2) differ. +2
        // Vertical: (1,0)-(2,0) differ, (1,1)-(2,1) differ. +2
        // Jaggedness = 4.
        assert_eq!(board.jaggedness(), 4);
    }

    #[test]
    fn test_jaggedness_two_isolated_cells() {
        // 3x3, m=2. (0,0)=1 and (2,2)=1, rest=0.
        // (0,0): 2 differing edges.
        // (2,2): 2 differing edges.
        // No overlap. Jaggedness = 4.
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.jaggedness(), 4);
    }
}
