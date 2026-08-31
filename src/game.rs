use crate::core::board::Board;
use crate::core::piece::Piece;

#[derive(Clone)]
pub struct Game {
    board: Board,
    pieces: Vec<Piece>,
    next: usize,
}

impl Game {
    pub fn new(board: Board, pieces: Vec<Piece>) -> Self {
        assert!(!pieces.is_empty(), "must have at least one piece");
        Self {
            board,
            pieces,
            next: 0,
        }
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    pub fn next_index(&self) -> usize {
        self.next
    }

    pub fn next_piece(&self) -> Option<&Piece> {
        self.pieces.get(self.next)
    }

    pub fn remaining(&self) -> usize {
        self.pieces.len() - self.next
    }

    pub fn all_placed(&self) -> bool {
        self.next >= self.pieces.len()
    }

    pub fn is_solved(&self) -> bool {
        self.all_placed() && self.board.is_solved()
    }

    pub fn place_next(&mut self, row: usize, col: usize) {
        let piece = self.pieces[self.next];
        let mask = piece.placed_at(row, col);
        self.board.apply_piece(mask);
        self.next += 1;
    }

    pub fn undo_last(&mut self, row: usize, col: usize) {
        assert!(self.next > 0, "no pieces to undo");
        self.next -= 1;
        let piece = self.pieces[self.next];
        let mask = piece.placed_at(row, col);
        self.board.undo_piece(mask);
    }
}

impl std::fmt::Debug for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Game(next={}/{}, {:?})",
            self.next,
            self.pieces.len(),
            self.board
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_game() -> Game {
        let board = Board::new_solved(3, 3, 2);
        let p1 = Piece::from_grid(&[&[true]]);
        let p2 = Piece::from_grid(&[&[true]]);
        Game::new(board, vec![p1, p2])
    }

    #[test]
    fn test_initial_state() {
        let game = make_game();
        assert_eq!(game.next_index(), 0);
        assert_eq!(game.remaining(), 2);
        assert!(!game.all_placed());
        assert!(!game.is_solved());
        assert!(game.next_piece().is_some());
    }

    #[test]
    fn test_place_next() {
        let mut game = make_game();
        game.place_next(0, 0);
        assert_eq!(game.next_index(), 1);
        assert_eq!(game.remaining(), 1);
        assert_eq!(game.board().get(0, 0), 1);
    }

    #[test]
    fn test_undo_last() {
        let mut game = make_game();
        game.place_next(0, 0);
        assert_eq!(game.board().get(0, 0), 1);

        game.undo_last(0, 0);
        assert_eq!(game.next_index(), 0);
        assert_eq!(game.remaining(), 2);
        assert_eq!(game.board().get(0, 0), 0);
    }

    #[test]
    fn test_place_all_and_solve() {
        let mut game = make_game();
        game.place_next(0, 0);
        game.place_next(0, 0);
        assert!(game.all_placed());
        assert!(game.is_solved());
    }

    #[test]
    fn test_place_all_unsolved() {
        let mut game = make_game();
        game.place_next(0, 0);
        game.place_next(1, 1);
        assert!(game.all_placed());
        assert!(!game.is_solved());
    }

    #[test]
    fn test_next_piece_exhausted() {
        let mut game = make_game();
        game.place_next(0, 0);
        game.place_next(0, 0);
        assert!(game.next_piece().is_none());
    }

    #[test]
    fn test_multi_cell_piece() {
        let board = Board::new_solved(3, 3, 2);
        let piece = Piece::from_grid(&[&[true, true], &[true, false]]);
        let mut game = Game::new(board, vec![piece, piece]);

        game.place_next(0, 0);
        assert_eq!(game.board().get(0, 0), 1);
        assert_eq!(game.board().get(0, 1), 1);
        assert_eq!(game.board().get(1, 0), 1);
        assert_eq!(game.board().get(1, 1), 0); // not covered

        game.undo_last(0, 0);
        assert_eq!(game.board().get(0, 0), 0);
        assert_eq!(game.board().get(0, 1), 0);
        assert_eq!(game.board().get(1, 0), 0);
    }

    #[test]
    #[should_panic(expected = "no pieces to undo")]
    fn test_undo_empty() {
        let mut game = make_game();
        game.undo_last(0, 0);
    }

    #[test]
    #[should_panic(expected = "must have at least one piece")]
    fn test_no_pieces() {
        let board = Board::new_solved(3, 3, 2);
        Game::new(board, vec![]);
    }
}
