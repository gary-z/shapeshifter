use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use shapeshifter::generate;
use shapeshifter::puzzle::{PuzzleJson, generate_html_guide};
use shapeshifter::solver;

fn solve_one(
    puzzle: &PuzzleJson,
    parallel: bool,
    exhaustive: bool,
    worker: bool,
    assets_dir: &str,
    output_path: Option<&str>,
    json_path: Option<&str>,
) -> bool {
    let game = puzzle.to_game();

    for (piece_index, piece) in game.pieces().iter().enumerate() {
        if !generate::is_known_shape(piece) {
            eprintln!(
                "Warning: piece {} is not a known Shapeshifter shape ({}x{}, {} cells)",
                piece_index,
                piece.height(),
                piece.width(),
                piece.cell_count(),
            );
        }
    }

    let start = Instant::now();
    let prepared = solver::prepare(&game, parallel, exhaustive);
    let preparation = start.elapsed();
    if worker {
        println!("READY {}", preparation.as_millis());
        std::io::stdout()
            .flush()
            .expect("failed to flush worker readiness");
    }
    let search_start = Instant::now();
    let result = prepared.solve();
    let search_elapsed = search_start.elapsed();
    let elapsed = start.elapsed();

    if worker {
        let solved = result.solution.is_some();
        println!(
            "{} {} {}",
            result.nodes_visited,
            search_elapsed.as_millis(),
            solved
        );
        return solved;
    }

    println!(
        "Level {}: {}x{}, M={}, {} pieces",
        puzzle.level,
        puzzle.rows,
        puzzle.columns,
        puzzle.m,
        puzzle.pieces.len()
    );

    match result.solution {
        Some(solution) => {
            println!(
                "Solved in {:.3?} ({:.3?} preparation, {:.3?} search, {} nodes)",
                elapsed, preparation, search_elapsed, result.nodes_visited
            );

            let default_output = json_path
                .map(|p| {
                    Path::new(p)
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join("solution.html")
                })
                .unwrap_or_else(|| Path::new("solution.html").to_path_buf());
            let output = output_path.unwrap_or_else(|| default_output.to_str().unwrap());

            let html = generate_html_guide(puzzle, &solution, assets_dir);
            std::fs::write(output, &html).expect("failed to write solution HTML");
            println!("Written to {}", output);
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
                       --exhaustive      Continue through the bounded search tree\n  \
                       --worker          Benchmark protocol: READY preparation_ms, then nodes search_ms solved\n  \
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

    if let Some(path) = json_path {
        let puzzle = PuzzleJson::load(path);
        let solved = solve_one(
            &puzzle,
            parallel,
            exhaustive,
            worker,
            assets_dir,
            output_path.map(String::as_str),
            json_path.map(String::as_str),
        );
        if !solved && !worker {
            std::process::exit(1);
        }
        return;
    }

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("failed to read stdin");

    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();

    if lines.len() <= 1 {
        let puzzle: PuzzleJson =
            serde_json::from_str(&input).expect("failed to parse puzzle JSON from stdin");
        let solved = solve_one(
            &puzzle,
            parallel,
            exhaustive,
            worker,
            assets_dir,
            output_path.map(String::as_str),
            None,
        );
        if !solved && !worker {
            std::process::exit(1);
        }
    } else {
        let mut all_ok = true;
        for (line_index, line) in lines.iter().enumerate() {
            let puzzle: PuzzleJson = match serde_json::from_str(line) {
                Ok(puzzle) => puzzle,
                Err(error) => {
                    eprintln!("Error parsing line {}: {}", line_index + 1, error);
                    all_ok = false;
                    continue;
                }
            };
            if !solve_one(
                &puzzle, parallel, exhaustive, worker, assets_dir, None, None,
            ) {
                all_ok = false;
            }
        }
        if !all_ok && !worker {
            std::process::exit(1);
        }
    }
}
