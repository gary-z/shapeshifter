use rand::SeedableRng;
use shapeshifter::generate::generate_for_level;
use shapeshifter::level::get_level;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut level: Option<u32> = None;
    let mut count: u32 = 1;
    let mut seed: Option<u64> = None;
    let mut start_level: Option<u32> = None;
    let mut end_level: Option<u32> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--count" | "-n" => {
                i += 1;
                count = args[i].parse().expect("invalid count");
            }
            "--seed" | "-s" => {
                i += 1;
                seed = Some(args[i].parse().expect("invalid seed"));
            }
            "--range" => {
                i += 1;
                start_level = Some(args[i].parse().expect("invalid start level"));
                i += 1;
                end_level = Some(args[i].parse().expect("invalid end level"));
            }
            "-h" | "--help" => {
                eprintln!(
                    "Usage: generate [LEVEL] [OPTIONS]\n\n\
                     Generates puzzles in the same JSON format as puzzle_history.jsonl.\n\
                     Output is JSONL (one puzzle per line) to stdout.\n\n\
                     Arguments:\n  \
                       LEVEL             Level number (1-100)\n\n\
                     Options:\n  \
                       -n, --count N     Number of puzzles to generate (default: 1)\n  \
                       -s, --seed S      RNG seed for reproducibility\n  \
                       --range START END Generate one puzzle per level in range\n  \
                       -h, --help        Show this help"
                );
                std::process::exit(0);
            }
            arg => {
                if level.is_none() {
                    level = Some(arg.parse().unwrap_or_else(|_| {
                        eprintln!("Invalid level: {}", arg);
                        std::process::exit(1);
                    }));
                } else {
                    eprintln!("Unexpected argument: {}", arg);
                    std::process::exit(1);
                }
            }
        }
        i += 1;
    }

    if let (Some(start), Some(end)) = (start_level, end_level) {
        // Range mode: one puzzle per level.
        let base_seed = seed.unwrap_or(42);
        for lvl in start..=end {
            let spec = match get_level(lvl) {
                Some(s) => s,
                None => {
                    eprintln!("Skipping unknown level {}", lvl);
                    continue;
                }
            };
            let mut rng = rand::rngs::SmallRng::seed_from_u64(base_seed + lvl as u64);
            let game = generate_for_level(lvl, &mut rng).unwrap();
            print_puzzle(&game, lvl, &spec);
        }
        return;
    }

    let level = level.unwrap_or_else(|| {
        eprintln!("Error: level number required. Use --help for usage.");
        std::process::exit(1);
    });

    let spec = get_level(level).unwrap_or_else(|| {
        eprintln!("Unknown level: {}", level);
        std::process::exit(1);
    });

    let base_seed = seed.unwrap_or(42);
    for g in 0..count {
        let s = base_seed.wrapping_add(g as u64);
        let mut rng = rand::rngs::SmallRng::seed_from_u64(s);
        let game = generate_for_level(level, &mut rng).unwrap();
        print_puzzle(&game, level, &spec);
    }
}

fn print_puzzle(game: &shapeshifter::game::Game, level: u32, spec: &shapeshifter::level::LevelSpec) {
    let puz = serde_json::json!({
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
        "icons": Vec::<String>::new(),
    });
    println!("{}", serde_json::to_string(&puz).unwrap());
}
