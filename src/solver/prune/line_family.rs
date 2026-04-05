//! Line-family deficit DP pruning (rows, columns, diagonals, zigzags).
//!
//! For each family of parallel lines (e.g., all rows), compute per-line
//! deficit weights and check via an independent-set DP whether the remaining
//! piece budgets can cover them. Per-line position-aware budgets are enabled
//! for rows/columns when M>=3.

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::piece::Piece;

/// Max number of lines in any family (diagonals on 14x14: 27).
pub(crate) const MAX_LINES: usize = 27;
/// Max number of pieces (n+1 for suffix arrays).
pub(crate) const MAX_PIECES: usize = 37;

/// A family of parallel lines for the deficit DP pruning.
pub(crate) struct LineFamily {
    pub(crate) masks: [Bitboard; MAX_LINES],
    pub(crate) num_lines: usize,
    pub(crate) remaining_budget: [u32; MAX_PIECES],
    pub(crate) suffix_max_span: [u8; MAX_PIECES],
    pub(crate) has_per_line_budget: bool,
    pub(crate) per_line_budget: [[u32; MAX_LINES]; MAX_PIECES],
}

impl LineFamily {
    pub(crate) fn new() -> Self {
        Self {
            masks: [Bitboard::ZERO; MAX_LINES],
            num_lines: 0,
            remaining_budget: [0; MAX_PIECES],
            suffix_max_span: [0; MAX_PIECES],
            has_per_line_budget: false,
            per_line_budget: [[0; MAX_LINES]; MAX_PIECES],
        }
    }
}

/// Check a line family. Returns false if any prune fires.
#[inline(always)]
fn check_line_family(
    board: &Board,
    family: &LineFamily,
    piece_idx: usize,
    m: u8,
) -> bool {
    let gap = family.suffix_max_span[piece_idx] as usize;
    let n = family.num_lines;
    if n == 0 {
        return true;
    }

    let mut weights = [0u32; MAX_LINES];
    for i in 0..n {
        for d in 1..m {
            weights[i] += d as u32 * (board.plane(d) & family.masks[i]).count_ones();
        }
        if family.has_per_line_budget && family.per_line_budget[piece_idx][i] < weights[i] {
            return false;
        }
    }

    if gap > 0 {
        let mut dp = [0u32; MAX_LINES];
        for i in 0..n {
            let take = weights[i] + if i >= gap { dp[i - gap] } else { 0 };
            let skip = if i > 0 { dp[i - 1] } else { 0 };
            dp[i] = take.max(skip);
        }
        if family.remaining_budget[piece_idx] < dp[n - 1] {
            return false;
        }
    }

    true
}

/// Precomputed data for line-family pruning across all 6 families.
pub(crate) struct LineFamilyPrune {
    /// [rows, cols, diags, antidiags, zigzag_r, zigzag_l]
    families: [LineFamily; 6],
}

impl LineFamilyPrune {
    /// Build all 6 line families from pieces, order, and board dimensions.
    pub fn precompute(pieces: &[Piece], order: &[usize], h: u8, w: u8, m: u8) -> Self {
        let bh = h as usize;
        let bw = w as usize;
        let n = pieces.len();
        assert!(n < MAX_PIECES, "too many pieces for LineFamily arrays");

        // --- Rows ---
        let mut rows_family = LineFamily::new();
        rows_family.num_lines = bh;
        rows_family.has_per_line_budget = m >= 3;
        for r in 0..bh {
            for c in 0..bw {
                rows_family.masks[r].set_bit((r * 15 + c) as u32);
            }
        }
        for i in (0..n).rev() {
            let piece = &pieces[order[i]];
            rows_family.remaining_budget[i] = rows_family.remaining_budget[i + 1] + piece.max_row_thickness();
            rows_family.suffix_max_span[i] = rows_family.suffix_max_span[i + 1].max(piece.height());
            if m >= 3 {
                let ph = piece.height() as usize;
                let pw = piece.width() as usize;
                let mut row_thick = [0u32; 5];
                for pr in 0..ph {
                    let row_bits = (piece.shape() >> (pr as u32 * 15)).limbs()[0] & ((1u64 << pw) - 1);
                    row_thick[pr] = row_bits.count_ones();
                }
                for r in 0..bh {
                    let p_min = if r + ph > bh { r + ph - bh } else { 0 };
                    let p_max = r.min(ph - 1);
                    let mut max_t = 0u32;
                    for p in p_min..=p_max {
                        if row_thick[p] > max_t { max_t = row_thick[p]; }
                    }
                    rows_family.per_line_budget[i][r] = rows_family.per_line_budget[i + 1][r] + max_t;
                }
            }
        }

        // --- Cols ---
        let mut cols_family = LineFamily::new();
        cols_family.num_lines = bw;
        cols_family.has_per_line_budget = m >= 3;
        for c in 0..bw {
            for r in 0..bh {
                cols_family.masks[c].set_bit((r * 15 + c) as u32);
            }
        }
        for i in (0..n).rev() {
            let piece = &pieces[order[i]];
            cols_family.remaining_budget[i] = cols_family.remaining_budget[i + 1] + piece.max_col_thickness();
            cols_family.suffix_max_span[i] = cols_family.suffix_max_span[i + 1].max(piece.width());
            if m >= 3 {
                let ph = piece.height() as usize;
                let pw = piece.width() as usize;
                let mut col_thick = [0u32; 5];
                for pc in 0..pw {
                    for pr in 0..ph {
                        if piece.shape().get_bit((pr * 15 + pc) as u32) {
                            col_thick[pc] += 1;
                        }
                    }
                }
                for c in 0..bw {
                    let q_min = if c + pw > bw { c + pw - bw } else { 0 };
                    let q_max = c.min(pw - 1);
                    let mut max_t = 0u32;
                    for q in q_min..=q_max {
                        if col_thick[q] > max_t { max_t = col_thick[q]; }
                    }
                    cols_family.per_line_budget[i][c] = cols_family.per_line_budget[i + 1][c] + max_t;
                }
            }
        }

        // --- Diags ---
        let num_diags = bh + bw - 1;
        let mut diags_family = LineFamily::new();
        diags_family.num_lines = num_diags;
        for r in 0..bh {
            for c in 0..bw {
                diags_family.masks[(r as i32 - c as i32 + bw as i32 - 1) as usize]
                    .set_bit((r * 15 + c) as u32);
            }
        }
        for i in (0..n).rev() {
            diags_family.remaining_budget[i] = diags_family.remaining_budget[i + 1] + pieces[order[i]].max_diag_thickness();
            diags_family.suffix_max_span[i] = diags_family.suffix_max_span[i + 1].max(pieces[order[i]].diag_span());
        }

        // --- Antidiags ---
        let mut antidiags_family = LineFamily::new();
        antidiags_family.num_lines = num_diags;
        for r in 0..bh {
            for c in 0..bw {
                antidiags_family.masks[r + c].set_bit((r * 15 + c) as u32);
            }
        }
        for i in (0..n).rev() {
            antidiags_family.remaining_budget[i] = antidiags_family.remaining_budget[i + 1] + pieces[order[i]].max_antidiag_thickness();
            antidiags_family.suffix_max_span[i] = antidiags_family.suffix_max_span[i + 1].max(pieces[order[i]].diag_span());
        }

        // --- Zigzag right-leaning ---
        let num_zigzag_bands = (bw + 1) / 2;
        let mut zigzag_r_family = LineFamily::new();
        zigzag_r_family.num_lines = num_zigzag_bands;
        for r in 0..bh {
            for c in 0..bw {
                if r % 2 == c % 2 {
                    zigzag_r_family.masks[c / 2].set_bit((r * 15 + c) as u32);
                }
            }
        }
        for i in (0..n).rev() {
            zigzag_r_family.remaining_budget[i] = zigzag_r_family.remaining_budget[i + 1] + pieces[order[i]].max_zigzag_r_thickness();
            zigzag_r_family.suffix_max_span[i] = zigzag_r_family.suffix_max_span[i + 1].max(pieces[order[i]].zigzag_span());
        }

        // --- Zigzag left-leaning ---
        let mut zigzag_l_family = LineFamily::new();
        zigzag_l_family.num_lines = num_zigzag_bands;
        for r in 0..bh {
            for c in 0..bw {
                if r % 2 != c % 2 {
                    zigzag_l_family.masks[c / 2].set_bit((r * 15 + c) as u32);
                }
            }
        }
        for i in (0..n).rev() {
            zigzag_l_family.remaining_budget[i] = zigzag_l_family.remaining_budget[i + 1] + pieces[order[i]].max_zigzag_l_thickness();
            zigzag_l_family.suffix_max_span[i] = zigzag_l_family.suffix_max_span[i + 1].max(pieces[order[i]].zigzag_span());
        }

        Self {
            families: [rows_family, cols_family, diags_family, antidiags_family, zigzag_r_family, zigzag_l_family],
        }
    }

    /// Check row and column families. Returns false to prune.
    #[inline(always)]
    pub fn try_prune_rowcol(&self, board: &Board, piece_idx: usize, m: u8) -> bool {
        for f in &self.families[..2] {
            if !check_line_family(board, f, piece_idx, m) { return false; }
        }
        true
    }

    /// Check diagonal families (diags, antidiags, zigzags). Returns false to prune.
    #[inline(always)]
    pub fn try_prune_diagonal(&self, board: &Board, piece_idx: usize, m: u8) -> bool {
        for f in &self.families[2..] {
            if !check_line_family(board, f, piece_idx, m) { return false; }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    #[test]
    fn test_precompute_creates_6_families() {
        let p = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p];
        let order = vec![0];
        let lf = LineFamilyPrune::precompute(&pieces, &order, 3, 3, 2);

        // Rows: 3 lines, Cols: 3 lines, Diags: 5 lines, Antidiags: 5, Zigzag: 2 each
        assert_eq!(lf.families[0].num_lines, 3); // rows
        assert_eq!(lf.families[1].num_lines, 3); // cols
        assert_eq!(lf.families[2].num_lines, 5); // diags
        assert_eq!(lf.families[3].num_lines, 5); // antidiags
        assert_eq!(lf.families[4].num_lines, 2); // zigzag_r
        assert_eq!(lf.families[5].num_lines, 2); // zigzag_l
    }

    #[test]
    fn test_try_prune_solved_board() {
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p];
        let order = vec![0];
        let lf = LineFamilyPrune::precompute(&pieces, &order, 3, 3, 2);

        assert!(lf.try_prune_rowcol(&board, 0, 2));
        assert!(lf.try_prune_diagonal(&board, 0, 2));
    }

    #[test]
    fn test_per_line_budget_m3() {
        // M=3: per-line budgets should be enabled for rows and cols.
        let p = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p];
        let order = vec![0];
        let lf = LineFamilyPrune::precompute(&pieces, &order, 3, 3, 3);

        assert!(lf.families[0].has_per_line_budget); // rows
        assert!(lf.families[1].has_per_line_budget); // cols
        assert!(!lf.families[2].has_per_line_budget); // diags: no per-line
    }

    #[test]
    fn test_per_line_budget_m2_disabled() {
        let p = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p];
        let order = vec![0];
        let lf = LineFamilyPrune::precompute(&pieces, &order, 3, 3, 2);

        assert!(!lf.families[0].has_per_line_budget); // M=2: disabled
        assert!(!lf.families[1].has_per_line_budget);
    }
}
