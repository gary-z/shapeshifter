use std::simd::u16x16;

use super::board::SubgameBoard;
use super::piece::SubgamePiece;

/// A subgame instance: a 1D board, a list of pieces, and a piece pointer.
///
/// Mirrors the full `Game` API but operates on 1D projections.
/// The board tracks unreduced deficit sums; pieces are row/column profiles.
#[derive(Clone)]
pub struct SubgameGame {
    board: SubgameBoard,
    pieces: Vec<SubgamePiece>,
    /// Index of the next piece to place.
    next: usize,
    /// Precomputed placements for each piece: Vec of (position, shifted_profile).
    all_placements: Vec<Vec<(usize, u16x16)>>,
}

impl SubgameGame {
    /// Create a new subgame from a board and pieces.
    ///
    /// Precomputes all valid placements for each piece on the board.
    pub fn new(board: SubgameBoard, pieces: Vec<SubgamePiece>) -> Self {
        assert!(!pieces.is_empty(), "must have at least one piece");
        let board_len = board.len();
        let all_placements: Vec<Vec<(usize, u16x16)>> = pieces
            .iter()
            .map(|p| p.placements(board_len))
            .collect();
        Self {
            board,
            pieces,
            next: 0,
            all_placements,
        }
    }

    pub fn board(&self) -> &SubgameBoard {
        &self.board
    }

    pub fn board_mut(&mut self) -> &mut SubgameBoard {
        &mut self.board
    }

    pub fn pieces(&self) -> &[SubgamePiece] {
        &self.pieces
    }

    /// The index of the next piece to place.
    pub fn next_index(&self) -> usize {
        self.next
    }

    /// Number of pieces remaining.
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

    /// Get precomputed placements for piece at index `i`.
    pub fn placements_for(&self, i: usize) -> &[(usize, u16x16)] {
        &self.all_placements[i]
    }

    /// Total remaining piece cells from piece `from_idx` onward.
    pub fn remaining_cells_from(&self, from_idx: usize) -> u32 {
        self.pieces[from_idx..].iter().map(|p| p.cell_count() as u32).sum()
    }

    /// Place the next piece with the given shifted profile.
    /// Returns `true` if the placement was valid (no underflow).
    /// Advances the piece pointer on success.
    /// On failure, the board is NOT modified (caller need not undo).
    #[inline(always)]
    pub fn place_next(&mut self, shifted_profile: u16x16) -> bool {
        if self.board.apply_piece(shifted_profile) {
            self.next += 1;
            true
        } else {
            false
        }
    }

    /// Undo the last placed piece with the given shifted profile.
    #[inline(always)]
    pub fn undo_last(&mut self, shifted_profile: u16x16) {
        assert!(self.next > 0, "no pieces to undo");
        self.next -= 1;
        self.board.undo_piece(shifted_profile);
    }
}

impl std::fmt::Debug for SubgameGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SubgameGame(next={}/{}, {:?})",
            self.next,
            self.pieces.len(),
            self.board
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_game() -> SubgameGame {
        // Board [3, 3], two pieces with profile [1, 1] each
        // Two placements of [1,1] at pos 0 covers 2+2=4 > deficit 6, so needs 3 of them
        // Actually let's use a simpler setup: board [2, 2], pieces [1,1] and [1,1]
        let board = SubgameBoard::from_cells(&[2, 2]);
        let p = SubgamePiece::from_profile(&[1, 1]);
        SubgameGame::new(board, vec![p, p])
    }

    #[test]
    fn test_initial_state() {
        let game = make_game();
        assert_eq!(game.next_index(), 0);
        assert_eq!(game.remaining(), 2);
        assert!(!game.all_placed());
        assert!(!game.is_solved());
    }

    #[test]
    fn test_place_next_valid() {
        let mut game = make_game();
        let placements = game.placements_for(0).to_vec();
        assert!(!placements.is_empty());

        // Place first piece at position 0
        let ok = game.place_next(placements[0].1);
        assert!(ok);
        assert_eq!(game.next_index(), 1);
        assert_eq!(game.board().get(0), 1);
        assert_eq!(game.board().get(1), 1);
    }

    #[test]
    fn test_place_and_solve() {
        let mut game = make_game();
        let pls = game.placements_for(0).to_vec();

        // Place both pieces at position 0: [2,2] - [1,1] - [1,1] = [0,0]
        assert!(game.place_next(pls[0].1));
        let pls2 = game.placements_for(1).to_vec();
        assert!(game.place_next(pls2[0].1));
        assert!(game.is_solved());
    }

    #[test]
    fn test_undo_last() {
        let mut game = make_game();
        let pls = game.placements_for(0).to_vec();

        assert!(game.place_next(pls[0].1));
        assert_eq!(game.board().get(0), 1);

        game.undo_last(pls[0].1);
        assert_eq!(game.next_index(), 0);
        assert_eq!(game.board().get(0), 2);
        assert_eq!(game.board().get(1), 2);
    }

    #[test]
    fn test_place_underflow_rejected() {
        // Board [1, 0], piece [1, 1] -> cell 1 would go below 0
        let board = SubgameBoard::from_cells(&[1, 0]);
        let p = SubgamePiece::from_profile(&[1, 1]);
        let mut game = SubgameGame::new(board, vec![p]);

        let pls = game.placements_for(0).to_vec();
        let ok = game.place_next(pls[0].1);
        assert!(!ok);
        // Board should be unchanged
        assert_eq!(game.next_index(), 0);
    }

    #[test]
    fn test_remaining_cells() {
        let game = make_game();
        assert_eq!(game.remaining_cells_from(0), 4);
        assert_eq!(game.remaining_cells_from(1), 2);
    }

    #[test]
    #[should_panic(expected = "no pieces to undo")]
    fn test_undo_empty() {
        let mut game = make_game();
        let pls = game.placements_for(0).to_vec();
        game.undo_last(pls[0].1);
    }

    #[test]
    #[should_panic(expected = "must have at least one piece")]
    fn test_no_pieces() {
        let board = SubgameBoard::from_cells(&[1, 2]);
        SubgameGame::new(board, vec![]);
    }
}
