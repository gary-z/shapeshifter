use crate::core::board::Board;
use crate::core::piece::Piece;
use crate::game::Game;
use crate::generate::SHAPE_CATALOG;

use super::board::SubgameBoard;
use super::game::SubgameGame;
use super::piece::SubgamePiece;

/// Compute the row profile of a 2D piece: for each row in the piece's bounding
/// box, count the number of filled cells.
///
/// Returns a `SubgamePiece` with profile length = piece height.
pub fn piece_row_profile(piece: &Piece) -> SubgamePiece {
    let h = piece.height() as usize;
    let w = piece.width() as usize;
    let shape = piece.shape();
    let mut profile = Vec::with_capacity(h);
    for r in 0..h {
        let mut count = 0u16;
        for c in 0..w {
            if shape.get_bit((r * 15 + c) as u32) {
                count += 1;
            }
        }
        profile.push(count);
    }
    SubgamePiece::from_profile(&profile)
}

/// Compute the column profile of a 2D piece: for each column in the piece's
/// bounding box, count the number of filled cells.
///
/// Returns a `SubgamePiece` with profile length = piece width.
pub fn piece_col_profile(piece: &Piece) -> SubgamePiece {
    let h = piece.height() as usize;
    let w = piece.width() as usize;
    let shape = piece.shape();
    let mut profile = Vec::with_capacity(w);
    for c in 0..w {
        let mut count = 0u16;
        for r in 0..h {
            if shape.get_bit((r * 15 + c) as u32) {
                count += 1;
            }
        }
        profile.push(count);
    }
    SubgamePiece::from_profile(&profile)
}

/// Compute the row subgame board: for each row, sum the per-cell deficits.
///
/// `deficit(r, c) = (M - board[r][c]) % M`
///
/// The result is an unreduced sum in `[0, W * (M-1)]`.
pub fn board_row_deficits(board: &Board) -> SubgameBoard {
    let h = board.height() as usize;
    let w = board.width() as usize;
    let m = board.m() as u16;
    let mut cells = Vec::with_capacity(h);
    for r in 0..h {
        let mut sum = 0u16;
        for c in 0..w {
            let v = board.get(r, c) as u16;
            sum += (m - v) % m;
        }
        cells.push(sum);
    }
    SubgameBoard::from_cells(&cells)
}

/// Compute the column subgame board: for each column, sum the per-cell deficits.
pub fn board_col_deficits(board: &Board) -> SubgameBoard {
    let h = board.height() as usize;
    let w = board.width() as usize;
    let m = board.m() as u16;
    let mut cells = Vec::with_capacity(w);
    for c in 0..w {
        let mut sum = 0u16;
        for r in 0..h {
            let v = board.get(r, c) as u16;
            sum += (m - v) % m;
        }
        cells.push(sum);
    }
    SubgameBoard::from_cells(&cells)
}

/// Project a full game into its row subgame.
///
/// - Board: each cell is the sum of per-column deficits for that row.
/// - Pieces: each piece's row profile (cells per row in bounding box).
pub fn to_row_subgame(game: &Game) -> SubgameGame {
    let board = board_row_deficits(game.board());
    let pieces: Vec<SubgamePiece> = game.pieces().iter().map(|p| piece_row_profile(p)).collect();
    SubgameGame::new(board, pieces)
}

/// Project a full game into its column subgame.
pub fn to_col_subgame(game: &Game) -> SubgameGame {
    let board = board_col_deficits(game.board());
    let pieces: Vec<SubgamePiece> = game.pieces().iter().map(|p| piece_col_profile(p)).collect();
    SubgameGame::new(board, pieces)
}

/// Build the full subgame piece catalog: row and column profiles for each of
/// the 75 shapes in SHAPE_CATALOG.
///
/// Returns `(row_profiles, col_profiles)`, both length 75.
pub fn build_subgame_catalog() -> (Vec<SubgamePiece>, Vec<SubgamePiece>) {
    let mut row_profiles = Vec::with_capacity(SHAPE_CATALOG.len());
    let mut col_profiles = Vec::with_capacity(SHAPE_CATALOG.len());

    for &(_h, w, flat) in SHAPE_CATALOG.iter() {
        let grid: Vec<Vec<bool>> = flat.chunks(w as usize).map(|row| row.to_vec()).collect();
        let refs: Vec<&[bool]> = grid.iter().map(|r| r.as_slice()).collect();
        let piece = Piece::from_grid(&refs);
        row_profiles.push(piece_row_profile(&piece));
        col_profiles.push(piece_col_profile(&piece));
    }

    (row_profiles, col_profiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    // --- Piece profile tests ---

    #[test]
    fn test_single_cell_profile() {
        let piece = Piece::from_grid(&[&[true]]);
        let rp = piece_row_profile(&piece);
        let cp = piece_col_profile(&piece);
        assert_eq!(rp.len(), 1);
        assert_eq!(rp.get(0), 1);
        assert_eq!(cp.len(), 1);
        assert_eq!(cp.get(0), 1);
    }

    #[test]
    fn test_horizontal_bar_profile() {
        // ###
        let piece = Piece::from_grid(&[&[true, true, true]]);
        let rp = piece_row_profile(&piece);
        let cp = piece_col_profile(&piece);
        // Row profile: [3] (1 row, 3 cells)
        assert_eq!(rp.len(), 1);
        assert_eq!(rp.get(0), 3);
        // Col profile: [1, 1, 1] (3 cols, 1 cell each)
        assert_eq!(cp.len(), 3);
        assert_eq!(cp.get(0), 1);
        assert_eq!(cp.get(1), 1);
        assert_eq!(cp.get(2), 1);
    }

    #[test]
    fn test_vertical_bar_profile() {
        // #
        // #
        // #
        let piece = Piece::from_grid(&[&[true], &[true], &[true]]);
        let rp = piece_row_profile(&piece);
        let cp = piece_col_profile(&piece);
        assert_eq!(rp.len(), 3);
        assert_eq!(rp.get(0), 1);
        assert_eq!(rp.get(1), 1);
        assert_eq!(rp.get(2), 1);
        assert_eq!(cp.len(), 1);
        assert_eq!(cp.get(0), 3);
    }

    #[test]
    fn test_l_shape_profile() {
        // ##
        // #.
        let piece = Piece::from_grid(&[&[true, true], &[true, false]]);
        let rp = piece_row_profile(&piece);
        let cp = piece_col_profile(&piece);
        // Row: [2, 1]
        assert_eq!(rp.len(), 2);
        assert_eq!(rp.get(0), 2);
        assert_eq!(rp.get(1), 1);
        // Col: [2, 1]
        assert_eq!(cp.len(), 2);
        assert_eq!(cp.get(0), 2);
        assert_eq!(cp.get(1), 1);
    }

    #[test]
    fn test_t_shape_profile() {
        // ###
        // .#.
        let piece = Piece::from_grid(&[&[true, true, true], &[false, true, false]]);
        let rp = piece_row_profile(&piece);
        let cp = piece_col_profile(&piece);
        // Row: [3, 1]
        assert_eq!(rp.len(), 2);
        assert_eq!(rp.get(0), 3);
        assert_eq!(rp.get(1), 1);
        // Col: [1, 2, 1]
        assert_eq!(cp.len(), 3);
        assert_eq!(cp.get(0), 1);
        assert_eq!(cp.get(1), 2);
        assert_eq!(cp.get(2), 1);
    }

    #[test]
    fn test_profile_cell_count_matches_piece() {
        let piece = Piece::from_grid(&[
            &[true, false, true],
            &[true, true, true],
            &[false, true, false],
        ]);
        let rp = piece_row_profile(&piece);
        let cp = piece_col_profile(&piece);
        assert_eq!(rp.cell_count(), piece.cell_count() as u16);
        assert_eq!(cp.cell_count(), piece.cell_count() as u16);
    }

    // --- Board deficit tests ---

    #[test]
    fn test_row_deficits_solved() {
        let board = Board::new_solved(3, 4, 3);
        let rd = board_row_deficits(&board);
        assert_eq!(rd.len(), 3);
        for i in 0..3 {
            assert_eq!(rd.get(i), 0);
        }
        assert_eq!(rd.total_deficit(), 0);
    }

    #[test]
    fn test_col_deficits_solved() {
        let board = Board::new_solved(4, 3, 2);
        let cd = board_col_deficits(&board);
        assert_eq!(cd.len(), 3);
        for i in 0..3 {
            assert_eq!(cd.get(i), 0);
        }
    }

    #[test]
    fn test_row_deficits_nontrivial() {
        // 3x3, M=3: board values
        // 0 1 2
        // 2 1 0
        // 1 0 2
        // Deficits: (3-v)%3
        // 0 2 1  -> sum = 3
        // 1 2 0  -> sum = 3
        // 2 0 1  -> sum = 3
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 1, 0], &[1, 0, 2]];
        let board = Board::from_grid(grid, 3);
        let rd = board_row_deficits(&board);
        assert_eq!(rd.len(), 3);
        assert_eq!(rd.get(0), 3);
        assert_eq!(rd.get(1), 3);
        assert_eq!(rd.get(2), 3);
        assert_eq!(rd.total_deficit(), 9);
    }

    #[test]
    fn test_col_deficits_nontrivial() {
        // Same board as above
        // Deficits:
        // 0 2 1
        // 1 2 0
        // 2 0 1
        // Col sums: 3, 4, 2 -> wait:
        // Col 0: 0+1+2 = 3
        // Col 1: 2+2+0 = 4
        // Col 2: 1+0+1 = 2
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 1, 0], &[1, 0, 2]];
        let board = Board::from_grid(grid, 3);
        let cd = board_col_deficits(&board);
        assert_eq!(cd.len(), 3);
        assert_eq!(cd.get(0), 3);
        assert_eq!(cd.get(1), 4);
        assert_eq!(cd.get(2), 2);
        assert_eq!(cd.total_deficit(), 9);
    }

    #[test]
    fn test_row_and_col_deficits_same_total() {
        // Row and column total deficits must always be equal
        // (both equal the full board's total deficit).
        let grid: &[&[u8]] = &[
            &[0, 1, 2, 0],
            &[1, 0, 1, 2],
            &[2, 2, 0, 1],
        ];
        let board = Board::from_grid(grid, 3);
        let rd = board_row_deficits(&board);
        let cd = board_col_deficits(&board);
        assert_eq!(rd.total_deficit(), cd.total_deficit());
    }

    #[test]
    fn test_m2_deficits() {
        // M=2: deficit = (2-v)%2, so 0->0, 1->1
        let grid: &[&[u8]] = &[&[0, 1, 1], &[1, 0, 0], &[0, 1, 0]];
        let board = Board::from_grid(grid, 2);
        let rd = board_row_deficits(&board);
        // Row 0: 0+1+1 = 2
        // Row 1: 1+0+0 = 1
        // Row 2: 0+1+0 = 1
        assert_eq!(rd.get(0), 2);
        assert_eq!(rd.get(1), 1);
        assert_eq!(rd.get(2), 1);
    }

    #[test]
    fn test_m5_deficits() {
        // M=5: deficit = (5-v)%5
        // v=0 -> 0, v=1 -> 4, v=2 -> 3, v=3 -> 2, v=4 -> 1
        let grid: &[&[u8]] = &[&[0, 1, 2], &[3, 4, 0], &[1, 2, 3]];
        let board = Board::from_grid(grid, 5);
        let rd = board_row_deficits(&board);
        // Row 0: 0+4+3 = 7
        // Row 1: 2+1+0 = 3
        // Row 2: 4+3+2 = 9
        assert_eq!(rd.get(0), 7);
        assert_eq!(rd.get(1), 3);
        assert_eq!(rd.get(2), 9);
    }

    // --- Full projection tests ---

    #[test]
    fn test_to_row_subgame() {
        // 3x3, M=2, all zeros. 2 pieces: 1x1 single cells.
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![p, p]);

        let sg = to_row_subgame(&game);
        assert_eq!(sg.board().len(), 3);
        assert!(sg.board().is_solved());
        assert_eq!(sg.pieces().len(), 2);
        // Each piece has row profile [1]
        assert_eq!(sg.pieces()[0].len(), 1);
        assert_eq!(sg.pieces()[0].get(0), 1);
    }

    #[test]
    fn test_to_col_subgame() {
        let board = Board::new_solved(3, 4, 2);
        let p = Piece::from_grid(&[&[true, true]]);
        let game = Game::new(board, vec![p]);

        let sg = to_col_subgame(&game);
        assert_eq!(sg.board().len(), 4);
        // Piece col profile: [1, 1]
        assert_eq!(sg.pieces()[0].len(), 2);
    }

    #[test]
    fn test_subgame_total_deficit_matches() {
        // The subgame total deficit must match the full game's total deficit.
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 0, 1], &[1, 2, 0]];
        let board = Board::from_grid(grid, 3);
        let p = Piece::from_grid(&[&[true, true, true]]);
        let game = Game::new(board, vec![p, p, p]);

        let row_sg = to_row_subgame(&game);
        let col_sg = to_col_subgame(&game);

        assert_eq!(row_sg.board().total_deficit(), board.total_deficit());
        assert_eq!(col_sg.board().total_deficit(), board.total_deficit());
    }

    // --- Catalog tests ---

    #[test]
    fn test_build_subgame_catalog() {
        let (rows, cols) = build_subgame_catalog();
        assert_eq!(rows.len(), SHAPE_CATALOG.len());
        assert_eq!(cols.len(), SHAPE_CATALOG.len());

        // Every profile should have matching cell counts
        for i in 0..SHAPE_CATALOG.len() {
            assert_eq!(
                rows[i].cell_count(), cols[i].cell_count(),
                "catalog[{i}]: row and col profiles must have same cell count"
            );
        }
    }

    #[test]
    fn test_catalog_single_cell() {
        let (rows, cols) = build_subgame_catalog();
        // First entry is 1x1 single cell
        assert_eq!(rows[0].len(), 1);
        assert_eq!(rows[0].get(0), 1);
        assert_eq!(cols[0].len(), 1);
        assert_eq!(cols[0].get(0), 1);
    }

    #[test]
    fn test_catalog_horizontal_domino() {
        let (rows, cols) = build_subgame_catalog();
        // Second entry: 1x2 horizontal domino
        assert_eq!(rows[1].len(), 1);
        assert_eq!(rows[1].get(0), 2);
        assert_eq!(cols[1].len(), 2);
        assert_eq!(cols[1].get(0), 1);
        assert_eq!(cols[1].get(1), 1);
    }

    #[test]
    fn test_catalog_vertical_domino() {
        let (rows, cols) = build_subgame_catalog();
        // Third entry: 2x1 vertical domino
        assert_eq!(rows[2].len(), 2);
        assert_eq!(rows[2].get(0), 1);
        assert_eq!(rows[2].get(1), 1);
        assert_eq!(cols[2].len(), 1);
        assert_eq!(cols[2].get(0), 2);
    }

    // --- Counterexample from DESIGN.md ---

    #[test]
    fn test_design_counterexample_subgames_solvable() {
        // 3x3, M=3:
        // Board:          Deficits:
        //   0 1 2           0 2 1
        //   2 0 1           1 0 2
        //   1 2 0           2 1 0
        // Three 1x3 horizontal bars.
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 0, 1], &[1, 2, 0]];
        let board = Board::from_grid(grid, 3);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let game = Game::new(board, vec![bar, bar, bar]);

        let row_sg = to_row_subgame(&game);
        let col_sg = to_col_subgame(&game);

        // Row subgame: deficit = [3, 3, 3], each bar has row profile [3].
        assert_eq!(row_sg.board().as_slice(), vec![3, 3, 3]);
        assert_eq!(row_sg.pieces()[0].get(0), 3);
        assert_eq!(row_sg.pieces()[0].len(), 1);

        // Column subgame: deficit = [3, 3, 3], each bar has col profile [1, 1, 1].
        assert_eq!(col_sg.board().as_slice(), vec![3, 3, 3]);
        assert_eq!(col_sg.pieces()[0].len(), 3);
    }
}
