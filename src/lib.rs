#![feature(portable_simd)]

pub mod core;
pub mod game;
pub mod generate;
pub mod level;
pub mod puzzle;
pub mod solver;

#[cfg(feature = "wasm")]
mod wasm;
