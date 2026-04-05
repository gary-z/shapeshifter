use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use rand::SeedableRng;
use wait_timeout::ChildExt;
use shapeshifter::generate::generate_game;
use shapeshifter::level::LevelSpec;
use shapeshifter::subgame::game::SubgameGame;
use shapeshifter::subgame::generate::{
    board_col_deficits, board_row_deficits, piece_col_profile, piece_row_profile,
};
use shapeshifter::subgame::piece::SubgamePiece;
use shapeshifter::subgame::solver::{SubgameSolver, SubgamePruningConfig, SubgameSolveResult};

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

/// Find a no-wrapping subgame for the given level spec and game index.
fn find_subgames(
    spec: &LevelSpec,
    game_idx: u32,
    base_seed: u64,
) -> Option<(SubgameGame, SubgameGame, u64, usize)> {
    for skip in 1..spec.shapes as usize {
        for trial in 0..20u64 {
            let seed = base_seed + spec.level as u64 * 10000 + game_idx as u64 * 100 + trial;
            if let Some(sg) = generate_subgames(spec, seed, skip) {
                return Some((sg.0, sg.1, seed, skip));
            }
        }
    }
    None
}

// --- Worker mode: solve a single subgame task ---

fn run_worker(level: u32, game_idx: u32, axis: &str, seed: u64, skip: usize) {
    let spec = shapeshifter::level::get_level(level).expect("invalid level");

    let (row_sg, col_sg) = generate_subgames(&spec, seed, skip)
        .expect("failed to regenerate subgame");

    let sg = match axis {
        "row" => row_sg,
        "col" => col_sg,
        _ => panic!("invalid axis"),
    };

    let baseline_config = SubgamePruningConfig::none().only(|c| {
        c.total_deficit = true;
    });

    let solver = SubgameSolver::with_config(sg.clone(), baseline_config);
    let (_, base_stats) = solver.solve();

    let solver = SubgameSolver::new(sg);
    let (_, opt_stats) = solver.solve();

    // Output: base_nodes opt_nodes
    println!("{} {}", base_stats.nodes_visited, opt_stats.nodes_visited);
}

// --- Bench mode: spawn worker subprocesses ---

struct Task {
    level: u32,
    game_idx: u32,
    board_desc: String,
    label: &'static str,
    seed: u64,
    skip: usize,
}

struct TaskResult {
    level: u32,
    game_idx: u32,
    board_desc: String,
    label: &'static str,
    base_nodes: Option<u64>,
    opt_nodes: Option<u64>,
    status: String,
}

fn run_task(
    task: &Task,
    exe_path: &std::path::Path,
    timeout_secs: u64,
) -> TaskResult {
    let mut cmd = Command::new(exe_path);
    cmd.arg("--worker")
        .arg(task.level.to_string())
        .arg(task.game_idx.to_string())
        .arg(task.label)
        .arg(task.seed.to_string())
        .arg(task.skip.to_string());
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to spawn worker: {}", e);
            return TaskResult {
                level: task.level, game_idx: task.game_idx,
                board_desc: task.board_desc.clone(), label: task.label,
                base_nodes: None, opt_nodes: None, status: "ERROR".into(),
            };
        }
    };

    let timeout = Duration::from_secs(timeout_secs);
    let wait_result = child.wait_timeout(timeout);
    let stdout_pipe = child.stdout.take();

    match wait_result {
        Ok(Some(status)) if status.success() => {
            if let Some(mut pipe) = stdout_pipe {
                use std::io::Read;
                let mut stdout = String::new();
                let _ = pipe.read_to_string(&mut stdout);
                if let Some(line) = stdout.lines().next() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        return TaskResult {
                            level: task.level, game_idx: task.game_idx,
                            board_desc: task.board_desc.clone(), label: task.label,
                            base_nodes: parts[0].parse().ok(),
                            opt_nodes: parts[1].parse().ok(),
                            status: "OK".into(),
                        };
                    }
                }
            }
            TaskResult {
                level: task.level, game_idx: task.game_idx,
                board_desc: task.board_desc.clone(), label: task.label,
                base_nodes: None, opt_nodes: None, status: "ERROR".into(),
            }
        }
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            TaskResult {
                level: task.level, game_idx: task.game_idx,
                board_desc: task.board_desc.clone(), label: task.label,
                base_nodes: None, opt_nodes: None, status: "TIMEOUT".into(),
            }
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            TaskResult {
                level: task.level, game_idx: task.game_idx,
                board_desc: task.board_desc.clone(), label: task.label,
                base_nodes: None, opt_nodes: None, status: "ERROR".into(),
            }
        }
    }
}

fn print_usage() {
    eprintln!(
        "Usage: bench_subgame [OPTIONS]\n\n\
         Benchmark the subgame solver on projected subgames from full 2D games.\n\n\
         Options:\n  \
           --start LEVEL    Start level (default: 1)\n  \
           --end LEVEL      End level (default: 20)\n  \
           --games-per N    Games per level (default: 10)\n  \
           --timeout SECS   Timeout per task in seconds (default: 30)\n  \
           --seed SEED      Base random seed (default: 0)\n  \
           -h, --help       Show this help"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Worker mode: bench_subgame --worker <level> <game_idx> <axis> <seed> <skip>
    if args.len() >= 7 && args[1] == "--worker" {
        let level: u32 = args[2].parse().expect("invalid level");
        let game_idx: u32 = args[3].parse().expect("invalid game_idx");
        let axis = &args[4];
        let seed: u64 = args[5].parse().expect("invalid seed");
        let skip: usize = args[6].parse().expect("invalid skip");
        run_worker(level, game_idx, axis, seed, skip);
        return;
    }

    // Bench mode.
    let mut start_level: u32 = 1;
    let mut end_level: u32 = 20;
    let mut games_per: u32 = 10;
    let mut base_seed: u64 = 0;
    let mut timeout_secs: u64 = 30;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--start" => { i += 1; start_level = args[i].parse().expect("invalid start level"); }
            "--end" => { i += 1; end_level = args[i].parse().expect("invalid end level"); }
            "--games-per" => { i += 1; games_per = args[i].parse().expect("invalid games-per"); }
            "--timeout" => { i += 1; timeout_secs = args[i].parse().expect("invalid timeout"); }
            "--seed" => { i += 1; base_seed = args[i].parse().expect("invalid seed"); }
            "-h" | "--help" => { print_usage(); std::process::exit(0); }
            other => { eprintln!("Unknown option: {}", other); print_usage(); std::process::exit(1); }
        }
        i += 1;
    }

    let exe_path = std::env::current_exe().unwrap();

    // Build tasks.
    let levels = shapeshifter::level::load_levels();
    let mut tasks: Vec<Task> = Vec::new();

    for spec in &levels {
        if spec.level < start_level || spec.level > end_level { continue; }
        let board_desc = format!("{}x{}/M{}", spec.rows, spec.columns, spec.shifts);

        for g in 0..games_per {
            if let Some((_, _, seed, skip)) = find_subgames(spec, g, base_seed) {
                tasks.push(Task {
                    level: spec.level, game_idx: g, board_desc: board_desc.clone(),
                    label: "row", seed, skip,
                });
                tasks.push(Task {
                    level: spec.level, game_idx: g, board_desc: board_desc.clone(),
                    label: "col", seed, skip,
                });
            }
        }
    }

    let n_tasks = tasks.len();
    let n_workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    eprintln!(
        "Benchmarking {} tasks, {} workers, {}s timeout",
        n_tasks, n_workers, timeout_secs,
    );

    // Run tasks in parallel via thread pool + subprocess spawning.
    let (result_tx, result_rx) = mpsc::channel();
    let task_iter = std::sync::Mutex::new(tasks.into_iter());

    std::thread::scope(|scope| {
        for _ in 0..n_workers {
            let result_tx = result_tx.clone();
            let task_iter = &task_iter;
            let exe_path = &exe_path;
            scope.spawn(move || {
                loop {
                    let task = {
                        let mut iter = task_iter.lock().unwrap();
                        iter.next()
                    };
                    let task = match task {
                        Some(t) => t,
                        None => break,
                    };
                    let r = run_task(&task, exe_path, timeout_secs);
                    let _ = result_tx.send(r);
                }
            });
        }
        drop(result_tx);

        // Print results as they arrive.
        println!(
            "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>8}",
            "Level", "Game", "Board", "Type", "Base", "Opt", "Speedup"
        );
        println!("{}", "-".repeat(60));

        let mut total_base: u64 = 0;
        let mut total_opt: u64 = 0;
        let mut n_ok: u32 = 0;
        let mut n_timeout: u32 = 0;
        let mut results: Vec<TaskResult> = Vec::new();

        for r in result_rx {
            match r.status.as_str() {
                "OK" => {
                    let base = r.base_nodes.unwrap_or(0);
                    let opt = r.opt_nodes.unwrap_or(0);
                    total_base += base;
                    total_opt += opt;
                    n_ok += 1;
                    let speedup = if opt > 0 { base as f64 / opt as f64 } else { f64::INFINITY };
                    println!(
                        "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>7.1}x",
                        r.level, r.game_idx, r.board_desc, r.label, base, opt, speedup,
                    );
                }
                "TIMEOUT" => {
                    n_timeout += 1;
                    println!(
                        "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>8}",
                        r.level, r.game_idx, r.board_desc, r.label, "-", "-", "TIMEOUT",
                    );
                }
                _ => {
                    println!(
                        "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>8}",
                        r.level, r.game_idx, r.board_desc, r.label, "-", "-", "ERROR",
                    );
                }
            }
            let _ = std::io::stdout().flush();
            results.push(r);
        }

        let ratio = if total_opt > 0 { total_base as f64 / total_opt as f64 } else { f64::INFINITY };
        println!("\n--- Summary ---");
        println!("Games:            {} ({} timed out)", n_ok + n_timeout, n_timeout);
        println!("Baseline:         {} nodes", total_base);
        println!("Optimized:        {} nodes", total_opt);
        println!("Node reduction:   {:.2}x", ratio);
    });
}
