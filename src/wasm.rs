use wasm_bindgen::prelude::*;

use crate::puzzle::PuzzleJson;
use crate::solver::{self, runtime};

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[allow(unused_imports)] // Re-exported to JavaScript by wasm-bindgen.
pub use wasm_bindgen_rayon::init_thread_pool;

/// Preparation is separate so it does not consume the browser's search budget.
#[wasm_bindgen]
pub struct BrowserSearch {
    prepared: solver::PreparedSearch,
}

#[wasm_bindgen]
impl BrowserSearch {
    #[wasm_bindgen(constructor)]
    pub fn new(json: &str) -> Result<BrowserSearch, JsValue> {
        let puzzle = parse_puzzle(json).map_err(|message| JsValue::from_str(&message))?;
        Ok(Self {
            prepared: solver::prepare(&puzzle.to_game(), true, false),
        })
    }

    /// Branching, inference and frontier construction all count toward the cap.
    pub fn solve(&self, budget_ms: u32) -> String {
        let budget = runtime::Duration::from_millis(u64::from(budget_ms.min(120_000)));
        let start = runtime::Instant::now();
        let result = self.prepared.solve_with_budget(budget);
        let elapsed = start.elapsed();
        serde_json::json!({
            "solved": result.solution.is_some(),
            "placements": result.solution,
            "nodes": result.nodes_visited,
            "search_ms": elapsed.as_secs_f64() * 1000.0,
            "cancelled": runtime::cancelled(),
            "timed_out": result.solution.is_none() && elapsed >= budget,
        })
        .to_string()
    }
}

#[wasm_bindgen]
pub fn solve_puzzle(json: &str) -> String {
    match BrowserSearch::new(json) {
        Ok(search) => search.solve(120_000),
        Err(error) => {
            serde_json::json!({ "error": error.as_string().unwrap_or_default() }).to_string()
        }
    }
}

#[wasm_bindgen]
pub fn worker_count() -> usize {
    runtime::workers()
}

/// The UI writes this shared u32 atomically while Rust occupies the workers.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn cancellation_ptr() -> *const std::sync::atomic::AtomicU32 {
    &runtime::CANCELLED
}

fn parse_puzzle(json: &str) -> Result<PuzzleJson, String> {
    let puzzle: PuzzleJson = serde_json::from_str(json).map_err(|error| error.to_string())?;
    if !(2..=5).contains(&puzzle.m)
        || !(3..=14).contains(&puzzle.rows)
        || !(3..=14).contains(&puzzle.columns)
        || puzzle.board.len() != usize::from(puzzle.rows)
        || puzzle.board.iter().any(|row| {
            row.len() != usize::from(puzzle.columns) || row.iter().any(|&value| value >= puzzle.m)
        })
        || !(1..=36).contains(&puzzle.pieces.len())
        || puzzle.pieces.iter().any(|piece| {
            piece.is_empty()
                || piece.len() > 5
                || piece[0].is_empty()
                || piece[0].len() > 5
                || piece.iter().any(|row| row.len() != piece[0].len())
                || !piece.iter().flatten().any(|&cell| cell)
        })
    {
        return Err("Invalid board or piece dimensions or cell values".to_owned());
    }
    Ok(puzzle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_puzzles_before_entering_wasm_search() {
        let valid = r#"{"level":1,"m":3,"rows":3,"columns":3,"board":[[0,0,0],[0,0,0],[0,0,0]],"pieces":[[[true]]]}"#;
        assert!(parse_puzzle(valid).is_ok());
        assert!(parse_puzzle(&valid.replace("\"m\":3", "\"m\":0")).is_err());
        assert!(parse_puzzle(&valid.replace("[0,0,0]", "[0,0]")).is_err());
        assert!(parse_puzzle(&valid.replace("true", "false")).is_err());
        assert!(parse_puzzle(&valid.replace("[[[true]]]", "[]")).is_err());
    }
}
