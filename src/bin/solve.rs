use std::io::Read;
use std::path::Path;
use std::time::Instant;

use shapeshifter::generate;
use shapeshifter::puzzle::{self, PuzzleJson};
use shapeshifter::solver;

fn solve_one(
    puz: &PuzzleJson,
    parallel: bool,
    exhaustive: bool,
    worker: bool,
    assets_dir: &str,
    output_path: Option<&str>,
    json_path: Option<&str>,
) -> bool {
    let game = puz.to_game();

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
        return solved;
    }

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
                .unwrap_or_else(|| default_output.to_str().unwrap());

            let html = puzzle::generate_html_guide(puz, &game, &solution, assets_dir);
            std::fs::write(out, &html).expect("failed to write solution HTML");
            println!("Written to {}", out);
            true
        }
        None => {
            eprintln!(
                "No solution found ({:.3?}, {} nodes)",
                elapsed, result.nodes_visited
            );
            false
        }
    }
}

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
                     Reads puzzle JSON from a file argument or stdin.\n\
                     Stdin accepts single JSON or JSONL (one puzzle per line).\n\n\
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

    // File argument: single puzzle.
    if let Some(path) = json_path {
        let puz = PuzzleJson::load(path);
        let ok = solve_one(&puz, parallel, exhaustive, worker, assets_dir, output_path.map(|s| s.as_str()), json_path.map(|s| s.as_str()));
        if !ok && !worker {
            std::process::exit(1);
        }
        return;
    }

    // Stdin: read all input, then try JSONL (line-by-line) or single JSON.
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("failed to read stdin");

    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();

    if lines.len() <= 1 {
        // Single JSON object (may or may not have a trailing newline).
        let puz: PuzzleJson =
            serde_json::from_str(&input).expect("failed to parse puzzle JSON from stdin");
        let ok = solve_one(&puz, parallel, exhaustive, worker, assets_dir, output_path.map(|s| s.as_str()), None);
        if !ok && !worker {
            std::process::exit(1);
        }
    } else {
        // JSONL: one puzzle per line.
        let mut all_ok = true;
        for (idx, line) in lines.iter().enumerate() {
            let puz: PuzzleJson = match serde_json::from_str(line) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Error parsing line {}: {}", idx + 1, e);
                    all_ok = false;
                    continue;
                }
            };
            if !solve_one(&puz, parallel, exhaustive, worker, assets_dir, None, None) {
                all_ok = false;
            }
        }
        if !all_ok && !worker {
            std::process::exit(1);
        }
    }
}
