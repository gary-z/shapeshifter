pub mod bitboard;
pub mod board;
pub mod piece;

/// Bitboard row stride. Boards up to 14 columns wide fit in 15×14 = 210 bits
/// within a 256-bit (4×u64) SIMD bitboard. Cell (r, c) maps to bit r*STRIDE+c.
pub const STRIDE: usize = 15;
