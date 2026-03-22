use std::sync::mpsc;
use std::time::{Duration, Instant};

use shapeshifter::puzzle::PuzzleJson;
use shapeshifter::solver;

const TIMEOUT: Duration = Duration::from_secs(120);

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

    let n = puzzles.len();
    let levels: Vec<u32> = puzzles.iter().map(|p| p.level).collect();
    let piece_counts: Vec<usize> = puzzles.iter().map(|p| p.pieces.len()).collect();

    let (tx, rx) = mpsc::channel();
    for (idx, puz) in puzzles.iter().enumerate() {
        let game = puz.to_game();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = solver::solve(&game);
            let elapsed = start.elapsed();
            let _ = tx.send((idx, result, elapsed));
        });
    }
    drop(tx);

    // Collect results concurrently with a global deadline.
    let mut results: Vec<Option<(solver::SolveResult, Duration)>> =
        (0..n).map(|_| None).collect();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let remaining_time = deadline.saturating_duration_since(Instant::now());
        if remaining_time.is_zero() { break; }
        match rx.recv_timeout(remaining_time) {
            Ok((idx, result, elapsed)) => {
                results[idx] = Some((result, elapsed));
                if results.iter().all(|r| r.is_some()) { break; }
            }
            Err(_) => break,
        }
    }

    let mut ok = 0;
    let mut fail = 0;
    let mut timeout = 0;

    for (i, r) in results.iter().enumerate() {
        match r {
            Some((result, elapsed)) => {
                let status = if result.solution.is_some() {
                    ok += 1;
                    "OK"
                } else {
                    fail += 1;
                    "FAIL"
                };
                println!(
                    "{:<8} {:<6} {:>12} {:>12.3?} {:<8}",
                    levels[i], piece_counts[i], result.nodes_visited, elapsed, status
                );
            }
            None => {
                timeout += 1;
                println!(
                    "{:<8} {:<6} {:>12} {:>12} {:<8}",
                    levels[i], piece_counts[i], "-",
                    format!(">{}s", TIMEOUT.as_secs()), "TIMEOUT"
                );
            }
        }
    }

    println!("{}", "-".repeat(54));
    println!("{} ok, {} fail, {} timeout", ok, fail, timeout);
}
