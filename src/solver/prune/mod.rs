//! Pruning techniques, each encapsulated as a struct with:
//! - `precompute()` — build static data from pieces/board (called once)
//! - `try_prune()` — check feasibility, return false to prune (called per node)
//!
//! All structs are concrete (no trait objects) for zero-overhead static dispatch.

pub(crate) mod hit_count;
pub(crate) mod jaggedness;
pub(crate) mod parity;
pub(crate) mod total_deficit;
