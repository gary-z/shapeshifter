use wasm_bindgen::prelude::*;

use crate::puzzle::PuzzleJson;
use crate::solver;

#[wasm_bindgen]
pub fn solve_puzzle(json: &str) -> String {
    let puzzle: PuzzleJson = match serde_json::from_str(json) {
        Ok(p) => p,
        Err(e) => {
            return format!(r#"{{"error": "Failed to parse JSON: {}"}}"#, e);
        }
    };

    let game = puzzle.to_game();
    let result = solver::solve(&game, false, false);

    match result.solution {
        Some(placements) => {
            let placements_json: Vec<String> = placements
                .iter()
                .map(|(r, c)| format!("[{},{}]", r, c))
                .collect();
            format!(
                r#"{{"solved": true, "placements": [{}], "nodes": {}}}"#,
                placements_json.join(","),
                result.nodes_visited
            )
        }
        None => {
            format!(
                r#"{{"solved": false, "nodes": {}}}"#,
                result.nodes_visited
            )
        }
    }
}
