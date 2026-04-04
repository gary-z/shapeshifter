use std::simd::{u16x16, cmp::SimdOrd, num::SimdUint};

use crate::core::board::Board;
use crate::core::piece::Piece;

use super::board::SubgameBoard;
use super::generate::{board_col_deficits, board_row_deficits, piece_col_profile, piece_row_profile};
use super::piece::SubgamePiece;

/// Maximum subgame nodes to explore per feasibility check before bailing out
/// (conservatively assuming feasible). Prevents the subgame check from
/// becoming a bottleneck on nodes where the subgame is hard but feasible.
const FEASIBILITY_NODE_BUDGET: u64 = 10_000;

/// Precomputed subgame data for all pieces in solver order.
///
/// Built once during solver precomputation. Provides O(1) lookup of shifted
/// profiles by `(piece_index, position)` so that [`super::state::SubgameState`]
/// can be updated incrementally during the main search with no allocation.
///
/// Also contains all data needed for zero-allocation subgame feasibility
/// checks: precomputed placements, suffix max-contribution vectors, and
/// suffix remaining-cell counts.
pub struct SubgameData {
    /// Initial row subgame board (before any pieces are placed).
    row_board: SubgameBoard,
    /// Initial column subgame board (before any pieces are placed).
    col_board: SubgameBoard,
    /// Row profile for each piece (in solver piece order).
    row_profiles: Vec<SubgamePiece>,
    /// Column profile for each piece (in solver piece order).
    col_profiles: Vec<SubgamePiece>,
    /// `row_shifted[piece_idx]` maps row position → shifted u16x16 profile.
    row_shifted: Vec<Vec<u16x16>>,
    /// `col_shifted[piece_idx]` maps col position → shifted u16x16 profile.
    col_shifted: Vec<Vec<u16x16>>,
    /// Precomputed placements for each piece in the row subgame:
    /// `row_placements[i]` = Vec of `(position, shifted_profile)`.
    row_placements: Vec<Vec<(usize, u16x16)>>,
    /// Precomputed placements for each piece in the col subgame.
    col_placements: Vec<Vec<(usize, u16x16)>>,
    /// Suffix max-contribution per cell for row subgame.
    /// `row_max_contrib_suffix[i]` = element-wise sum of per-piece max
    /// contributions from piece `i` onward.
    row_max_contrib_suffix: Vec<u16x16>,
    /// Suffix max-contribution per cell for col subgame.
    col_max_contrib_suffix: Vec<u16x16>,
    /// Suffix sum of piece cell counts for row profiles.
    /// `row_remaining_cells[i]` = total cells from piece `i` onward.
    row_remaining_cells: Vec<u32>,
    /// Suffix sum of piece cell counts for col profiles.
    col_remaining_cells: Vec<u32>,
}

impl SubgameData {
    /// Build subgame data from the full board and pieces in solver order.
    ///
    /// `order[i]` maps solver piece index → original piece index.
    /// The resulting profiles and shifted tables follow solver piece order.
    pub fn build(board: &Board, pieces: &[Piece], order: &[usize]) -> Self {
        let row_board = board_row_deficits(board);
        let col_board = board_col_deficits(board);
        let board_h = board.height();
        let board_w = board.width();

        let n = order.len();
        let mut row_profiles = Vec::with_capacity(n);
        let mut col_profiles = Vec::with_capacity(n);
        let mut row_shifted = Vec::with_capacity(n);
        let mut col_shifted = Vec::with_capacity(n);
        let mut row_placements = Vec::with_capacity(n);
        let mut col_placements = Vec::with_capacity(n);

        for &orig_idx in order {
            let piece = &pieces[orig_idx];

            let rp = piece_row_profile(piece);
            let row_pls = rp.placements(board_h);
            row_shifted.push(row_pls.iter().map(|&(_, shifted)| shifted).collect());
            row_placements.push(row_pls);
            row_profiles.push(rp);

            let cp = piece_col_profile(piece);
            let col_pls = cp.placements(board_w);
            col_shifted.push(col_pls.iter().map(|&(_, shifted)| shifted).collect());
            col_placements.push(col_pls);
            col_profiles.push(cp);
        }

        // Build suffix max-contribution vectors for count-sat pruning.
        let row_max_contrib_suffix = Self::build_max_contrib_suffix(&row_placements);
        let col_max_contrib_suffix = Self::build_max_contrib_suffix(&col_placements);

        // Build suffix remaining-cell counts.
        let row_remaining_cells = Self::build_remaining_cells(&row_profiles);
        let col_remaining_cells = Self::build_remaining_cells(&col_profiles);

        Self {
            row_board,
            col_board,
            row_profiles,
            col_profiles,
            row_shifted,
            col_shifted,
            row_placements,
            col_placements,
            row_max_contrib_suffix,
            col_max_contrib_suffix,
            row_remaining_cells,
            col_remaining_cells,
        }
    }

    /// Build suffix max-contribution vectors from precomputed placements.
    fn build_max_contrib_suffix(placements: &[Vec<(usize, u16x16)>]) -> Vec<u16x16> {
        let n = placements.len();
        let mut suffix = vec![u16x16::splat(0); n + 1];
        for i in (0..n).rev() {
            let mut max_vec = u16x16::splat(0);
            for &(_pos, shifted) in &placements[i] {
                max_vec = max_vec.simd_max(shifted);
            }
            suffix[i] = suffix[i + 1] + max_vec;
        }
        suffix
    }

    /// Build suffix remaining-cell counts from profiles.
    fn build_remaining_cells(profiles: &[SubgamePiece]) -> Vec<u32> {
        let n = profiles.len();
        let mut suffix = vec![0u32; n + 1];
        for i in (0..n).rev() {
            suffix[i] = suffix[i + 1] + profiles[i].cell_count() as u32;
        }
        suffix
    }

    /// Initial row subgame board.
    #[inline(always)]
    pub fn row_board(&self) -> &SubgameBoard {
        &self.row_board
    }

    /// Initial column subgame board.
    #[inline(always)]
    pub fn col_board(&self) -> &SubgameBoard {
        &self.col_board
    }

    /// Row profile for piece at solver index `i`.
    #[inline(always)]
    pub fn row_profile(&self, i: usize) -> &SubgamePiece {
        &self.row_profiles[i]
    }

    /// Column profile for piece at solver index `i`.
    #[inline(always)]
    pub fn col_profile(&self, i: usize) -> &SubgamePiece {
        &self.col_profiles[i]
    }

    /// Shifted row profile for piece `piece_idx` placed at row `row`.
    #[inline(always)]
    pub fn row_shifted_at(&self, piece_idx: usize, row: usize) -> u16x16 {
        self.row_shifted[piece_idx][row]
    }

    /// Shifted col profile for piece `piece_idx` placed at column `col`.
    #[inline(always)]
    pub fn col_shifted_at(&self, piece_idx: usize, col: usize) -> u16x16 {
        self.col_shifted[piece_idx][col]
    }

    /// Number of valid row positions for piece `piece_idx`.
    #[inline(always)]
    pub fn num_row_positions(&self, piece_idx: usize) -> usize {
        self.row_shifted[piece_idx].len()
    }

    /// Number of valid col positions for piece `piece_idx`.
    #[inline(always)]
    pub fn num_col_positions(&self, piece_idx: usize) -> usize {
        self.col_shifted[piece_idx].len()
    }

    /// Number of pieces.
    #[inline(always)]
    pub fn num_pieces(&self) -> usize {
        self.row_profiles.len()
    }

    /// All row profiles (in solver order).
    pub fn row_profiles(&self) -> &[SubgamePiece] {
        &self.row_profiles
    }

    /// All column profiles (in solver order).
    pub fn col_profiles(&self) -> &[SubgamePiece] {
        &self.col_profiles
    }

    /// Check if both row and col subgames are feasible from `from_piece` onward,
    /// given the current subgame boards reconstructed from the main board.
    ///
    /// Returns `(feasible, subgame_nodes_visited)`.
    /// Uses a zero-allocation backtracker with a node budget to bound cost.
    pub fn check_feasible(
        &self,
        row_board: SubgameBoard,
        col_board: SubgameBoard,
        from_piece: usize,
    ) -> (bool, u64) {
        let mut nodes = 0u64;

        // Row subgame feasibility.
        if !self.check_1d_feasible(
            row_board,
            from_piece,
            &self.row_placements,
            &self.row_max_contrib_suffix,
            &self.row_remaining_cells,
            &mut nodes,
        ) {
            return (false, nodes);
        }

        // Col subgame feasibility.
        if !self.check_1d_feasible(
            col_board,
            from_piece,
            &self.col_placements,
            &self.col_max_contrib_suffix,
            &self.col_remaining_cells,
            &mut nodes,
        ) {
            return (false, nodes);
        }

        (true, nodes)
    }

    /// Check if a single 1D subgame is feasible from `from_piece` onward.
    fn check_1d_feasible(
        &self,
        board: SubgameBoard,
        from_piece: usize,
        placements: &[Vec<(usize, u16x16)>],
        max_contrib_suffix: &[u16x16],
        remaining_cells: &[u32],
        nodes: &mut u64,
    ) -> bool {
        // Quick check: total remaining cells must equal total deficit.
        if remaining_cells[from_piece] != board.total_deficit() {
            return false;
        }
        if board.is_solved() {
            return true;
        }
        // Count-sat check at root.
        let shortfall = board.cells().saturating_sub(max_contrib_suffix[from_piece]);
        if shortfall != u16x16::splat(0) {
            return false;
        }

        self.backtrack_1d(board, from_piece, placements, max_contrib_suffix, remaining_cells, nodes)
    }

    /// Zero-allocation recursive backtracker for 1D subgame feasibility.
    /// Max recursion depth = number of pieces (≤36), well within stack limits.
    fn backtrack_1d(
        &self,
        board: SubgameBoard,
        depth: usize,
        placements: &[Vec<(usize, u16x16)>],
        max_contrib_suffix: &[u16x16],
        remaining_cells: &[u32],
        nodes: &mut u64,
    ) -> bool {
        *nodes += 1;

        // Budget exceeded — bail out conservatively (assume feasible).
        if *nodes > FEASIBILITY_NODE_BUDGET {
            return true;
        }

        let n = placements.len();

        // Base case: all pieces placed.
        if depth >= n {
            return board.is_solved();
        }

        // Total deficit check.
        if remaining_cells[depth] != board.total_deficit() {
            return false;
        }

        // Count-sat check: can remaining pieces cover each cell's deficit?
        let shortfall = board.cells().saturating_sub(max_contrib_suffix[depth]);
        if shortfall != u16x16::splat(0) {
            return false;
        }

        // Try each placement for the current piece.
        for &(_pos, shifted) in &placements[depth] {
            let mut new_board = board;
            if new_board.apply_piece(shifted) {
                if self.backtrack_1d(new_board, depth + 1, placements, max_contrib_suffix, remaining_cells, nodes) {
                    return true;
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_basic() {
        // 3x3, M=2, all zeros. Two 1x1 pieces.
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p, p];
        let order = vec![0, 1];

        let data = SubgameData::build(&board, &pieces, &order);

        assert_eq!(data.num_pieces(), 2);
        assert!(data.row_board().is_solved());
        assert!(data.col_board().is_solved());

        // 1x1 piece has row profile [1], fits in 3 row positions
        assert_eq!(data.row_profile(0).len(), 1);
        assert_eq!(data.num_row_positions(0), 3);
        assert_eq!(data.num_col_positions(0), 3);
    }

    #[test]
    fn test_build_nontrivial() {
        // 3x3, M=3 board with deficits
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 0, 1], &[1, 2, 0]];
        let board = Board::from_grid(grid, 3);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let pieces = vec![bar, bar, bar];
        let order = vec![0, 1, 2];

        let data = SubgameData::build(&board, &pieces, &order);

        // Row deficits: [3, 3, 3]
        assert_eq!(data.row_board().get(0), 3);
        assert_eq!(data.row_board().get(1), 3);
        assert_eq!(data.row_board().get(2), 3);

        // 1x3 bar has row profile [3] (1 row, 3 cells)
        assert_eq!(data.row_profile(0).len(), 1);
        assert_eq!(data.row_profile(0).get(0), 3);
        assert_eq!(data.num_row_positions(0), 3);

        // Col profile: [1, 1, 1] (3 cols)
        assert_eq!(data.col_profile(0).len(), 3);
        assert_eq!(data.num_col_positions(0), 1);
    }

    #[test]
    fn test_shifted_profiles() {
        let board = Board::new_solved(4, 4, 2);
        let piece = Piece::from_grid(&[&[true], &[true]]);
        let pieces = vec![piece];
        let order = vec![0];

        let data = SubgameData::build(&board, &pieces, &order);

        assert_eq!(data.num_row_positions(0), 3);

        let s0 = data.row_shifted_at(0, 0);
        let arr0 = s0.to_array();
        assert_eq!(arr0[0], 1);
        assert_eq!(arr0[1], 1);
        assert_eq!(arr0[2], 0);

        let s2 = data.row_shifted_at(0, 2);
        let arr2 = s2.to_array();
        assert_eq!(arr2[0], 0);
        assert_eq!(arr2[1], 0);
        assert_eq!(arr2[2], 1);
        assert_eq!(arr2[3], 1);
    }

    #[test]
    fn test_solver_order_respected() {
        let board = Board::new_solved(4, 4, 2);
        let bar_h = Piece::from_grid(&[&[true, true, true]]);
        let bar_v = Piece::from_grid(&[&[true], &[true], &[true]]);
        let pieces = vec![bar_h, bar_v];

        let order = vec![1, 0];
        let data = SubgameData::build(&board, &pieces, &order);

        assert_eq!(data.row_profile(0).len(), 3);
        assert_eq!(data.col_profile(0).len(), 1);
        assert_eq!(data.row_profile(1).len(), 1);
        assert_eq!(data.col_profile(1).len(), 3);
    }

    #[test]
    fn test_feasible_solved_board() {
        // Solved board, no deficit → always feasible
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let pieces = vec![p, p];
        let order = vec![0, 1];
        let data = SubgameData::build(&board, &pieces, &order);

        // From piece 2 (all placed), boards are solved
        let (ok, _nodes) = data.check_feasible(
            *data.row_board(),
            *data.col_board(),
            2,
        );
        assert!(ok);
    }

    #[test]
    fn test_feasible_solvable_subgame() {
        // 3x3 M=2, all values 1 → deficits all 1
        // Row deficits: [3, 3, 3], Col deficits: [3, 3, 3]
        // Three 1x3 bars can solve both subgames
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let pieces = vec![bar, bar, bar];
        let order = vec![0, 1, 2];
        let data = SubgameData::build(&board, &pieces, &order);

        let (ok, nodes) = data.check_feasible(
            *data.row_board(),
            *data.col_board(),
            0,
        );
        assert!(ok);
        assert!(nodes > 0);
    }

    #[test]
    fn test_infeasible_deficit_mismatch() {
        // 3x3 M=2, all values 1 → total deficit 9
        // Only two 1x3 bars → total cells 6 ≠ 9
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let pieces = vec![bar, bar];
        let order = vec![0, 1];
        let data = SubgameData::build(&board, &pieces, &order);

        let (ok, _nodes) = data.check_feasible(
            *data.row_board(),
            *data.col_board(),
            0,
        );
        assert!(!ok);
    }

    #[test]
    fn test_max_contrib_suffix() {
        let board = Board::new_solved(4, 4, 2);
        // Two pieces: 1x1 (cell_count=1) and 1x2 (cell_count=2)
        let p1 = Piece::from_grid(&[&[true]]);
        let p2 = Piece::from_grid(&[&[true, true]]);
        let pieces = vec![p1, p2];
        let order = vec![0, 1];
        let data = SubgameData::build(&board, &pieces, &order);

        // row_remaining_cells: [3, 2, 0]
        assert_eq!(data.row_remaining_cells[0], 3);
        assert_eq!(data.row_remaining_cells[1], 2);
        assert_eq!(data.row_remaining_cells[2], 0);
    }
}
