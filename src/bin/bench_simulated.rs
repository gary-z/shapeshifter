use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use wait_timeout::ChildExt;

use rand::SeedableRng;
use shapeshifter::generate::generate_for_level;
use shapeshifter::level::get_level;

const TIMEOUT_SECS: u64 = 5;
const GAMES_PER_LEVEL: u32 = 5;

struct Task {
    level: u32,
    game_idx: u32,
    n_pieces: usize,
    board_desc: String,
    json: String,
}

struct Result {
    level: u32,
    game_idx: u32,
    n_pieces: usize,
    board_desc: String,
    nodes: Option<u64>,
    elapsed_ms: Option<u64>,
    status: String, // "OK", "FAIL", "TIMEOUT"
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let start_level = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1u32);
    let end_level = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50u32);
    let games_per = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(GAMES_PER_LEVEL);

    let max_parallel = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    // Build all tasks.
    let mut tasks: Vec<Task> = Vec::new();
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
            // Serialize the puzzle for the worker.
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
            tasks.push(Task {
                level, game_idx: g, n_pieces, board_desc,
                json: puz_json.to_string(),
            });
        }
    }

    let total = tasks.len();
    eprintln!(
        "Simulating levels {}-{}, {} games each ({}s timeout, {} tasks, {} parallel)",
        start_level, end_level, games_per, TIMEOUT_SECS, total, max_parallel,
    );

    // Find the worker binary.
    let exe = std::env::current_exe().unwrap();
    let solve_one = exe.parent().unwrap().join("solve_one");
    if !solve_one.exists() {
        eprintln!("Error: {} not found. Run: cargo build --release --bin solve_one", solve_one.display());
        std::process::exit(1);
    }

    // Process tasks with bounded parallelism using a thread pool.
    let (result_tx, result_rx) = mpsc::channel();
    let task_iter = std::sync::Mutex::new(tasks.into_iter());

    std::thread::scope(|scope| {
        for _ in 0..max_parallel {
            let task_iter = &task_iter;
            let result_tx = result_tx.clone();
            let solve_one = &solve_one;

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

                    let result = run_task(&task, solve_one, TIMEOUT_SECS);
                    let _ = result_tx.send(result);
                }
            });
        }
        drop(result_tx);

        // Collect and store results as they arrive.
        let mut results: Vec<Result> = Vec::with_capacity(total);
        for r in result_rx {
            results.push(r);
        }

        // Sort by (level, game_idx) for ordered output.
        results.sort_by_key(|r| (r.level, r.game_idx));

        // Print detailed results.
        println!(
            "{:<8} {:<5} {:<6} {:<6} {:>12} {:>12} {:<8}",
            "Level", "Game", "Pcs", "Board", "Nodes", "Time", "Result"
        );
        println!("{}", "-".repeat(68));

        let mut ok = 0u32;
        let mut fail = 0u32;
        let mut timeout = 0u32;

        for r in &results {
            match r.status.as_str() {
                "OK" => ok += 1,
                "FAIL" => fail += 1,
                _ => timeout += 1,
            }
            let nodes_str = r.nodes.map(|n| n.to_string()).unwrap_or("-".to_string());
            let time_str = r.elapsed_ms
                .map(|ms| format!("{:.3?}", Duration::from_millis(ms)))
                .unwrap_or(format!(">{}s", TIMEOUT_SECS));
            println!(
                "{:<8} {:<5} {:<6} {:<6} {:>12} {:>12} {:<8}",
                r.level, r.game_idx, r.n_pieces, r.board_desc, nodes_str, time_str, r.status,
            );
        }

        // Summary per level.
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
                if results[i].status == "OK" { level_ok += 1; }
                i += 1;
            }
            let rate = level_ok as f64 / level_total as f64 * 100.0;
            if rate == 100.0 && all_consistent {
                last_consistent = level;
            } else if rate < 100.0 {
                all_consistent = false;
            }
            println!("{:<8} {:>2}/{:<4} {:>5.0}%  {:<10}", level, level_ok, level_total, rate, board);
        }

        println!("\n{} ok, {} fail, {} timeout", ok, fail, timeout);
        println!("Consistently 100% through level: {}", last_consistent);
    });
}

fn run_task(task: &Task, solve_one: &std::path::Path, timeout_secs: u64) -> Result {
    let mut child = match Command::new(solve_one)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to spawn worker: {}", e);
            return Result {
                level: task.level, game_idx: task.game_idx,
                n_pieces: task.n_pieces, board_desc: task.board_desc.clone(),
                nodes: None, elapsed_ms: None, status: "ERROR".to_string(),
            };
        }
    };

    // Write puzzle JSON to worker's stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(task.json.as_bytes());
    }

    // Wait with timeout.
    let timeout = Duration::from_secs(timeout_secs);
    let wait_result = child.wait_timeout(timeout);
    let stdout_pipe = child.stdout.take();

    match wait_result {
        Ok(Some(status)) if status.success() => {
            // Read stdout from captured pipe.
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
                        return Result {
                            level: task.level, game_idx: task.game_idx,
                            n_pieces: task.n_pieces, board_desc: task.board_desc.clone(),
                            nodes, elapsed_ms,
                            status: if solved { "OK" } else { "FAIL" }.to_string(),
                        };
                    }
                }
            }
            Result {
                level: task.level, game_idx: task.game_idx,
                n_pieces: task.n_pieces, board_desc: task.board_desc.clone(),
                nodes: None, elapsed_ms: None, status: "ERROR".to_string(),
            }
        }
        Ok(None) => {
            // Timeout — kill the process.
            let _ = child.kill();
            let _ = child.wait();
            Result {
                level: task.level, game_idx: task.game_idx,
                n_pieces: task.n_pieces, board_desc: task.board_desc.clone(),
                nodes: None, elapsed_ms: None, status: "TIMEOUT".to_string(),
            }
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            Result {
                level: task.level, game_idx: task.game_idx,
                n_pieces: task.n_pieces, board_desc: task.board_desc.clone(),
                nodes: None, elapsed_ms: None, status: "ERROR".to_string(),
            }
        }
    }
}
