use crate::core::board::Board;
use crate::core::piece::Piece;
use crate::game::Game;
use serde::Deserialize;

#[derive(Deserialize, serde::Serialize)]
pub struct PuzzleJson {
    pub level: u32,
    pub m: u8,
    pub rows: u8,
    pub columns: u8,
    pub board: Vec<Vec<u8>>,
    pub pieces: Vec<Vec<Vec<bool>>>,
    #[serde(default)]
    pub icons: Vec<String>,
}

impl PuzzleJson {
    pub fn to_game(&self) -> Game {
        let grid: Vec<&[u8]> = self.board.iter().map(|r| r.as_slice()).collect();
        let board = Board::from_grid(&grid, self.m);

        let pieces: Vec<Piece> = self
            .pieces
            .iter()
            .map(|shape| {
                let rows: Vec<&[bool]> = shape.iter().map(|r| r.as_slice()).collect();
                Piece::from_grid(&rows)
            })
            .collect();

        Game::new(board, pieces)
    }
}
