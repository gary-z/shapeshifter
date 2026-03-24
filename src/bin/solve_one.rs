/// Worker binary: solve a single game from JSON on stdin, print result as JSON to stdout.
/// Used by benchmark harnesses to enable process-level timeouts.
use std::io::Read;
use std::time::Instant;

use shapeshifter::puzzle::PuzzleJson;
use shapeshifter::solver;

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("failed to read stdin");
    let puz: PuzzleJson = serde_json::from_str(&input).expect("bad JSON");
    let game = puz.to_game();

    let start = Instant::now();
    // Use serial solver with cancellation.
    // Support DISABLE_PRUNE env var for ablation studies.
    let mut config = solver::PruningConfig::default();
    if let Ok(flag) = std::env::var("DISABLE_PRUNE") {
        match flag.as_str() {
            "active_planes" => config.active_planes = false,
            "min_flips_global" => config.min_flips_global = false,
            "min_flips_rowcol" => config.min_flips_rowcol = false,
            "min_flips_diagonal" => config.min_flips_diagonal = false,
            "coverage" => config.coverage = false,
            "jaggedness" => config.jaggedness = false,
            "cell_locking" => config.cell_locking = false,
            "component_checks" => config.component_checks = false,
            "duplicate_pruning" => config.duplicate_pruning = false,
            "single_cell_endgame" => config.single_cell_endgame = false,
            _ => eprintln!("Unknown prune flag: {}", flag),
        }
    }
    let result = if std::env::var("PARALLEL").is_ok() {
        solver::solve(&game)
    } else {
        solver::solve_with_config(&game, &config)
    };
    let elapsed = start.elapsed();

    let solved = result.solution.is_some();
    // Print compact result line: nodes elapsed_ms solved
    println!("{} {} {}", result.nodes_visited, elapsed.as_millis(), solved);
}
