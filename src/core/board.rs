use crate::core::bitboard::Bitboard;
use crate::core::STRIDE;

/// Maximum value of M (number of digit states per cell).
pub const MAX_M: usize = 5;

/// Result of split jaggedness computation.
pub struct JaggednessResult {
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
    /// Total deficit: sum of per-cell decrements still needed to solve.
    /// = sum_{d=1}^{M-1} d * popcount(planes[d])
    total_deficit: u32,
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
                let index = (r * crate::core::STRIDE + c) as u32;
                planes[val as usize].set_bit(index);
            }
        }

        let mut total_deficit = 0u32;
        for d in 1..m as usize {
            let cnt = planes[d].count_ones();
            total_deficit += d as u32 * cnt;
        }

        Self {
            planes,
            m,
            height: height as u8,
            width: width as u8,
            total_deficit,
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
                mask.set_bit((r * crate::core::STRIDE + c) as u32);
            }
        }

        let mut planes = [Bitboard::ZERO; MAX_M];
        planes[0] = mask;

        Self {
            planes,
            m,
            height,
            width,
            total_deficit: 0,
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
        let index = (row * crate::core::STRIDE + col) as u32;
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
        self.total_deficit == 0
    }

    /// Apply a piece placement: decrement the deficit of each covered cell by 1 (mod M).
    /// A cell at deficit 0 wraps to M-1 (overshoot penalty).
    /// Each plane rotates: cells at deficit d move to deficit (d-1) mod M.
    #[inline(always)]
    pub fn apply_piece(&mut self, piece_mask: Bitboard) {
        if self.m == 2 {
            // M=2 fast path: toggling deficit 0↔1 is just XOR on both planes.
            self.planes[0] = self.planes[0] ^ piece_mask;
            self.planes[1] = self.planes[1] ^ piece_mask;
            // For M=2, total_deficit = popcount(planes[1]).
            self.total_deficit = self.planes[1].count_ones();
            return;
        }

        let m = self.m as u32;
        // Incremental total_deficit update:
        // Cells at deficit 0 wrap to M-1 (penalty M-1); all others decrease deficit by 1.
        // delta = M * popcount(plane[0] & mask) - popcount(mask)
        let zeros_hit = (self.planes[0] & piece_mask).count_ones();
        self.total_deficit = self.total_deficit + m * zeros_hit - piece_mask.count_ones();

        let m = m as usize;
        // Hoist the NOT outside the loop: compute the keep-mask once.
        let keep_mask = !piece_mask;
        // Rotate down: deficit d → deficit d-1, with deficit 0 wrapping to M-1.
        let bottom = self.planes[0] & piece_mask;
        let mut i = 0;
        while i < m - 1 {
            let moving = self.planes[i + 1] & piece_mask;
            self.planes[i] = (self.planes[i] & keep_mask) | moving;
            i += 1;
        }
        self.planes[m - 1] = (self.planes[m - 1] & keep_mask) | bottom;
    }

    /// Undo a piece placement: restore the deficit of each covered cell (reverse of apply_piece).
    /// Equivalent to applying the piece M-1 more times (since M applications is the identity).
    #[inline(always)]
    pub fn undo_piece(&mut self, piece_mask: Bitboard) {
        if self.m == 2 {
            // M=2: XOR is self-inverse, so undo = apply.
            self.apply_piece(piece_mask);
            return;
        }
        for _ in 1..self.m {
            self.apply_piece(piece_mask);
        }
    }

    /// Total deficit: sum of per-cell hits still needed to reach all-zero (cached, O(1)).
    /// = sum_{d=1}^{M-1} d * popcount(planes[d])
    #[inline(always)]
    pub fn total_deficit(&self) -> u32 {
        self.total_deficit
    }

    /// Split jaggedness into (horizontal, vertical) components.
    /// Horizontal = mismatching (r,c)-(r,c+1) pairs. Vertical = (r,c)-(r+1,c) pairs.
    /// Takes precomputed masks to avoid rebuilding them per call.
    ///
    /// Edges are weighted by circular distance min(|a-b|, M-|a-b|).
    /// For M<=3 all distinct pairs have distance 1, so this equals binary.
    ///
    /// Directional (forward/backward) distances are also computed.
    /// Forward = (b-a) mod M per pair. The bound 2*forward <= M*perimeter is
    /// strictly tighter than circular when adjacencies are direction-biased.
    #[inline(always)]
    pub fn split_jaggedness(&self, h_mask: Bitboard, v_mask: Bitboard) -> JaggednessResult {
        let m = self.m as usize;
        let mut sh = [Bitboard::ZERO; 5]; // M <= 5
        let mut sv = [Bitboard::ZERO; 5];
        for d in 0..m {
            sh[d] = self.planes[d] >> 1;
            sv[d] = self.planes[d] >> STRIDE as u32;
        }
        let mut circ_h = 0u32;
        let mut circ_v = 0u32;
        let mut fwd_h = 0u32;
        let mut fwd_v = 0u32;
        let mut bwd_h = 0u32;
        let mut bwd_v = 0u32;
        for d1 in 0..m {
            let p = self.planes[d1];
            for d2 in 0..m {
                if d1 == d2 { continue; }
                let h_count = (p & sh[d2] & h_mask).count_ones();
                let v_count = (p & sv[d2] & v_mask).count_ones();
                let diff = if d1 > d2 { d1 - d2 } else { d2 - d1 };
                let cw = diff.min(m - diff) as u32;
                circ_h += cw * h_count;
                circ_v += cw * v_count;
                let fw = ((d2 + m - d1) % m) as u32; // (d2-d1) mod M
                let bw = ((d1 + m - d2) % m) as u32; // (d1-d2) mod M
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
        // 3x3, m=3, all zeros. Apply decrements: 0 wraps to M-1=2, then 2→1, then 1→0.
        let mut board = Board::new_solved(3, 3, 3);
        // Piece covering only (0,0)
        let piece = Bitboard::from_bit(0);

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 2); // 0 wraps to M-1
        assert_eq!(board.get(0, 1), 0); // untouched

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 1); // 2 → 1

        board.apply_piece(piece);
        assert_eq!(board.get(0, 0), 0); // 1 → 0, deficit cycles back to 0
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
        assert_eq!(board.get(0, 0), 2); // 0 wraps to M-1

        board.undo_piece(piece);
        assert_eq!(board.get(0, 0), 0);
    }

    #[test]
    fn test_undo_piece_restores_deficit() {
        // m=3, cell at deficit 0, undo = apply M-1=2 times: 0→2→1
        let mut board = Board::new_solved(3, 3, 3);
        let piece = Bitboard::from_bit(0);

        board.undo_piece(piece);
        assert_eq!(board.get(0, 0), 1); // undo increments deficit by 1

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
        assert_eq!(b.get(1, 1), 3); // 4 - 1 = 3 (decrement)
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

    /// Compute jaggedness for a board by building masks and calling split_jaggedness.
    fn jaggedness(board: &Board) -> u32 {
        let h = board.height() as usize;
        let w = board.width() as usize;
        let mut h_mask = Bitboard::ZERO;
        let mut v_mask = Bitboard::ZERO;
        for r in 0..h {
            for c in 0..w {
                let bit = (r * crate::core::STRIDE + c) as u32;
                if c + 1 < w {
                    h_mask.set_bit(bit);
                }
                if r + 1 < h {
                    v_mask.set_bit(bit);
                }
            }
        }
        let result = board.split_jaggedness(h_mask, v_mask);
        result.circular_h + result.circular_v
    }

    #[test]
    fn test_jaggedness_solved_board() {
        // All zeros: every adjacent pair matches. Jaggedness = 0.
        let board = Board::new_solved(3, 3, 2);
        assert_eq!(jaggedness(&board), 0);
    }

    #[test]
    fn test_jaggedness_uniform_nonzero() {
        // All 1s: every adjacent pair matches. Jaggedness = 0.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 0);
    }

    #[test]
    fn test_jaggedness_single_cell_different() {
        // 3x3, m=2. Only (0,0)=1, rest=0.
        // (0,0) differs from (0,1) horizontally and (1,0) vertically. Jaggedness = 2.
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 2);
    }

    #[test]
    fn test_jaggedness_corner_cell() {
        // 3x3, m=2. Only (2,2)=1 (bottom-right corner).
        // Neighbors: (2,1) horizontal, (1,2) vertical. Both are 0. Jaggedness = 2.
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 2);
    }

    #[test]
    fn test_jaggedness_center_cell() {
        // 3x3, m=2. Only (1,1)=1 (center).
        // 4 neighbors: (0,1), (2,1), (1,0), (1,2) all 0. Jaggedness = 4.
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 4);
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
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn test_jaggedness_vertical_stripe() {
        // 3x3, m=2. Left column all 1, rest 0.
        // Vertical: col 0 pairs match. +0
        // Horizontal: row 0: (0,0)=1 vs (0,1)=0 differ. Same for rows 1,2. +3
        // Jaggedness = 3.
        let grid: &[&[u8]] = &[&[1, 0, 0], &[1, 0, 0], &[1, 0, 0]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 3);
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
        assert_eq!(jaggedness(&board), 12);
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
        assert_eq!(jaggedness(&board), 5);
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
        assert_eq!(jaggedness(&board), 12);
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
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn test_jaggedness_after_apply_piece() {
        // Start solved (jaggedness=0), apply a piece, check jaggedness increases.
        let mut board = Board::new_solved(3, 3, 2);
        assert_eq!(jaggedness(&board), 0);

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
        assert_eq!(jaggedness(&board), 4);
    }

    #[test]
    fn test_jaggedness_two_isolated_cells() {
        // 3x3, m=2. (0,0)=1 and (2,2)=1, rest=0.
        // (0,0): 2 differing edges.
        // (2,2): 2 differing edges.
        // No overlap. Jaggedness = 4.
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(jaggedness(&board), 4);
    }

    #[test]
    fn test_jaggedness_m4_weighted() {
        // 3x3, m=4. Values at circular distance 2 get weight 2.
        // 0 2 0
        // 0 0 0
        // 0 0 0
        // (0,0)=0 vs (0,1)=2: min(2, 4-2) = 2. (0,1)=2 vs (0,2)=0: min(2, 4-2) = 2.
        // (0,0)=0 vs (1,0)=0: 0. (0,1)=2 vs (1,1)=0: 2. (0,2)=0 vs (1,2)=0: 0.
        // Weighted jaggedness = 2 + 2 + 2 = 6.
        let grid: &[&[u8]] = &[&[0, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 6);
    }

    #[test]
    fn test_jaggedness_m4_distance_1() {
        // 3x3, m=4. Adjacent values at distance 1.
        // 0 1 0
        // 0 0 0
        // 0 0 0
        // (0,0)-(0,1): min(1,3)=1. (0,1)-(0,2): min(1,3)=1.
        // (0,0)-(1,0): 0. (0,1)-(1,1): min(1,3)=1. (0,2)-(1,2): 0.
        // Weighted jaggedness = 3. Same as binary.
        let grid: &[&[u8]] = &[&[0, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 3);
    }

    #[test]
    fn test_jaggedness_m4_wrap_around() {
        // 3x3, m=4. Value 3 is distance 1 from 0 (wraps around).
        // 3 0 0
        // 0 0 0
        // 0 0 0
        // (0,0)=3 vs (0,1)=0: min(3, 4-3) = 1.
        // (0,0)=3 vs (1,0)=0: min(3, 4-3) = 1.
        // Weighted jaggedness = 2. Same as binary.
        let grid: &[&[u8]] = &[&[3, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 4);
        assert_eq!(jaggedness(&board), 2);
    }
}
