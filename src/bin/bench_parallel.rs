/// Benchmark for the parallel solver with per-game process-level timeouts.
/// Each game runs in a child process using `solve_one` with PARALLEL=1,
/// so the parallel solver (with placement tie-shuffling) gets all cores.
/// Games run sequentially (one at a time, all cores per game).
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

use rand::SeedableRng;
use shapeshifter::generate::generate_for_level;
use shapeshifter::level::get_level;

const TIMEOUT_SECS: u64 = 5;
const GAMES_PER_LEVEL: u32 = 5;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let start_level = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(36u32);
    let end_level = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50u32);
    let games_per = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(GAMES_PER_LEVEL);

    let exe = std::env::current_exe().unwrap();
    let solve_one = exe.parent().unwrap().join("solve_one");
    if !solve_one.exists() {
        eprintln!("Error: {} not found", solve_one.display());
        std::process::exit(1);
    }

    eprintln!(
        "Parallel benchmark: levels {}-{}, {} games each, {}s timeout",
        start_level, end_level, games_per, TIMEOUT_SECS,
    );

    println!(
        "{:<8} {:<5} {:<6} {:<10} {:>12} {:>10} {:<8}",
        "Level", "Game", "Pcs", "Board", "Nodes", "Time", "Result"
    );
    println!("{}", "-".repeat(65));

    struct LevelStats {
        level: u32,
        board: String,
        ok: u32,
        total: u32,
    }
    let mut level_stats: Vec<LevelStats> = Vec::new();

    for level in start_level..=end_level {
        let spec = match get_level(level) {
            Some(s) => s,
            None => continue,
        };
        let board_desc = format!("{}x{}/M{}", spec.rows, spec.columns, spec.shifts);
        let mut level_ok = 0u32;

        for g in 0..games_per {
            let seed = level as u64 * 1000 + g as u64;
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
            let game = generate_for_level(level, &mut rng).unwrap();
            let n_pieces = game.pieces().len();

            // Serialize puzzle for worker.
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

            let mut child = Command::new(&solve_one)
                .env("PARALLEL", "1")
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

            let (nodes_str, time_str, status) = match wait_result {
                Ok(Some(exit)) if exit.success() => {
                    if let Some(mut pipe) = stdout_pipe {
                        use std::io::Read;
                        let mut stdout = String::new();
                        let _ = pipe.read_to_string(&mut stdout);
                        if let Some(line) = stdout.lines().next() {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 3 {
                                let nodes = parts[0].to_string();
                                let ms: u64 = parts[1].parse().unwrap_or(0);
                                let solved = parts[2] == "true";
                                let time = format!("{:.3?}", Duration::from_millis(ms));
                                if solved {
                                    level_ok += 1;
                                    (nodes, time, "OK".to_string())
                                } else {
                                    (nodes, time, "FAIL".to_string())
                                }
                            } else {
                                ("-".to_string(), "-".to_string(), "ERROR".to_string())
                            }
                        } else {
                            ("-".to_string(), "-".to_string(), "ERROR".to_string())
                        }
                    } else {
                        ("-".to_string(), "-".to_string(), "ERROR".to_string())
                    }
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    ("-".to_string(), format!(">{}s", TIMEOUT_SECS), "TIMEOUT".to_string())
                }
            };

            println!(
                "{:<8} {:<5} {:<6} {:<10} {:>12} {:>10} {:<8}",
                level, g, n_pieces, board_desc, nodes_str, time_str, status,
            );
        }

        level_stats.push(LevelStats {
            level,
            board: board_desc,
            ok: level_ok,
            total: games_per,
        });
    }

    // Summary.
    println!("\n{:<8} {:<7} {:<7} {:<10}", "Level", "OK", "Rate", "Board");
    println!("{}", "-".repeat(35));
    let mut all_ok = 0u32;
    let mut all_total = 0u32;
    let mut last_consistent = 0u32;
    let mut all_consistent = true;
    for s in &level_stats {
        let rate = s.ok as f64 / s.total as f64 * 100.0;
        if rate == 100.0 && all_consistent {
            last_consistent = s.level;
        } else if rate < 100.0 {
            all_consistent = false;
        }
        println!("{:<8} {:>2}/{:<4} {:>5.0}%  {:<10}", s.level, s.ok, s.total, rate, s.board);
        all_ok += s.ok;
        all_total += s.total;
    }
    println!("\n{}/{} solved", all_ok, all_total);
    println!("Consistently 100% through level: {}", last_consistent);
}
