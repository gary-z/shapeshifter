/// Worker binary: solve a single game from JSON on stdin, print result as JSON to stdout.
/// Used by benchmark harnesses to enable process-level timeouts.
use std::io::Read;
use std::time::Instant;

use shapeshifter::puzzle::PuzzleJson;
use shapeshifter::solver;

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("failed to read stdin");
    let puz: PuzzleJson = serde_json::from_str(&input).expect("bad JSON");
    let game = puz.to_game();

    let start = Instant::now();
    let result = solver::solve(&game);
    let elapsed = start.elapsed();

    let solved = result.solution.is_some();
    // Print compact result line: nodes elapsed_ms solved
    println!("{} {} {}", result.nodes_visited, elapsed.as_millis(), solved);
}
