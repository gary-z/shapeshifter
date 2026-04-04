use std::time::Instant;

use rand::SeedableRng;
use shapeshifter::generate::generate_game;
use shapeshifter::level::LevelSpec;
use shapeshifter::subgame::game::SubgameGame;
use shapeshifter::subgame::generate::{
    board_col_deficits, board_row_deficits, piece_col_profile, piece_row_profile,
};
use shapeshifter::subgame::piece::SubgamePiece;
use shapeshifter::subgame::solver;

fn print_usage() {
    eprintln!(
        "Usage: bench_subgame [OPTIONS]\n\n\
         Benchmark the subgame solver on projected subgames from full 2D games.\n\
         Pieces are sorted using the main solver's ordering.\n\n\
         Options:\n  \
           --start LEVEL    Start level (default: 1)\n  \
           --end LEVEL      End level (default: 20)\n  \
           --games-per N    Games per level (default: 10)\n  \
           --seed SEED      Base random seed (default: 0)\n  \
           -h, --help       Show this help"
    );
}

/// Generate a full 2D game, sort pieces like the main solver, project to
/// row and column subgames.
fn generate_subgames(
    spec: &LevelSpec,
    seed: u64,
) -> (SubgameGame, SubgameGame) {
    let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
    let game = generate_game(spec, &mut rng);
    let h = spec.rows;
    let w = spec.columns;

    // Sort pieces like the main solver: fewer placements first, then
    // larger perimeter, larger cell count, limbs tiebreak.
    let pieces = game.pieces();
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
    let sorted_pieces: Vec<_> = indexed.iter().map(|(i, _)| &pieces[*i]).collect();

    let row_profiles: Vec<SubgamePiece> =
        sorted_pieces.iter().map(|p| piece_row_profile(p)).collect();
    let col_profiles: Vec<SubgamePiece> =
        sorted_pieces.iter().map(|p| piece_col_profile(p)).collect();

    let row_board = board_row_deficits(game.board());
    let col_board = board_col_deficits(game.board());

    let row_sg = SubgameGame::new(row_board, row_profiles);
    let col_sg = SubgameGame::new(col_board, col_profiles);
    (row_sg, col_sg)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut start_level: u32 = 1;
    let mut end_level: u32 = 20;
    let mut games_per: u32 = 10;
    let mut base_seed: u64 = 0;

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

    println!(
        "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>8}",
        "Level", "Game", "Board", "Type", "Base", "Opt", "Speedup"
    );
    println!("{}", "-".repeat(60));

    let mut total_base_nodes: u64 = 0;
    let mut total_opt_nodes: u64 = 0;
    let mut total_base_us: u128 = 0;
    let mut total_opt_us: u128 = 0;
    let mut total_games: u32 = 0;

    let levels = shapeshifter::level::load_levels();

    for spec in &levels {
        if spec.level < start_level || spec.level > end_level {
            continue;
        }
        let board_desc = format!("{}x{}/M{}", spec.rows, spec.columns, spec.shifts);

        for g in 0..games_per {
            let seed = base_seed + spec.level as u64 * 1000 + g as u64;
            let (row_sg, col_sg) = generate_subgames(spec, seed);

            for (label, sg) in [("row", row_sg), ("col", col_sg)] {
                let start = Instant::now();
                let (_, base_stats) = solver::solve_baseline(sg.clone());
                let base_elapsed = start.elapsed();

                let start = Instant::now();
                let (_, opt_stats) = solver::solve(sg);
                let opt_elapsed = start.elapsed();

                total_base_nodes += base_stats.nodes_visited;
                total_opt_nodes += opt_stats.nodes_visited;
                total_base_us += base_elapsed.as_micros();
                total_opt_us += opt_elapsed.as_micros();
                total_games += 1;

                let speedup = if opt_stats.nodes_visited > 0 {
                    base_stats.nodes_visited as f64 / opt_stats.nodes_visited as f64
                } else if base_stats.nodes_visited > 0 {
                    f64::INFINITY
                } else {
                    1.0
                };

                println!(
                    "{:<6} {:<4} {:<10} {:<5} {:>10} {:>10} {:>7.1}x",
                    spec.level,
                    g,
                    board_desc,
                    label,
                    base_stats.nodes_visited,
                    opt_stats.nodes_visited,
                    speedup,
                );
            }
        }
    }

    let node_ratio = if total_opt_nodes > 0 {
        total_base_nodes as f64 / total_opt_nodes as f64
    } else {
        f64::INFINITY
    };
    let time_ratio = if total_opt_us > 0 {
        total_base_us as f64 / total_opt_us as f64
    } else {
        f64::INFINITY
    };

    println!("\n--- Summary ---");
    println!("Games:            {}", total_games);
    println!(
        "Baseline:         {} nodes, {:.3} ms",
        total_base_nodes,
        total_base_us as f64 / 1000.0
    );
    println!(
        "Optimized:        {} nodes, {:.3} ms",
        total_opt_nodes,
        total_opt_us as f64 / 1000.0
    );
    println!("Node reduction:   {:.2}x", node_ratio);
    println!("Time reduction:   {:.2}x", time_ratio);
}
