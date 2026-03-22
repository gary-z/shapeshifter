mod bitboard;
mod board;
mod coverage;
mod game;
mod generate;
mod level;
mod piece;
mod puzzle;
mod solver;

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <puzzle.json or .jsonl>", args[0]);
        std::process::exit(1);
    }

    let path = &args[1];
    let content = std::fs::read_to_string(path).expect("failed to read file");
    let puzzles: Vec<puzzle::PuzzleJson> = if path.ends_with(".jsonl") {
        content.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("bad JSONL line"))
            .collect()
    } else {
        vec![serde_json::from_str(&content).expect("bad JSON")]
    };

    println!("{:<8} {:<6} {:<12} {:<12} {:<8}",
        "Level", "Pcs", "Nodes", "Time", "Result");
    println!("{}", "-".repeat(50));

    for puz in &puzzles {
        let game = puz.to_game();
        let start = Instant::now();
        let result = solver::solve(&game);
        let elapsed = start.elapsed();
        let status = if result.solution.is_some() { "OK" } else { "FAIL" };
        println!("{:<8} {:<6} {:<12} {:<12.3?} {:<8}",
            puz.level, puz.pieces.len(), result.nodes_visited, elapsed, status);
    }
}
