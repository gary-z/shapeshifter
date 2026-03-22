use std::sync::mpsc;
use std::time::{Duration, Instant};

use shapeshifter::puzzle::PuzzleJson;
use shapeshifter::solver;

const TIMEOUT: Duration = Duration::from_secs(60);

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "data/puzzle_history.jsonl".to_string()
    });
    let content = std::fs::read_to_string(&path).expect("failed to read file");
    let puzzles: Vec<PuzzleJson> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("bad JSONL"))
        .collect();

    println!(
        "{:<8} {:<6} {:>12} {:>12} {:<8}",
        "Level", "Pcs", "Nodes", "Time", "Result"
    );
    println!("{}", "-".repeat(54));

    // Solve all puzzles in parallel using one thread per puzzle.
    let mut handles: Vec<(u32, usize, _)> = Vec::new();
    for puz in &puzzles {
        let game = puz.to_game();
        let level = puz.level;
        let n_pieces = puz.pieces.len();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let start = Instant::now();
            let result = solver::solve(&game);
            let elapsed = start.elapsed();
            let _ = tx.send((result, elapsed));
        });

        handles.push((level, n_pieces, rx));
    }

    let mut ok = 0;
    let mut fail = 0;
    let mut timeout = 0;

    for (level, n_pieces, rx) in handles {
        match rx.recv_timeout(TIMEOUT) {
            Ok((result, elapsed)) => {
                let status = if result.solution.is_some() {
                    ok += 1;
                    "OK"
                } else {
                    fail += 1;
                    "FAIL"
                };
                println!(
                    "{:<8} {:<6} {:>12} {:>12.3?} {:<8}",
                    level, n_pieces, result.nodes_visited, elapsed, status
                );
            }
            Err(_) => {
                timeout += 1;
                println!(
                    "{:<8} {:<6} {:>12} {:>12} {:<8}",
                    level, n_pieces, "-", ">60s", "TIMEOUT"
                );
            }
        }
    }

    println!("{}", "-".repeat(54));
    println!("{} ok, {} fail, {} timeout", ok, fail, timeout);
}
