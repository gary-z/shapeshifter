use std::io::Read;
use std::path::Path;
use std::time::Instant;

use shapeshifter::generate;
use shapeshifter::puzzle::{self, PuzzleJson};
use shapeshifter::solver;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut json_path = None;
    let mut assets_dir = "x";
    let mut output_path = None;
    let mut parallel = false;
    let mut exhaustive = false;
    let mut worker = false;

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
            "--parallel" => parallel = true,
            "--exhaustive" => exhaustive = true,
            "--worker" => worker = true,
            "-h" | "--help" => {
                eprintln!(
                    "Usage: solve [puzzle.json] [OPTIONS]\n\n\
                     Reads puzzle JSON from a file argument or stdin.\n\n\
                     Options:\n  \
                       --parallel        Use parallel solver (all cores)\n  \
                       --exhaustive      Explore full search tree\n  \
                       --worker          Compact output for benchmarks (nodes elapsed_ms solved)\n  \
                       --assets-dir URL  Base URL for piece images in HTML output\n  \
                       -o, --output PATH Write solution HTML to PATH\n  \
                       -h, --help        Show this help"
                );
                std::process::exit(0);
            }
            _ => {
                json_path = Some(&args[i]);
            }
        }
        i += 1;
    }

    // Load puzzle from file or stdin.
    let puz: PuzzleJson = if let Some(path) = json_path {
        PuzzleJson::load(path)
    } else {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .expect("failed to read stdin");
        serde_json::from_str(&input).expect("failed to parse puzzle JSON from stdin")
    };
    let game = puz.to_game();

    // Validate that all pieces are known game shapes.
    for (i, piece) in game.pieces().iter().enumerate() {
        if !generate::is_known_shape(piece) {
            eprintln!(
                "Warning: piece {} is not a known Shapeshifter shape ({}x{}, {} cells)",
                i,
                piece.height(),
                piece.width(),
                piece.cell_count(),
            );
        }
    }

    let start = Instant::now();
    let result = solver::solve(&game, parallel, exhaustive);
    let elapsed = start.elapsed();

    if worker {
        let solved = result.solution.is_some();
        println!("{} {} {}", result.nodes_visited, elapsed.as_millis(), solved);
        return;
    }

    // Interactive output with optional HTML guide.
    println!(
        "Level {}: {}x{}, M={}, {} pieces",
        puz.level, puz.rows, puz.columns, puz.m,
        puz.pieces.len()
    );

    match result.solution {
        Some(solution) => {
            println!("Solved in {:.3?} ({} nodes)", elapsed, result.nodes_visited);

            let default_output = json_path
                .map(|p| {
                    Path::new(p)
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join("solution.html")
                })
                .unwrap_or_else(|| Path::new("solution.html").to_path_buf());
            let out = output_path
                .map(|s| s.as_str())
                .unwrap_or_else(|| default_output.to_str().unwrap());

            let html = puzzle::generate_html_guide(&puz, &game, &solution, assets_dir);
            std::fs::write(out, &html).expect("failed to write solution HTML");
            println!("Written to {}", out);
        }
        None => {
            eprintln!(
                "No solution found ({:.3?}, {} nodes)",
                elapsed, result.nodes_visited
            );
            std::process::exit(1);
        }
    }
}
