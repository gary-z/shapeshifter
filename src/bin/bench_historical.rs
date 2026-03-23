use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use wait_timeout::ChildExt;

use shapeshifter::puzzle::PuzzleJson;

const TIMEOUT_SECS: u64 = 120;

struct Task {
    idx: usize,
    level: u32,
    n_pieces: usize,
    json: String,
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "data/puzzle_history.jsonl".to_string()
    });
    let content = std::fs::read_to_string(&path).expect("failed to read file");
    let puzzles: Vec<PuzzleJson> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("bad JSONL"))
        .collect();

    let max_parallel = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let exe = std::env::current_exe().unwrap();
    let solve_one = exe.parent().unwrap().join("solve_one");
    if !solve_one.exists() {
        eprintln!("Error: {} not found. Run: cargo build --release --bin solve_one", solve_one.display());
        std::process::exit(1);
    }

    let tasks: Vec<Task> = puzzles.iter().enumerate().map(|(idx, puz)| {
        Task {
            idx,
            level: puz.level,
            n_pieces: puz.pieces.len(),
            json: serde_json::to_string(puz).unwrap(),
        }
    }).collect();
    let total = tasks.len();

    eprintln!("Benchmarking {} puzzles ({}s timeout, {} parallel)", total, TIMEOUT_SECS, max_parallel);

    println!(
        "{:<8} {:<6} {:>12} {:>12} {:<8}",
        "Level", "Pcs", "Nodes", "Time", "Result"
    );
    println!("{}", "-".repeat(54));

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

                    let r = run_task(&task, solve_one);
                    let _ = result_tx.send(r);
                }
            });
        }
        drop(result_tx);

        let mut results: Vec<(usize, u32, usize, Option<u64>, Option<u64>, String)> =
            Vec::with_capacity(total);
        for r in result_rx {
            results.push(r);
        }
        results.sort_by_key(|r| r.0);

        let mut ok = 0u32;
        let mut fail = 0u32;
        let mut timeout = 0u32;

        for &(_, level, n_pieces, nodes, elapsed_ms, ref status) in &results {
            match status.as_str() {
                "OK" => ok += 1,
                "FAIL" => fail += 1,
                _ => timeout += 1,
            }
            let nodes_str = nodes.map(|n| n.to_string()).unwrap_or("-".to_string());
            let time_str = elapsed_ms
                .map(|ms| format!("{:.3?}", Duration::from_millis(ms)))
                .unwrap_or(format!(">{}s", TIMEOUT_SECS));
            println!(
                "{:<8} {:<6} {:>12} {:>12} {:<8}",
                level, n_pieces, nodes_str, time_str, status
            );
        }

        println!("{}", "-".repeat(54));
        println!("{} ok, {} fail, {} timeout", ok, fail, timeout);
    });
}

fn run_task(task: &Task, solve_one: &std::path::Path)
    -> (usize, u32, usize, Option<u64>, Option<u64>, String)
{
    let mut child = match Command::new(solve_one)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return (task.idx, task.level, task.n_pieces, None, None, "ERROR".into()),
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(task.json.as_bytes());
    }

    let timeout = Duration::from_secs(TIMEOUT_SECS);
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
                        let status = if solved { "OK" } else { "FAIL" };
                        return (task.idx, task.level, task.n_pieces, nodes, elapsed_ms, status.into());
                    }
                }
            }
            (task.idx, task.level, task.n_pieces, None, None, "ERROR".into())
        }
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            (task.idx, task.level, task.n_pieces, None, None, "TIMEOUT".into())
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            (task.idx, task.level, task.n_pieces, None, None, "ERROR".into())
        }
    }
}
