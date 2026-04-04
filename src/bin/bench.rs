use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use wait_timeout::ChildExt;

use rand::SeedableRng;
use shapeshifter::generate::generate_for_level;
use shapeshifter::level::get_level;
use shapeshifter::puzzle::PuzzleJson;

struct Task {
    level: u32,
    game_idx: u32,
    n_pieces: usize,
    board_desc: String,
    json: String,
}

struct TaskResult {
    level: u32,
    game_idx: u32,
    n_pieces: usize,
    board_desc: String,
    nodes: Option<u64>,
    elapsed_ms: Option<u64>,
    status: String,
}

fn find_solver() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let solve = exe.parent().unwrap().join("solve");
    if !solve.exists() {
        eprintln!(
            "Error: {} not found. Run: cargo build --release --bin solve",
            solve.display()
        );
        std::process::exit(1);
    }
    solve
}

fn run_task(
    task: &Task,
    solver_path: &std::path::Path,
    timeout_secs: u64,
    parallel: bool,
    exhaustive: bool,
    subgame: bool,
) -> TaskResult {
    let mut cmd = Command::new(solver_path);
    cmd.arg("--worker");
    if parallel {
        cmd.arg("--parallel");
    }
    if exhaustive {
        cmd.arg("--exhaustive");
    }
    if subgame {
        cmd.arg("--subgame");
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to spawn worker: {}", e);
            return TaskResult {
                level: task.level,
                game_idx: task.game_idx,
                n_pieces: task.n_pieces,
                board_desc: task.board_desc.clone(),
                nodes: None,
                elapsed_ms: None,
                status: "ERROR".to_string(),
            };
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(task.json.as_bytes());
    }

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
                    if parts.len() >= 3 {
                        let nodes = parts[0].parse().ok();
                        let elapsed_ms = parts[1].parse().ok();
                        let solved = parts[2] == "true";
                        return TaskResult {
                                        level: task.level,
                            game_idx: task.game_idx,
                            n_pieces: task.n_pieces,
                            board_desc: task.board_desc.clone(),
                            nodes,
                            elapsed_ms,
                            status: if solved { "OK" } else { "FAIL" }.to_string(),
                        };
                    }
                }
            }
            TaskResult {
                level: task.level,
                game_idx: task.game_idx,
                n_pieces: task.n_pieces,
                board_desc: task.board_desc.clone(),
                nodes: None,
                elapsed_ms: None,
                status: "ERROR".to_string(),
            }
        }
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            TaskResult {
                level: task.level,
                game_idx: task.game_idx,
                n_pieces: task.n_pieces,
                board_desc: task.board_desc.clone(),
                nodes: None,
                elapsed_ms: None,
                status: "TIMEOUT".to_string(),
            }
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            TaskResult {
                level: task.level,
                game_idx: task.game_idx,
                n_pieces: task.n_pieces,
                board_desc: task.board_desc.clone(),
                nodes: None,
                elapsed_ms: None,
                status: "ERROR".to_string(),
            }
        }
    }
}

fn game_to_json(game: &shapeshifter::game::Game, level: u32, spec: &shapeshifter::level::LevelSpec) -> String {
    let puz_json = serde_json::json!({
        "level": level,
        "m": spec.shifts,
        "rows": spec.rows,
        "columns": spec.columns,
        "board": (0..spec.rows).map(|r| {
            (0..spec.columns).map(|c| game.board().get(r as usize, c as usize) as u8).collect::<Vec<_>>()
        }).collect::<Vec<_>>(),
        "pieces": game.pieces().iter().map(|p| {
            (0..p.height() as usize).map(|r| {
                (0..p.width() as usize).map(|c| p.shape().get_bit((r * 15 + c) as u32)).collect::<Vec<_>>()
            }).collect::<Vec<_>>()
        }).collect::<Vec<_>>(),
    });
    puz_json.to_string()
}

fn build_simulated_tasks(
    start_level: u32,
    end_level: u32,
    games_per: u32,
) -> Vec<Task> {
    let mut tasks = Vec::new();
    for level in start_level..=end_level {
        let spec = match get_level(level) {
            Some(s) => s,
            None => continue,
        };
        for g in 0..games_per {
            let seed = level as u64 * 1000 + g as u64;
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
            let game = generate_for_level(level, &mut rng).unwrap();
            let n_pieces = game.pieces().len();
            let board_desc = format!("{}x{}/M{}", spec.rows, spec.columns, spec.shifts);
            let json = game_to_json(&game, level, &spec);
            tasks.push(Task {
                level,
                game_idx: g,
                n_pieces,
                board_desc,
                json,
            });
        }
    }
    tasks
}

fn build_historical_tasks(path: &str) -> Vec<Task> {
    let content = std::fs::read_to_string(path).expect("failed to read file");
    let puzzles: Vec<PuzzleJson> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("bad JSONL"))
        .collect();

    puzzles
        .iter()
        .enumerate()
        .map(|(idx, puz)| {
            let board_desc = format!("{}x{}/M{}", puz.rows, puz.columns, puz.m);
            Task {
                level: puz.level,
                game_idx: idx as u32,
                n_pieces: puz.pieces.len(),
                board_desc,
                json: serde_json::to_string(puz).unwrap(),
            }
        })
        .collect()
}

fn run_bench(
    tasks: Vec<Task>,
    solver_path: &std::path::Path,
    timeout_secs: u64,
    max_parallel: usize,
    parallel: bool,
    exhaustive: bool,
    subgame: bool,
    show_nodes_per_sec: bool,
) {
    let total = tasks.len();

    let (result_tx, result_rx) = mpsc::channel();
    let task_iter = std::sync::Mutex::new(tasks.into_iter());

    std::thread::scope(|scope| {
        for _ in 0..max_parallel {
            let task_iter = &task_iter;
            let result_tx = result_tx.clone();
            let solver_path = &solver_path;

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

                    let r = run_task(&task, solver_path, timeout_secs, parallel, exhaustive, subgame);
                    let _ = result_tx.send(r);
                }
            });
        }
        drop(result_tx);

        let mut results: Vec<TaskResult> = Vec::with_capacity(total);
        for r in result_rx {
            results.push(r);
        }
        results.sort_by_key(|r| (r.level, r.game_idx));

        // Print detailed results.
        if show_nodes_per_sec {
            println!(
                "{:<6} {:<4} {:<6} {:<10} {:>14} {:>10} {:>12} {:<8}",
                "Level", "Game", "Pcs", "Board", "Nodes", "Time", "Nodes/sec", "Status"
            );
            println!("{}", "-".repeat(80));
        } else {
            println!(
                "{:<8} {:<5} {:<6} {:<10} {:>12} {:>12} {:<8}",
                "Level", "Game", "Pcs", "Board", "Nodes", "Time", "Result"
            );
            println!("{}", "-".repeat(68));
        }

        let mut ok = 0u32;
        let mut fail = 0u32;
        let mut timeout = 0u32;

        for r in &results {
            match r.status.as_str() {
                "OK" | "DONE" => ok += 1,
                "FAIL" => fail += 1,
                "TIMEOUT" => timeout += 1,
                _ => {}
            }

            let nodes_str = r.nodes.map(|n| n.to_string()).unwrap_or("-".to_string());
            let time_str = r
                .elapsed_ms
                .map(|ms| format!("{:.3?}", Duration::from_millis(ms)))
                .unwrap_or(format!(">{}s", timeout_secs));

            if show_nodes_per_sec {
                let nps_str = match (r.nodes, r.elapsed_ms) {
                    (Some(n), Some(ms)) if ms > 0 => format!("{}", n * 1000 / ms),
                    _ => "-".to_string(),
                };
                println!(
                    "{:<6} {:<4} {:<6} {:<10} {:>14} {:>10} {:>12} {:<8}",
                    r.level, r.game_idx, r.n_pieces, r.board_desc, nodes_str, time_str, nps_str,
                    r.status,
                );
            } else {
                println!(
                    "{:<8} {:<5} {:<6} {:<10} {:>12} {:>12} {:<8}",
                    r.level, r.game_idx, r.n_pieces, r.board_desc, nodes_str, time_str, r.status,
                );
            }
        }

        // Per-level summary.
        println!("\n{:<8} {:<7} {:<7} {:<10}", "Level", "OK", "Rate", "Board");
        println!("{}", "-".repeat(35));

        let mut all_consistent = true;
        let mut last_consistent = 0u32;
        let mut i = 0;
        while i < results.len() {
            let level = results[i].level;
            let board = results[i].board_desc.clone();
            let mut level_ok = 0u32;
            let mut level_total = 0u32;
            while i < results.len() && results[i].level == level {
                level_total += 1;
                if results[i].status == "OK" || results[i].status == "DONE" {
                    level_ok += 1;
                }
                i += 1;
            }
            let rate = level_ok as f64 / level_total as f64 * 100.0;
            if rate == 100.0 && all_consistent {
                last_consistent = level;
            } else if rate < 100.0 {
                all_consistent = false;
            }
            println!(
                "{:<8} {:>2}/{:<4} {:>5.0}%  {:<10}",
                level, level_ok, level_total, rate, board
            );
        }

        println!("\n{} ok, {} fail, {} timeout", ok, fail, timeout);
        println!("Consistently 100% through level: {}", last_consistent);
    });
}

fn print_usage() {
    eprintln!(
        "Usage: bench <MODE> [OPTIONS]\n\n\
         Modes:\n  \
           simulated [START] [END]  Benchmark randomly generated puzzles (default: 1-50)\n  \
           historical [PATH]       Benchmark historical puzzles from JSONL file\n\n\
         Options:\n  \
           --parallel       Use parallel solver (each game gets all cores)\n  \
           --exhaustive     Explore full search tree (no early termination)\n  \
           --subgame        Enable subgame pruning\n  \
           --timeout SECS   Timeout per game (default: 5 for simulated, 60 for historical)\n  \
           --games-per N    Games per level for simulated mode (default: 5)\n  \
           -h, --help       Show this help"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let mut mode = None;
    let mut positional = Vec::new();
    let mut parallel = false;
    let mut exhaustive = false;
    let mut subgame = false;
    let mut timeout_secs: Option<u64> = None;
    let mut games_per: Option<u32> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--parallel" => parallel = true,
            "--exhaustive" => exhaustive = true,
            "--subgame" => subgame = true,
            "--timeout" => {
                i += 1;
                timeout_secs = Some(args[i].parse().expect("invalid timeout"));
            }
            "--games-per" => {
                i += 1;
                games_per = Some(args[i].parse().expect("invalid games-per"));
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            arg if !arg.starts_with('-') => {
                if mode.is_none() {
                    mode = Some(arg.to_string());
                } else {
                    positional.push(arg.to_string());
                }
            }
            other => {
                eprintln!("Unknown option: {}", other);
                print_usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let mode = mode.unwrap_or_else(|| {
        print_usage();
        std::process::exit(1);
    });

    let solver_path = find_solver();

    match mode.as_str() {
        "simulated" => {
            let start_level: u32 = positional.first().and_then(|s| s.parse().ok()).unwrap_or(1);
            let end_level: u32 = positional.get(1).and_then(|s| s.parse().ok()).unwrap_or(50);
            let games_per = games_per.unwrap_or(5);
            let timeout = timeout_secs.unwrap_or(if exhaustive { 120 } else { 5 });

            // When using parallel solver, run one game at a time (it uses all cores).
            // When using exhaustive, also run one at a time.
            let max_parallel = if parallel || exhaustive {
                1
            } else {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
            };

            let tasks = build_simulated_tasks(start_level, end_level, games_per);
            eprintln!(
                "Benchmarking levels {}-{}, {} games each ({}s timeout, {} tasks, {} workers)",
                start_level,
                end_level,
                games_per,
                timeout,
                tasks.len(),
                max_parallel,
            );

            run_bench(
                tasks,
                &solver_path,
                timeout,
                max_parallel,
                parallel,
                exhaustive,
                subgame,
                exhaustive,
            );
        }
        "historical" => {
            let path = positional
                .first()
                .map(|s| s.as_str())
                .unwrap_or("data/puzzle_history.jsonl");
            let timeout = timeout_secs.unwrap_or(60);

            let max_parallel = if parallel {
                1
            } else {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
            };

            let tasks = build_historical_tasks(path);
            eprintln!(
                "Benchmarking {} historical puzzles ({}s timeout, {} workers)",
                tasks.len(),
                timeout,
                max_parallel,
            );

            run_bench(
                tasks,
                &solver_path,
                timeout,
                max_parallel,
                parallel,
                exhaustive,
                subgame,
                false,
            );
        }
        other => {
            eprintln!("Unknown mode: {}", other);
            print_usage();
            std::process::exit(1);
        }
    }
}
