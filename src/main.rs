mod bitboard;
mod board;
mod coverage;
mod game;
mod generate;
mod level;
mod piece;
mod solver;

use rayon::prelude::*;
use std::time::Duration;

fn solve_with_timeout(game: &game::Game, timeout: Duration) -> Option<Duration> {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    let game = game.clone();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let start = Instant::now();
        let _result = solver::solve(&game);
        let elapsed = start.elapsed();
        let _ = tx.send(elapsed);
    });

    match rx.recv_timeout(timeout) {
        Ok(elapsed) => Some(elapsed),
        Err(_) => None,
    }
}

fn main() {
    let timeout = Duration::from_secs(1);
    let num_seeds = 20u64;

    println!(
        "{:<6} {:<4} {:<5} {:<5} {:<6} {:<8} {:<10} {:<10}",
        "Level", "M", "Rows", "Cols", "Pcs", "Solved", "Rate", "AvgTime"
    );
    println!("{}", "-".repeat(62));

    let results: Vec<_> = (1..=100u32)
        .into_par_iter()
        .map(|lvl| {
            let spec = level::get_level(lvl).unwrap();
            let mut solved = 0u64;
            let mut total_time = Duration::ZERO;

            for seed in 0..num_seeds {
                let mut rng =
                    <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = generate::generate_game(&spec, &mut rng);

                match solve_with_timeout(&game, timeout) {
                    Some(elapsed) => {
                        solved += 1;
                        total_time += elapsed;
                    }
                    None => {
                        total_time += timeout;
                    }
                }
            }

            let rate = (solved as f64 / num_seeds as f64) * 100.0;
            let avg = total_time / num_seeds as u32;
            (lvl, spec.shifts, spec.rows, spec.columns, spec.shapes, solved, rate, avg)
        })
        .collect();

    for &(lvl, shifts, rows, cols, shapes, solved, rate, avg) in &results {
        println!(
            "{:<6} {:<4} {:<5} {:<5} {:<6} {:<8} {:<10.1}% {:<10.3?}",
            lvl, shifts, rows, cols, shapes,
            format!("{}/{}", solved, num_seeds),
            rate, avg
        );
    }
}
