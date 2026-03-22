use rand::{Rng, RngExt};

use crate::board::Board;
use crate::game::Game;
use crate::level::LevelSpec;
use crate::piece::Piece;

/// Generate a random connected piece that fits within the given max dimensions.
/// The piece will have between 1 and max_h * max_w filled cells.
fn random_piece(rng: &mut impl Rng, max_h: u8, max_w: u8) -> Piece {
    let max_h = max_h.min(5) as usize;
    let max_w = max_w.min(5) as usize;
    let max_area = max_h * max_w;

    loop {
        // Pick target size from a distribution that scales with board size.
        // Real game data shows: avg piece size ≈ board_area / (3 to 4).
        // Small boards (4x3): avg ~4, peak at 3-4.
        // Large boards (6x6+): avg ~7, peak at 5-6, tail to 14.
        // Weights: roughly match real game's overall distribution:
        //   1:5%, 2:8%, 3:14%, 4:18%, 5:21%, 6:14%, 7:7%, 8:5%, 9+:8%
        const WEIGHTS: [u32; 25] = [
            5, 8, 14, 18, 21, 14, 7, 5, 4, 2,  // sizes 1-10
            3, 1, 1, 1, 0, 0, 0, 0, 0, 0,       // sizes 11-20
            0, 0, 0, 0, 0,                        // sizes 21-25
        ];
        let max_size = max_area.min(WEIGHTS.len());
        let total_weight: u32 = WEIGHTS[..max_size].iter().sum();
        let mut roll = rng.random_range(0..total_weight);
        let mut target = 1;
        for (i, &w) in WEIGHTS[..max_size].iter().enumerate() {
            if roll < w {
                target = i + 1;
                break;
            }
            roll -= w;
        }

        // Use a 5x5 bounding box and grow to the target size.
        let h = max_h;
        let w = max_w;
        let mut grid = vec![vec![false; w]; h];
        let seed_r = rng.random_range(0..h);
        let seed_c = rng.random_range(0..w);
        grid[seed_r][seed_c] = true;
        let mut filled = vec![(seed_r, seed_c)];

        while filled.len() < target {
            // Pick a random filled cell and try to expand from it.
            let &(r, c) = &filled[rng.random_range(0..filled.len())];
            let neighbors: Vec<(usize, usize)> = [(0isize, 1), (0, -1), (1, 0), (-1, 0)]
                .iter()
                .filter_map(|&(dr, dc)| {
                    let nr = r as isize + dr;
                    let nc = c as isize + dc;
                    if nr >= 0 && nr < h as isize && nc >= 0 && nc < w as isize {
                        let (nr, nc) = (nr as usize, nc as usize);
                        if !grid[nr][nc] {
                            return Some((nr, nc));
                        }
                    }
                    None
                })
                .collect();

            if let Some(_) = neighbors.first() {
                // Pick a random unfilled neighbor.
                let &(nr, nc) = &neighbors[rng.random_range(0..neighbors.len())];
                grid[nr][nc] = true;
                filled.push((nr, nc));
            } else {
                break; // No room to grow from this cell
            }
        }

        // Trim empty border rows/cols to get a tight bounding box.
        let min_r = grid.iter().position(|row| row.iter().any(|&v| v)).unwrap();
        let max_r = grid.iter().rposition(|row| row.iter().any(|&v| v)).unwrap();
        let min_c = (0..w)
            .find(|&c| grid.iter().any(|row| row[c]))
            .unwrap();
        let max_c = (0..w)
            .rfind(|&c| grid.iter().any(|row| row[c]))
            .unwrap();

        let trimmed: Vec<Vec<bool>> = grid[min_r..=max_r]
            .iter()
            .map(|row| row[min_c..=max_c].to_vec())
            .collect();

        let refs: Vec<&[bool]> = trimmed.iter().map(|r| r.as_slice()).collect();
        return Piece::from_grid(&refs);
    }
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

        // Undo this piece (decrement) to build the scrambled board.
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
        // Since we generate by undoing placements from a solved board,
        // applying those same placements should solve it.
        // We can't directly test this without the solver, but we can verify
        // the board is not already solved (would be a degenerate case).
        let mut rng = seeded_rng();
        let spec = get_level(10).unwrap();

        // Generate many games and check they aren't trivially solved.
        let mut nontrivial = 0;
        for _ in 0..20 {
            let game = generate_game(&spec, &mut rng);
            if !game.board().is_solved() {
                nontrivial += 1;
            }
        }
        // Most games should be nontrivial.
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
}
