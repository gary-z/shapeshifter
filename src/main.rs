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
        eprintln!("Usage: {} <puzzle.json> [--assets-dir <path>]", args[0]);
        std::process::exit(1);
    }

    let path = &args[1];

    // Default assets dir: same name as json but with _files suffix
    let default_assets = path.replace(".json", "_files");
    let assets_dir = args.iter()
        .position(|a| a == "--assets-dir")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or(&default_assets);

    let puz = puzzle::PuzzleJson::load(path);
    let game = puz.to_game();

    eprintln!("Loaded level {}: {}x{}, M={}, {} pieces",
        puz.level, puz.rows, puz.columns, puz.m, puz.pieces.len());

    let start = Instant::now();
    let solution = solver::solve(&game);
    let elapsed = start.elapsed();

    match solution {
        Some(sol) => {
            eprintln!("Solved in {:.3?}", elapsed);
            let html = puzzle::generate_html_guide(&puz, &game, &sol, assets_dir);
            // Output to <input>_solution.html, or data/solution.html if input is data/puzzle.json
            let out_path = if path.ends_with("puzzle.json") {
                path.replace("puzzle.json", "solution.html")
            } else {
                path.replace(".json", "_solution.html")
            };
            std::fs::write(&out_path, &html).expect("failed to write HTML");
            eprintln!("Solution written to {}", out_path);
        }
        None => {
            eprintln!("No solution found ({:.3?})", elapsed);
            std::process::exit(1);
        }
    }
}
