use std::time::{Duration, Instant};

use rand::SeedableRng;
use rayon::prelude::*;
use shapeshifter::generate::generate_game;
use shapeshifter::level::LevelSpec;
use shapeshifter::subgame::game::SubgameGame;
use shapeshifter::subgame::generate::{
    board_col_deficits, board_row_deficits, piece_col_profile, piece_row_profile,
};
use shapeshifter::subgame::piece::SubgamePiece;
use shapeshifter::subgame::solver::{SubgameSolver, SubgamePruningConfig, SubgameSolveResult, SolverStats};

fn print_usage() {
    eprintln!(
        "Usage: bench_subgame [OPTIONS]\n\n\
         Benchmark the subgame solver on projected subgames from full 2D games.\n\n\
         Options:\n  \
           --start LEVEL    Start level (default: 1)\n  \
           --end LEVEL      End level (default: 20)\n  \
           --games-per N    Games per level (default: 10)\n  \
           --timeout SECS   Timeout per solve in seconds (default: no limit)\n  \
           --seed SEED      Base random seed (default: 0)\n  \
           -h, --help       Show this help"
    );
}

use rand::RngExt;

/// Generate a full 2D game, place `skip` pieces randomly to create a
/// mid-game state, then project the remaining pieces into subgames.
fn generate_subgames(
    spec: &LevelSpec,
    seed: u64,
    skip: usize,
) -> Option<(SubgameGame, SubgameGame)> {
    let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
    let game = generate_game(spec, &mut rng);
    let h = spec.rows;
    let w = spec.columns;

    let pieces = game.pieces();
    let n = pieces.len();
    if skip >= n { return None; }

    let mut indexed: Vec<(usize, usize)> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.placements(h, w).len()))
        .collect();
    indexed.sort_by(|(i, a_pl), (j, b_pl)| {
        a_pl.cmp(b_pl)
            .then_with(|| pieces[*j].perimeter().cmp(&pieces[*i].perimeter()))
            .then_with(|| pieces[*j].cell_count().cmp(&pieces[*i].cell_count()))
            .then_with(|| pieces[*i].shape().limbs().cmp(&pieces[*j].shape().limbs()))
    });
    let order: Vec<usize> = indexed.iter().map(|(i, _)| *i).collect();

    let mut board = game.board().clone();
    for k in 0..skip {
        let orig_idx = order[k];
        let p = &pieces[orig_idx];
        let pls = p.placements(h, w);
        if pls.is_empty() { return None; }
        let j = rng.random_range(0..pls.len());
        let (_, _, mask) = pls[j];
        board.apply_piece(mask);
    }

    let remaining: Vec<_> = order[skip..].iter().map(|&i| &pieces[i]).collect();

    let row_profiles: Vec<SubgamePiece> =
        remaining.iter().map(|p| piece_row_profile(p)).collect();
    let col_profiles: Vec<SubgamePiece> =
        remaining.iter().map(|p| piece_col_profile(p)).collect();

    let row_board = board_row_deficits(&board);
    let col_board = board_col_deficits(&board);

    if row_profiles.is_empty() { return None; }

    let total_cells: u32 = remaining.iter().map(|p| p.cell_count()).sum();
    if total_cells != row_board.total_deficit() {
        return None;
    }

    let row_sg = SubgameGame::new(row_board, row_profiles);
    let col_sg = SubgameGame::new(col_board, col_profiles);
    Some((row_sg, col_sg))
}

struct BenchTask {
    level: u32,
    game_idx: u32,
    board_desc: String,
    label: &'static str,
    subgame: SubgameGame,
}

struct BenchResult {
    level: u32,
    game_idx: u32,
    board_desc: String,
    label: &'static str,
    base_nodes: u64,
    opt_nodes: u64,
    elapsed_secs: f64,
    timed_out: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut start_level: u32 = 1;
    let mut end_level: u32 = 20;
    let mut games_per: u32 = 10;
    let mut base_seed: u64 = 0;
    let mut timeout_secs: Option<f64> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--start" => {
                i += 1;
                start_level = args[i].parse().expect("invalid start level");
            }
            "--end" => {
                i += 1;
                end_level = args[i].parse().expect("invalid end level");
            }
            "--games-per" => {
                i += 1;
                games_per = args[i].parse().expect("invalid games-per");
            }
            "--timeout" => {
                i += 1;
                timeout_secs = Some(args[i].parse().expect("invalid timeout"));
            }
            "--seed" => {
                i += 1;
                base_seed = args[i].parse().expect("invalid seed");
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown option: {}", other);
                print_usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let timeout = timeout_secs.map(|s| Duration::from_secs_f64(s));

    // Build all tasks first.
    let levels = shapeshifter::level::load_levels();
    let mut tasks: Vec<BenchTask> = Vec::new();

    for spec in &levels {
        if spec.level < start_level || spec.level > end_level {
            continue;
        }
        let board_desc = format!("{}x{}/M{}", spec.rows, spec.columns, spec.shifts);

        for g in 0..games_per {
            let mut found = None;
            'search: for skip in 1..spec.shapes as usize {
                for trial in 0..20u64 {
                    let seed = base_seed + spec.level as u64 * 10000 + g as u64 * 100 + trial;
                    if let Some(sg) = generate_subgames(spec, seed, skip) {
                        found = Some(sg);
                        break 'search;
                    }
                }
            }
            let (row_sg, col_sg) = match found {
                Some(sg) => sg,
                None => continue,
            };

            tasks.push(BenchTask {
                level: spec.level, game_idx: g, board_desc: board_desc.clone(),
                label: "row", subgame: row_sg,
            });
            tasks.push(BenchTask {
                level: spec.level, game_idx: g, board_desc: board_desc.clone(),
                label: "col", subgame: col_sg,
            });
        }
    }

    let n_tasks = tasks.len();
    let n_workers = rayon::current_num_threads();
    let timeout_str = timeout_secs.map_or("none".to_string(), |s| format!("{}s", s));
    eprintln!(
        "Benchmarking {} tasks across {} workers (timeout: {})",
        n_tasks, n_workers, timeout_str,
    );

    let baseline_config = SubgamePruningConfig::none().only(|c| {
        c.total_deficit = true;
    });

    // Run all tasks in parallel.
    let mut results: Vec<BenchResult> = tasks
        .into_par_iter()
        .map(|task| {
            // Each solve gets its own deadline so it actually aborts.
            let base_deadline = timeout.map(|t| Instant::now() + t);
            let mut solver = SubgameSolver::with_config(task.subgame.clone(), baseline_config);
            if let Some(dl) = base_deadline { solver = solver.with_deadline(dl); }
            let start = Instant::now();
            let (base_result, base_stats) = solver.solve();
            let base_elapsed = start.elapsed();
            let base_to = matches!(base_result, SubgameSolveResult::Timeout);

            let opt_deadline = timeout.map(|t| Instant::now() + t);
            let mut solver = SubgameSolver::new(task.subgame);
            if let Some(dl) = opt_deadline { solver = solver.with_deadline(dl); }
            let start2 = Instant::now();
            let (opt_result, opt_stats) = solver.solve();
            let opt_elapsed = start2.elapsed();
            let opt_to = matches!(opt_result, SubgameSolveResult::Timeout);

            BenchResult {
                level: task.level,
                game_idx: task.game_idx,
                board_desc: task.board_desc,
                label: task.label,
                base_nodes: base_stats.nodes_visited,
                opt_nodes: opt_stats.nodes_visited,
                elapsed_secs: (base_elapsed + opt_elapsed).as_secs_f64(),
                timed_out: base_to || opt_to,
            }
        })
        .collect();

    // Sort by level, game, label for consistent output.
    results.sort_by(|a, b| {
        a.level.cmp(&b.level)
            .then(a.game_idx.cmp(&b.game_idx))
            .then(a.label.cmp(&b.label))
    });

    // Print results.
    println!(
        "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>8} {:>8}",
        "Level", "Game", "Board", "Type", "Base", "Opt", "Speedup", "Time"
    );
    println!("{}", "-".repeat(68));

    let mut total_base_nodes: u64 = 0;
    let mut total_opt_nodes: u64 = 0;
    let mut total_games: u32 = 0;
    let mut total_timeouts: u32 = 0;

    for r in &results {
        if r.timed_out {
            total_timeouts += 1;
            println!(
                "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>8} {:>7.1}s",
                r.level, r.game_idx, r.board_desc, r.label,
                "-", "-", "TIMEOUT", r.elapsed_secs,
            );
        } else {
            total_base_nodes += r.base_nodes;
            total_opt_nodes += r.opt_nodes;
            total_games += 1;

            let speedup = if r.opt_nodes > 0 {
                r.base_nodes as f64 / r.opt_nodes as f64
            } else if r.base_nodes > 0 {
                f64::INFINITY
            } else {
                1.0
            };

            println!(
                "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>7.1}x {:>7.1}s",
                r.level, r.game_idx, r.board_desc, r.label,
                r.base_nodes, r.opt_nodes, speedup, r.elapsed_secs,
            );
        }
    }

    let node_ratio = if total_opt_nodes > 0 {
        total_base_nodes as f64 / total_opt_nodes as f64
    } else {
        f64::INFINITY
    };

    println!("\n--- Summary ---");
    println!("Games:            {} ({} timed out)", total_games + total_timeouts, total_timeouts);
    println!("Baseline:         {} nodes", total_base_nodes);
    println!("Optimized:        {} nodes", total_opt_nodes);
    println!("Node reduction:   {:.2}x", node_ratio);
}
