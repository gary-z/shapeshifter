use std::path::Path;
use std::time::Instant;

use shapeshifter::puzzle::{self, PuzzleJson};
use shapeshifter::solver;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut json_path = None;
    let mut assets_dir = "x";
    let mut output_path = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--assets-dir" => {
                i += 1;
                assets_dir = &args[i];
            }
            "-o" | "--output" => {
                i += 1;
                output_path = Some(&args[i]);
            }
            _ => {
                json_path = Some(&args[i]);
            }
        }
        i += 1;
    }

    let json_path = json_path.unwrap_or_else(|| {
        eprintln!("Usage: solve <puzzle.json> [--assets-dir URL] [-o solution.html]");
        std::process::exit(1);
    });

    let puz = PuzzleJson::load(json_path);
    let game = puz.to_game();

    // Default output path: sibling of input named solution.html
    let default_output = Path::new(json_path)
        .parent()
        .unwrap_or(Path::new("."))
        .join("solution.html");
    let output_path = output_path
        .map(|s| s.as_str())
        .unwrap_or_else(|| default_output.to_str().unwrap());

    println!(
        "Level {}: {}x{}, M={}, {} pieces",
        puz.level,
        puz.rows,
        puz.columns,
        puz.m,
        puz.pieces.len()
    );

    let start = Instant::now();
    let result = solver::solve(&game);
    let elapsed = start.elapsed();

    match result.solution {
        Some(solution) => {
            println!("Solved in {:.3?} ({} nodes)", elapsed, result.nodes_visited);
            let html = puzzle::generate_html_guide(&puz, &game, &solution, assets_dir);
            std::fs::write(output_path, &html).expect("failed to write solution HTML");
            println!("Written to {}", output_path);
        }
        None => {
            eprintln!("No solution found ({:.3?}, {} nodes)", elapsed, result.nodes_visited);
            std::process::exit(1);
        }
    }
}
