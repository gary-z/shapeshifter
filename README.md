# Shapeshifter
This project is a solver for Shapeshifter, a Neopets puzzle game.

## Game rules
- There is an H by W board (3 <= H, W <= 14) of cells filled with numbers in [0, M).
- There are 2 <= N <= 36 pieces that must be placed on the board.
- Each piece fits within a 5x5 cell region and its dimensions do not exceed H or W.
- Pieces are continuous along cardinal directions.
- Pieces cannot be rotated or flipped; they must be placed in their given orientation.
- Each cell's deficit is `(M - value) % M`: the number of hits still needed to reach 0. When a piece is placed, each filled cell's deficit is decremented by 1 (equivalently, the cell value increments by 1 mod M).
- Multiple pieces may overlap the same board cell.
- Each piece must be placed exactly once (though the piece list may contain duplicates).
- Placement order does not matter; only the final positions matter.
- The goal is to place all the pieces such that the entire board is filled with 0s.

## Levels
Level specifications (shifts/M, rows, columns, number of shapes) are defined in `data/levels.json`.