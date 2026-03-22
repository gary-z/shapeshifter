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
        eprintln!("Usage: {} <puzzle.json or .jsonl> [--compare]", args[0]);
        std::process::exit(1);
    }

    let path = &args[1];
    let compare = args.iter().any(|a| a == "--compare");
    let content = std::fs::read_to_string(path).expect("failed to read file");

    let puzzles: Vec<puzzle::PuzzleJson> = if path.ends_with(".jsonl") {
        content.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("bad JSONL line"))
            .collect()
    } else {
        vec![serde_json::from_str(&content).expect("bad JSON")]
    };

    if compare {
        println!("{:<8} {:<6} {:<14} {:<14} {:<10}",
            "Level", "Pcs", "Nodes(all)", "Nodes(noLine)", "Ratio");
        println!("{}", "-".repeat(55));

        let mut config_no_lines = solver::PruningConfig::default();
        config_no_lines.min_flips_rowcol = false;
        config_no_lines.min_flips_diagonal = false;

        for puz in &puzzles {
            let game = puz.to_game();

            let r_all = solver::solve_with_config(&game, &solver::PruningConfig::default());
            let r_no = solver::solve_with_config(&game, &config_no_lines);

            let n_all = r_all.nodes_visited;
            let n_no = r_no.nodes_visited;
            let ratio = if n_all > 0 { n_no as f64 / n_all as f64 } else { 1.0 };

            println!("{:<8} {:<6} {:<14} {:<14} {:<10.2}x",
                puz.level, puz.pieces.len(), n_all, n_no, ratio);
        }
    } else {
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
}
