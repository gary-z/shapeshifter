//! Pruning techniques, each encapsulated as a struct with:
//! - `precompute()` — build static data from pieces/board (called once)
//! - `try_prune()` — check feasibility, return false to prune (called per node)
//!
//! All structs are concrete (no trait objects) for zero-overhead static dispatch.
//!
//! TODO: After completing the main solver refactor, apply the same pattern to
//! the subgame solver's pruning (total_deficit, count_sat, endgame checks).

pub(crate) mod hit_count;
pub(crate) mod jaggedness;
pub(crate) mod line_family;
pub(crate) mod parity;
pub(crate) mod subgame;
pub(crate) mod subset;
pub(crate) mod total_deficit;
pub(crate) mod weight_tuple;
