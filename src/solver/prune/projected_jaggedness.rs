//! Jaggedness pruning on the row and column projections of the board.
//!
//! Summing cell deficits modulo M along every row (or column) is a homomorphism
//! from the full game to a 1D game. A full solution therefore induces a
//! solution of both projected games.
//!
//! The general cyclic-distance bound is sound for every M, but benchmarks only
//! found useful pruning for M=2. In that case each projection fits in a u16,
//! applying a projected piece is XOR, and boundary-inclusive jaggedness is
//! `popcount(bits ^ (bits << 1))`. This is the specialized form used here.

use crate::core::STRIDE;
use crate::core::board::Board;
use crate::core::piece::Piece;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProjectedState {
    rows: u16,
    cols: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProjectedProfile {
    parity_mask: u8,
}

/// For M=2, circular, forward, and backward variation are all just the
/// number of transitions in the zero-padded binary vector.
#[inline(always)]
fn binary_variation(bits: u16) -> u32 {
    (bits ^ (bits << 1)).count_ones()
}

fn profiles(piece: &Piece) -> (ProjectedProfile, ProjectedProfile) {
    let height = piece.height() as usize;
    let width = piece.width() as usize;
    let mut rows = ProjectedProfile::default();
    let mut cols = ProjectedProfile::default();

    for r in 0..height {
        for c in 0..width {
            if piece.shape().get_bit((r * STRIDE + c) as u32) {
                rows.parity_mask ^= 1 << r;
                cols.parity_mask ^= 1 << c;
            }
        }
    }

    (rows, cols)
}

impl ProjectedState {
    pub(crate) fn from_board(board: &Board) -> Self {
        let mut state = Self::default();
        let m = board.m();
        if m != 2 {
            return state;
        }

        for r in 0..board.height() as usize {
            for c in 0..board.width() as usize {
                let value = board.get(r, c);
                state.rows ^= (value as u16) << r;
                state.cols ^= (value as u16) << c;
            }
        }

        state
    }

    #[inline(always)]
    pub(crate) fn apply_piece(
        &mut self,
        prune: &ProjectedJaggednessPrune,
        piece_idx: usize,
        row: usize,
        col: usize,
    ) {
        if prune.m != 2 {
            return;
        }
        self.rows ^= (prune.row_profiles[piece_idx].parity_mask as u16) << row;
        self.cols ^= (prune.col_profiles[piece_idx].parity_mask as u16) << col;
    }
}

pub(crate) struct ProjectedJaggednessPrune {
    row_profiles: Vec<ProjectedProfile>,
    col_profiles: Vec<ProjectedProfile>,
    remaining_row: Vec<u32>,
    remaining_col: Vec<u32>,
    first_row_useful: usize,
    first_col_useful: usize,
    m: u8,
}

impl ProjectedJaggednessPrune {
    pub(crate) fn precompute(
        pieces: &[Piece],
        order: &[usize],
        height: u8,
        width: u8,
        m: u8,
    ) -> Self {
        let n = order.len();
        let mut row_profiles = Vec::with_capacity(n);
        let mut col_profiles = Vec::with_capacity(n);

        if m == 2 {
            for &piece_idx in order {
                let (rows, cols) = profiles(&pieces[piece_idx]);
                row_profiles.push(rows);
                col_profiles.push(cols);
            }
        }

        let mut remaining_row = vec![0u32; n + 1];
        let mut remaining_col = vec![0u32; n + 1];
        if m == 2 {
            for i in (0..n).rev() {
                remaining_row[i] =
                    remaining_row[i + 1] + binary_variation(row_profiles[i].parity_mask as u16);
                remaining_col[i] =
                    remaining_col[i + 1] + binary_variation(col_profiles[i].parity_mask as u16);
            }
        }

        let max_row_variation = height as u32 + (height as u32 & 1);
        let max_col_variation = width as u32 + (width as u32 & 1);
        let first_row_useful = remaining_row
            .iter()
            .position(|&v| v < max_row_variation)
            .unwrap_or(n);
        let first_col_useful = remaining_col
            .iter()
            .position(|&v| v < max_col_variation)
            .unwrap_or(n);

        Self {
            row_profiles,
            col_profiles,
            remaining_row,
            remaining_col,
            first_row_useful,
            first_col_useful,
            m,
        }
    }

    #[inline(always)]
    pub(crate) fn try_prune(&self, state: &ProjectedState, piece_idx: usize) -> bool {
        if self.m != 2 {
            return true;
        }

        if piece_idx >= self.first_row_useful {
            let row = binary_variation(state.rows);
            if row > self.remaining_row[piece_idx] {
                return false;
            }
        }

        piece_idx < self.first_col_useful
            || binary_variation(state.cols) <= self.remaining_col[piece_idx]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_variation_includes_zero_boundaries() {
        assert_eq!(binary_variation(0b1_0110), 4);
        assert_eq!(binary_variation(0b11), 2);
        assert_eq!(binary_variation(0), 0);
    }

    #[test]
    fn piece_projection_preserves_row_and_column_parity() {
        let piece = Piece::from_grid(&[&[true, true], &[true, false]]);
        let (rows, cols) = profiles(&piece);
        assert_eq!(rows.parity_mask, 0b10);
        assert_eq!(cols.parity_mask, 0b10);
    }

    #[test]
    fn incremental_projection_matches_board_projection() {
        let grid: &[&[u8]] = &[&[0, 1, 1], &[1, 1, 0], &[1, 0, 1]];
        let mut board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true, true], &[true, false]]);
        let prune = ProjectedJaggednessPrune::precompute(&[piece], &[0], 3, 3, 2);
        let mut projected = ProjectedState::from_board(&board);

        board.apply_piece(piece.placed_at(1, 1));
        projected.apply_piece(&prune, 0, 1, 1);

        assert_eq!(projected, ProjectedState::from_board(&board));
    }

    #[test]
    fn prunes_when_one_profile_cannot_repair_projected_variation() {
        let piece = Piece::from_grid(&[&[true]]);
        let prune = ProjectedJaggednessPrune::precompute(&[piece], &[0], 5, 5, 2);
        let mut state = ProjectedState::default();
        state.rows = 0b01010;

        // The row projection has circular variation 4, while a translated
        // single-cell profile can contribute only 2.
        assert!(!prune.try_prune(&state, 0));
    }
}
