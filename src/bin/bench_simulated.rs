use std::sync::mpsc;
use std::time::{Duration, Instant};

use rand::SeedableRng;
use shapeshifter::generate::generate_for_level;
use shapeshifter::level::get_level;
use shapeshifter::solver;

const TIMEOUT: Duration = Duration::from_secs(1);
const GAMES_PER_LEVEL: u32 = 5;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse optional level range: bench_simulated [start] [end] [games_per_level]
    let start_level = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1u32);
    let end_level = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50u32);
    let games_per = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(GAMES_PER_LEVEL);

    println!(
        "Simulating levels {}-{}, {} games each",
        start_level, end_level, games_per
    );
    println!(
        "{:<8} {:<5} {:<6} {:<6} {:>12} {:>12} {:<8}",
        "Level", "Game", "Pcs", "Board", "Nodes", "Time", "Result"
    );
    println!("{}", "-".repeat(68));

    // Generate all games, then solve them all in parallel.
    let mut tasks: Vec<(u32, u32, String, usize, _)> = Vec::new();

    for level in start_level..=end_level {
        let spec = match get_level(level) {
            Some(s) => s,
            None => continue,
        };
        for g in 0..games_per {
            let seed = level as u64 * 1000 + g as u64;
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
            let game = generate_for_level(level, &mut rng).unwrap();
            let n_pieces = game.pieces().len();
            let board_desc = format!("{}x{}/M{}", spec.rows, spec.columns, spec.shifts);
            let (tx, rx) = mpsc::channel();

            std::thread::spawn(move || {
                let start = Instant::now();
                let result = solver::solve(&game);
                let elapsed = start.elapsed();
                let _ = tx.send((result, elapsed));
            });

            tasks.push((level, g, board_desc, n_pieces, rx));
        }
    }

    let mut ok = 0u32;
    let mut fail = 0u32;
    let mut timeout = 0u32;

    for (level, g, board_desc, n_pieces, rx) in tasks {
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
                    "{:<8} {:<5} {:<6} {:<6} {:>12} {:>12.3?} {:<8}",
                    level, g, n_pieces, board_desc, result.nodes_visited, elapsed, status
                );
            }
            Err(_) => {
                timeout += 1;
                println!(
                    "{:<8} {:<5} {:<6} {:<6} {:>12} {:>12} {:<8}",
                    level, g, n_pieces, board_desc, "-",
                    format!(">{}s", TIMEOUT.as_secs()), "TIMEOUT"
                );
            }
        }
    }

    println!("{}", "-".repeat(68));
    println!("{} ok, {} fail, {} timeout", ok, fail, timeout);
}
