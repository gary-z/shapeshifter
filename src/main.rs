mod bitboard;
mod board;
mod game;
mod generate;
mod level;
mod piece;
mod solver;

use std::time::{Duration, Instant};

fn main() {
    let timeout = Duration::from_secs(1);
    let seeds = [42u64, 123, 999, 7777, 31415];

    println!("{:<6} {:<8} {:<6} {:<6} {:<8} {:<10} {:<10}",
        "Level", "M", "Rows", "Cols", "Pieces", "Time", "Result");
    println!("{}", "-".repeat(65));

    for lvl in 1..=100 {
        let spec = level::get_level(lvl).unwrap();
        let mut solved_any = false;
        let mut worst_time = Duration::ZERO;

        for &seed in &seeds {
            let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
            let game = generate::generate_game(&spec, &mut rng);

            let start = Instant::now();
            let result = solver::solve(&game);
            let elapsed = start.elapsed();

            if elapsed > worst_time {
                worst_time = elapsed;
            }

            if result.is_some() {
                solved_any = true;
            }

            if elapsed > timeout {
                break;
            }
        }

        let status = if solved_any { "OK" } else { "FAIL" };
        println!("{:<6} {:<8} {:<6} {:<6} {:<8} {:<10.3?} {:<10}",
            lvl, spec.shifts, spec.rows, spec.columns, spec.shapes, worst_time, status);

        if worst_time > timeout {
            println!("Stopping: level {} exceeded 1s timeout", lvl);
            break;
        }
    }
}
