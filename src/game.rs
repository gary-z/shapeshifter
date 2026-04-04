use crate::core::board::Board;
use crate::core::piece::Piece;

/// A game instance: the board state, the list of pieces, and which piece is next.
#[derive(Clone)]
pub struct Game {
    board: Board,
    pieces: Vec<Piece>,
    /// Index of the next piece to place.
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

    /// The index of the next piece to place.
    pub fn next_index(&self) -> usize {
        self.next
    }

    /// The next piece to place, or None if all pieces have been placed.
    pub fn next_piece(&self) -> Option<&Piece> {
        self.pieces.get(self.next)
    }

    /// Number of pieces remaining (not yet placed).
    pub fn remaining(&self) -> usize {
        self.pieces.len() - self.next
    }

    /// True if all pieces have been placed.
    pub fn all_placed(&self) -> bool {
        self.next >= self.pieces.len()
    }

    /// True if all pieces are placed and the board is solved.
    pub fn is_solved(&self) -> bool {
        self.all_placed() && self.board.is_solved()
    }

    /// Place the next piece at the given position, decrementing covered cells' deficits.
    /// Advances the piece pointer. Panics if all pieces have already been placed.
    pub fn place_next(&mut self, row: usize, col: usize) {
        let piece = self.pieces[self.next];
        let mask = piece.placed_at(row, col);
        self.board.apply_piece(mask);
        self.next += 1;
    }

    /// Undo the last placed piece, restoring covered cells' deficits. Rewinds the piece pointer.
    /// Panics if no pieces have been placed.
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
        // 3x3 board, m=2, all zeros
        let board = Board::new_solved(3, 3, 2);
        // Two 1x1 pieces
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
        // 3x3, m=2, all zeros. Place two 1x1 pieces on the same cell -> 0+1+1 = 0 mod 2
        let mut game = make_game();
        game.place_next(0, 0);
        game.place_next(0, 0);
        assert!(game.all_placed());
        assert!(game.is_solved());
    }

    #[test]
    fn test_place_all_unsolved() {
        // Place two pieces on different cells -> those cells become 1, not solved
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
        // L-shaped piece
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
