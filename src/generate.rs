use rand::{Rng, RngExt};

use crate::core::board::Board;
use crate::game::Game;
use crate::level::LevelSpec;
use crate::core::piece::Piece;

/// The 75 piece shapes used by the real Shapeshifter game, extracted from
/// puzzle_history.jsonl. Each entry is (height, width, flat grid of bools).
pub const SHAPE_CATALOG: [(u8, u8, &[bool]); 75] = [
    // 1 cell
    (1, 1, &[true]),
    // 2 cells
    (1, 2, &[true, true]),
    (2, 1, &[true, true]),
    // 3 cells
    (1, 3, &[true, true, true]),
    (2, 2, &[false, true, true, true]),
    (2, 2, &[true, false, true, true]),
    (2, 2, &[true, true, false, true]),
    (2, 2, &[true, true, true, false]),
    (3, 1, &[true, true, true]),
    // 4 cells
    (2, 2, &[true, true, true, true]),
    (2, 3, &[false, true, false, true, true, true]),
    (2, 3, &[true, true, false, false, true, true]),
    (2, 3, &[true, true, true, false, true, false]),
    (3, 2, &[false, true, true, true, false, true]),
    (3, 2, &[true, false, true, true, false, true]),
    (3, 2, &[true, false, true, true, true, false]),
    (3, 2, &[true, true, true, false, true, false]),
    // 5 cells
    (2, 3, &[false, true, true, true, true, true]),
    (2, 3, &[true, false, true, true, true, true]),
    (2, 3, &[true, true, true, true, false, true]),
    (3, 2, &[true, true, false, true, true, true]),
    (3, 3, &[false, false, true, false, false, true, true, true, true]),
    (3, 3, &[false, false, true, false, true, true, true, true, false]),
    (3, 3, &[false, false, true, true, true, true, true, false, false]),
    (3, 3, &[false, true, false, true, true, true, false, true, false]),
    (3, 3, &[false, true, true, false, true, false, true, true, false]),
    (3, 3, &[true, false, false, true, true, false, false, true, true]),
    (3, 3, &[true, false, false, true, true, true, false, false, true]),
    (3, 3, &[true, false, false, true, true, true, true, false, false]),
    (3, 3, &[true, true, false, false, true, false, false, true, true]),
    (3, 3, &[true, true, false, false, true, true, false, false, true]),
    (3, 3, &[true, true, true, true, false, false, true, false, false]),
    // 6 cells
    (3, 2, &[true, true, true, true, true, true]),
    (3, 3, &[false, false, true, false, true, true, true, true, true]),
    (3, 3, &[false, true, false, true, true, true, true, false, true]),
    (3, 3, &[false, true, true, false, true, false, true, true, true]),
    (3, 3, &[false, true, true, false, true, true, true, true, false]),
    (3, 3, &[true, false, true, true, true, true, false, true, false]),
    (3, 3, &[true, true, false, false, true, true, true, true, false]),
    (3, 3, &[true, true, false, true, true, false, false, true, true]),
    (3, 4, &[true, true, true, true, true, false, false, false, true, false, false, false]),
    (4, 3, &[false, true, false, true, true, false, false, true, true, false, true, false]),
    // 7 cells
    (3, 3, &[true, false, true, true, false, true, true, true, true]),
    (3, 3, &[true, false, true, true, true, true, true, false, true]),
    (3, 3, &[true, true, true, false, true, false, true, true, true]),
    (3, 4, &[false, true, false, false, true, true, false, true, false, true, true, true]),
    (3, 4, &[false, true, true, false, true, true, false, false, false, true, true, true]),
    (3, 4, &[false, true, true, true, false, false, true, false, true, true, true, false]),
    (3, 4, &[true, true, true, false, true, false, true, false, false, false, true, true]),
    (4, 3, &[true, true, false, true, false, false, true, true, true, false, false, true]),
    // 8 cells
    (3, 3, &[true, true, true, true, false, true, true, true, true]),
    (3, 4, &[true, true, true, false, true, false, true, true, true, true, false, false]),
    (4, 3, &[false, true, false, true, true, true, true, false, true, true, false, true]),
    (4, 3, &[false, true, true, true, true, false, false, true, true, false, true, true]),
    (4, 4, &[false, true, false, true, true, true, true, true, false, false, true, false, false, false, true, false]),
    // 9 cells
    (3, 4, &[true, false, false, true, true, false, true, true, true, true, true, true]),
    (3, 4, &[true, true, false, true, true, false, true, true, true, true, true, false]),
    (3, 4, &[true, true, true, false, true, false, false, true, true, true, true, true]),
    (4, 4, &[false, true, true, true, true, true, false, true, false, true, false, false, false, true, true, false]),
    (4, 4, &[true, true, false, false, true, false, false, false, true, false, true, false, true, true, true, true]),
    (4, 4, &[true, true, true, false, false, false, true, true, true, true, true, false, false, true, false, false]),
    (4, 5, &[true, true, false, false, false, false, true, true, true, false, false, false, true, false, false, false, false, true, true, true]),
    // 10 cells
    (4, 4, &[false, true, false, true, true, true, false, true, false, true, true, true, true, true, false, false]),
    (4, 4, &[true, false, false, true, true, false, true, true, true, false, true, false, true, true, true, false]),
    (4, 4, &[true, true, false, true, true, false, false, true, true, true, true, true, false, false, true, false]),
    (4, 5, &[false, true, false, true, false, true, true, true, true, true, false, false, true, false, true, false, false, true, false, false]),
    (5, 4, &[false, true, false, false, false, true, true, true, false, false, true, false, true, true, true, false, true, false, true, false]),
    // 11 cells
    (4, 4, &[true, true, false, true, false, true, true, true, true, true, true, false, true, false, true, false]),
    (4, 4, &[true, true, true, true, true, false, true, true, false, true, true, false, false, false, true, true]),
    (4, 5, &[false, true, false, false, false, false, true, true, true, true, true, true, false, true, false, false, true, false, true, true]),
    (5, 4, &[false, false, false, true, true, false, true, true, true, false, true, false, true, true, true, false, true, false, true, false]),
    // 12 cells
    (5, 4, &[false, true, true, true, true, true, true, true, false, false, true, false, false, true, true, false, false, false, true, true]),
    // 13 cells
    (4, 5, &[true, true, false, true, true, false, true, true, false, true, false, false, true, true, true, true, true, true, false, false]),
    // 14 cells
    (5, 5, &[true, true, false, false, false, false, true, false, false, true, false, true, false, true, true, true, true, true, false, true, false, false, true, true, true]),
    (5, 5, &[true, true, false, false, false, false, true, true, false, true, false, false, true, false, true, true, false, true, true, true, true, true, true, false, false]),
];

/// Build a Piece from a catalog entry.
fn piece_from_catalog(_h: u8, w: u8, flat: &[bool]) -> Piece {
    let grid: Vec<Vec<bool>> = flat.chunks(w as usize).map(|row| row.to_vec()).collect();
    let refs: Vec<&[bool]> = grid.iter().map(|r| r.as_slice()).collect();
    Piece::from_grid(&refs)
}

/// Returns true if the given piece matches one of the 75 known game shapes.
pub fn is_known_shape(piece: &Piece) -> bool {
    SHAPE_CATALOG.iter().any(|&(h, w, flat)| {
        *piece == piece_from_catalog(h, w, flat)
    })
}

/// Pick a random piece from the shape catalog that fits on the given board.
fn random_piece(rng: &mut impl Rng, max_h: u8, max_w: u8) -> Piece {
    // Collect indices of shapes that fit within the board dimensions.
    let candidates: Vec<usize> = SHAPE_CATALOG
        .iter()
        .enumerate()
        .filter(|(_, (h, w, _))| *h <= max_h && *w <= max_w)
        .map(|(i, _)| i)
        .collect();

    let idx = candidates[rng.random_range(0..candidates.len())];
    let (h, w, flat) = SHAPE_CATALOG[idx];
    piece_from_catalog(h, w, flat)
}

/// Generate a random game for the given level spec.
/// Works backwards: starts from a solved board, generates random pieces with
/// random placements, then undoes them to produce the initial board state.
pub fn generate_game(spec: &LevelSpec, rng: &mut impl Rng) -> Game {
    let m = spec.shifts;
    let h = spec.rows;
    let w = spec.columns;
    let n = spec.shapes as usize;

    let mut board = Board::new_solved(h, w, m);
    let mut pieces = Vec::with_capacity(n);
    let mut placements = Vec::with_capacity(n);

    for _ in 0..n {
        let piece = random_piece(rng, h, w);
        let max_row = h - piece.height();
        let max_col = w - piece.width();
        let row = rng.random_range(0..=max_row as usize);
        let col = rng.random_range(0..=max_col as usize);

        // Undo this piece (increment deficit) to build the scrambled board.
        let mask = piece.placed_at(row, col);
        board.undo_piece(mask);

        pieces.push(piece);
        placements.push((row, col));
    }

    Game::new(board, pieces)
}

/// Generate a random game for the given level number.
pub fn generate_for_level(level: u32, rng: &mut impl Rng) -> Option<Game> {
    let spec = crate::level::get_level(level)?;
    Some(generate_game(&spec, rng))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::get_level;

    fn seeded_rng() -> impl Rng {
        <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42)
    }

    #[test]
    fn test_shape_catalog_valid() {
        // Every catalog entry should produce a valid piece.
        for (i, (h, w, flat)) in SHAPE_CATALOG.iter().enumerate() {
            assert_eq!(
                flat.len(),
                *h as usize * *w as usize,
                "catalog[{i}]: grid size mismatch"
            );
            assert!(*h >= 1 && *h <= 5, "catalog[{i}]: bad height {h}");
            assert!(*w >= 1 && *w <= 5, "catalog[{i}]: bad width {w}");
            let grid: Vec<Vec<bool>> = flat.chunks(*w as usize).map(|r| r.to_vec()).collect();
            let refs: Vec<&[bool]> = grid.iter().map(|r| r.as_slice()).collect();
            let piece = Piece::from_grid(&refs);
            assert!(piece.cell_count() >= 1, "catalog[{i}]: empty piece");
        }
    }

    #[test]
    fn test_random_piece_bounds() {
        let mut rng = seeded_rng();
        for _ in 0..100 {
            let piece = random_piece(&mut rng, 5, 5);
            assert!(piece.height() <= 5);
            assert!(piece.width() <= 5);
            assert!(piece.cell_count() >= 1);
        }
    }

    #[test]
    fn test_random_piece_respects_max() {
        let mut rng = seeded_rng();
        for _ in 0..100 {
            let piece = random_piece(&mut rng, 3, 3);
            assert!(piece.height() <= 3);
            assert!(piece.width() <= 3);
        }
    }

    #[test]
    fn test_generate_level_1() {
        let mut rng = seeded_rng();
        let game = generate_for_level(1, &mut rng).unwrap();
        assert_eq!(game.board().height(), 3);
        assert_eq!(game.board().width(), 3);
        assert_eq!(game.board().m(), 2);
        assert_eq!(game.pieces().len(), 2);
    }

    #[test]
    fn test_generate_level_100() {
        let mut rng = seeded_rng();
        let game = generate_for_level(100, &mut rng).unwrap();
        assert_eq!(game.board().height(), 14);
        assert_eq!(game.board().width(), 14);
        assert_eq!(game.board().m(), 5);
        assert_eq!(game.pieces().len(), 36);
    }

    #[test]
    fn test_generate_invalid_level() {
        let mut rng = seeded_rng();
        assert!(generate_for_level(0, &mut rng).is_none());
    }

    #[test]
    fn test_generated_game_has_solution() {
        let mut rng = seeded_rng();
        let spec = get_level(10).unwrap();

        let mut nontrivial = 0;
        for _ in 0..20 {
            let game = generate_game(&spec, &mut rng);
            if !game.board().is_solved() {
                nontrivial += 1;
            }
        }
        assert!(nontrivial > 15, "too many trivially solved games");
    }

    #[test]
    fn test_all_levels_generate() {
        let mut rng = seeded_rng();
        for level in 1..=100 {
            let game = generate_for_level(level, &mut rng);
            assert!(game.is_some(), "failed to generate level {level}");
        }
    }

    #[test]
    fn test_small_board_filters_large_pieces() {
        // A 3x3 board should never get pieces larger than 3x3.
        let mut rng = seeded_rng();
        for _ in 0..200 {
            let piece = random_piece(&mut rng, 3, 3);
            assert!(piece.height() <= 3);
            assert!(piece.width() <= 3);
        }
        // A 2x2 board should only get pieces that fit in 2x2.
        for _ in 0..200 {
            let piece = random_piece(&mut rng, 2, 2);
            assert!(piece.height() <= 2);
            assert!(piece.width() <= 2);
        }
    }
}
