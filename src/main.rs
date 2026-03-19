mod bitboard;
mod board;
mod coverage;
mod game;
mod generate;
mod level;
mod piece;
mod solver;

use rayon::prelude::*;
use std::time::{Duration, Instant};

/// Run solver with a hard timeout. Returns Some(elapsed) if solved, None if timed out.
fn solve_with_timeout(game: &game::Game, timeout: Duration) -> Option<Duration> {
    use std::sync::mpsc;
    use std::thread;

    let game = game.clone();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let start = Instant::now();
        let result = solver::solve(&game);
        let elapsed = start.elapsed();
        let _ = tx.send((result, elapsed));
    });

    match rx.recv_timeout(timeout) {
        Ok((Some(_), elapsed)) => Some(elapsed),
        Ok((None, elapsed)) => {
            // Solver returned None (no solution) within timeout.
            Some(elapsed)
        }
        Err(_) => None, // Timed out — thread is abandoned.
    }
}

fn main() {
    let timeout = Duration::from_secs(1);
    let seeds = [42u64, 123, 999, 7777, 31415];

    println!(
        "{:<6} {:<8} {:<6} {:<6} {:<8} {:<12} {:<10}",
        "Level", "M", "Rows", "Cols", "Pieces", "Time", "Result"
    );
    println!("{}", "-".repeat(67));

    let results: Vec<_> = (1..=100u32)
        .into_par_iter()
        .map(|lvl| {
            let spec = level::get_level(lvl).unwrap();
            let mut worst_time = Duration::ZERO;
            let mut all_solved = true;
            let mut any_timeout = false;

            for &seed in &seeds {
                let mut rng =
                    <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = generate::generate_game(&spec, &mut rng);

                match solve_with_timeout(&game, timeout) {
                    Some(elapsed) => {
                        if elapsed > worst_time {
                            worst_time = elapsed;
                        }
                    }
                    None => {
                        worst_time = timeout;
                        all_solved = false;
                        any_timeout = true;
                        break;
                    }
                }
            }

            let status = if any_timeout {
                "TIMEOUT"
            } else if all_solved {
                "OK"
            } else {
                "FAIL"
            };
            (lvl, spec.shifts, spec.rows, spec.columns, spec.shapes, worst_time, status)
        })
        .collect();

    for (lvl, shifts, rows, cols, shapes, time, status) in &results {
        println!(
            "{:<6} {:<8} {:<6} {:<6} {:<8} {:<12.3?} {:<10}",
            lvl, shifts, rows, cols, shapes, time, status
        );
    }
}
