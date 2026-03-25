/// Benchmark parallel exhaustive search (full tree traversal).
/// Uses EXHAUSTIVE mode: solver explores the entire search space even after
/// finding a solution. Removes luck factor from parallel efficiency measurements.
///
/// Usage: bench_exhaustive [start_level] [end_level] [games_per_level]
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

use rand::SeedableRng;
use shapeshifter::generate::generate_for_level;
use shapeshifter::level::get_level;

const TIMEOUT_SECS: u64 = 120;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let start_level = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(41u32);
    let end_level = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50u32);
    let games_per: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);

    let exe = std::env::current_exe().unwrap();
    let solve_one = exe.parent().unwrap().join("solve_one");
    if !solve_one.exists() {
        eprintln!("Error: {} not found", solve_one.display());
        std::process::exit(1);
    }

    eprintln!(
        "Exhaustive benchmark: levels {}-{}, {} games each, {}s timeout",
        start_level, end_level, games_per, TIMEOUT_SECS,
    );

    println!(
        "{:<6} {:<4} {:<6} {:<10} {:>14} {:>10} {:>12} {:<8}",
        "Level", "Game", "Pcs", "Board", "Nodes", "Time", "Nodes/sec", "Status"
    );
    println!("{}", "-".repeat(80));

    for level in start_level..=end_level {
        let spec = match get_level(level) {
            Some(s) => s,
            None => continue,
        };
        let board_desc = format!("{}x{}/M{}", spec.rows, spec.columns, spec.shifts);

        for g in 0..games_per {
            let seed = level as u64 * 1000 + g as u64;
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
            let game = generate_for_level(level, &mut rng).unwrap();
            let n_pieces = game.pieces().len();

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

            // EXHAUSTIVE=1: don't abort on solution found, explore full tree.
            let mut child = Command::new(&solve_one)
                .env("EXHAUSTIVE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("failed to spawn solve_one");

            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(puz_json.to_string().as_bytes());
            }

            let timeout = Duration::from_secs(TIMEOUT_SECS);
            let wait_result = child.wait_timeout(timeout);
            let stdout_pipe = child.stdout.take();

            let (nodes_str, time_str, nps_str, status) = match wait_result {
                Ok(Some(exit)) if exit.success() => {
                    if let Some(mut pipe) = stdout_pipe {
                        use std::io::Read;
                        let mut stdout = String::new();
                        let _ = pipe.read_to_string(&mut stdout);
                        if let Some(line) = stdout.lines().next() {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 3 {
                                let nodes: u64 = parts[0].parse().unwrap_or(0);
                                let ms: u64 = parts[1].parse().unwrap_or(0);
                                let nps = if ms > 0 { nodes * 1000 / ms } else { 0 };
                                let time = format!("{:.3?}", Duration::from_millis(ms));
                                (format!("{}", nodes), time, format!("{}", nps), "DONE".to_string())
                            } else {
                                ("-".into(), "-".into(), "-".into(), "ERROR".into())
                            }
                        } else {
                            ("-".into(), "-".into(), "-".into(), "ERROR".into())
                        }
                    } else {
                        ("-".into(), "-".into(), "-".into(), "ERROR".into())
                    }
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    ("-".into(), format!(">{}s", TIMEOUT_SECS), "-".into(), "TIMEOUT".into())
                }
            };

            println!(
                "{:<6} {:<4} {:<6} {:<10} {:>14} {:>10} {:>12} {:<8}",
                level, g, n_pieces, board_desc, nodes_str, time_str, nps_str, status,
            );
        }
    }
}
