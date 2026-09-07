use std::io::{Read, Write};
use std::time::Instant;

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let puzzle: shapeshifter::puzzle::PuzzleJson = serde_json::from_str(&input).unwrap();
    let game = puzzle.to_game();
    let start = Instant::now();
    let prepared = shapeshifter::solver::prepare(&game, true, false);
    let preparation_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{}",
        serde_json::json!({"ready": true, "preparation_ms": preparation_ms,
        "workers": std::thread::available_parallelism().unwrap().get()})
    );
    std::io::stdout().flush().unwrap();
    let search_start = Instant::now();
    let result = prepared.solve();
    let search_ms = search_start.elapsed().as_secs_f64() * 1000.0;
    if let Some(solution) = &result.solution {
        assert_eq!(solution.len(), game.pieces().len());
        let mut replay = *game.board();
        for (piece, &(row, column)) in game.pieces().iter().zip(solution) {
            assert!(row + piece.height() as usize <= puzzle.rows as usize);
            assert!(column + piece.width() as usize <= puzzle.columns as usize);
            replay.apply_piece(piece.placed_at(row, column));
        }
        assert!(replay.is_solved());
    }
    println!(
        "{}",
        serde_json::json!({"solved": result.solution.is_some(),
        "preparation_ms": preparation_ms, "search_ms": search_ms,
        "solve_ms": start.elapsed().as_secs_f64() * 1000.0,
        "nodes": result.nodes_visited, "placements": result.solution})
    );
}
