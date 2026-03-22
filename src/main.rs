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
        eprintln!("Usage: {} <puzzle.json or puzzle_history.jsonl>", args[0]);
        std::process::exit(1);
    }

    let path = &args[1];
    let content = std::fs::read_to_string(path).expect("failed to read file");

    // Detect JSONL (one JSON per line) vs single JSON
    let puzzles: Vec<puzzle::PuzzleJson> = if path.ends_with(".jsonl") {
        content.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("bad JSONL line"))
            .collect()
    } else {
        vec![serde_json::from_str(&content).expect("bad JSON")]
    };

    eprintln!("Loaded {} puzzle(s)", puzzles.len());

    let mut results: Vec<(u32, usize, u64, std::time::Duration, bool)> = Vec::new();

    for puz in &puzzles {
        let game = puz.to_game();
        let start = Instant::now();
        let result = solver::solve(&game);
        let elapsed = start.elapsed();
        let solved = result.solution.is_some();

        results.push((puz.level, puz.pieces.len(), result.nodes_visited, elapsed, solved));
    }

    println!("{:<8} {:<6} {:<12} {:<12} {:<8}",
        "Level", "Pcs", "Nodes", "Time", "Result");
    println!("{}", "-".repeat(50));

    let mut slowest_idx = 0;
    let mut slowest_time = std::time::Duration::ZERO;

    for (i, &(level, pcs, nodes, time, solved)) in results.iter().enumerate() {
        let status = if solved { "OK" } else { "FAIL" };
        println!("{:<8} {:<6} {:<12} {:<12.3?} {:<8}",
            level, pcs, nodes, time, status);
        if time > slowest_time {
            slowest_time = time;
            slowest_idx = i;
        }
    }

    println!();
    let (level, pcs, nodes, time, solved) = results[slowest_idx];
    println!("Slowest: Level {} ({} pcs, {} nodes, {:.3?}, {})",
        level, pcs, nodes, time, if solved { "OK" } else { "FAIL" });
}
