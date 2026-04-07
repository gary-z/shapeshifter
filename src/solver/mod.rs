mod backtrack;
mod precompute;
pub(crate) mod prune;
pub(crate) mod pruning;

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::core::bitboard::Bitboard;
use crate::core::coverage::CoverageCounter;
use crate::game::Game;

/// Format a count with SI suffix (e.g. 1234567 → "1.2M nodes").
fn format_count(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B nodes", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M nodes", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}K nodes", n as f64 / 1e3)
    } else {
        format!("{} nodes", n)
    }
}

/// A solution is a list of (row, col) placements, one per piece in original order.
pub type Solution = Vec<(usize, usize)>;

/// Result of a solve attempt: optional solution + number of nodes visited.
pub struct SolveResult {
    pub solution: Option<Solution>,
    pub nodes_visited: u64,
    /// Final progress fraction (0.0–1.0) of naive search space explored.
    /// Only meaningful for parallel solves; 0.0 for serial.
    pub progress: f64,
}

/// Configuration controlling which pruning techniques are enabled.
#[derive(Clone)]
pub struct PruningConfig {
    pub active_planes: bool,
    pub total_deficit_global: bool,
    pub total_deficit_rowcol: bool,
    pub total_deficit_diagonal: bool,
    pub coverage: bool,
    pub jaggedness: bool,
    pub cell_locking: bool,
    pub single_cell_endgame: bool,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            active_planes: true,
            total_deficit_global: true,
            total_deficit_rowcol: true,
            total_deficit_diagonal: true,
            coverage: true,
            jaggedness: true,
            cell_locking: true,
            single_cell_endgame: true,
        }
    }
}

impl PruningConfig {
    /// All pruning disabled.
    pub fn none() -> Self {
        Self {
            active_planes: false,
            total_deficit_global: false,
            total_deficit_rowcol: false,
            total_deficit_diagonal: false,
            coverage: false,
            jaggedness: false,
            cell_locking: false,
            single_cell_endgame: false,
        }
    }

    /// Only the specified prune enabled.
    pub fn only(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }
}

/// All precomputed data needed by the backtracking solver.
/// Bundled into a single struct to keep the backtrack signature small.
pub(crate) struct SolverData {
    pub(crate) all_placements: Vec<Vec<(usize, usize, Bitboard)>>,
    pub(crate) total_deficit_prune: prune::total_deficit::TotalDeficitPrune,
    pub(crate) jaggedness_prune: prune::jaggedness::JaggednessPrune,
    pub(crate) line_family_prune: prune::line_family::LineFamilyPrune,
    pub(crate) parity_prune: prune::parity::ParityPrune,
    pub(crate) subset_prune: prune::subset::SubsetPrune,
    pub(crate) weight_tuple_prune: prune::weight_tuple::WeightTuplePrune,
    pub(crate) hit_count_threshold: std::sync::atomic::AtomicU8,
    pub(crate) hit_count_thresholds: Vec<u8>,
    pub(crate) suffix_coverage: Vec<CoverageCounter>,
    pub(crate) skip_tables: Vec<Option<Vec<bool>>>,
    pub(crate) single_cell_start: usize,
    pub(crate) m: u8,
    pub(crate) h: u8,
    pub(crate) w: u8,
    pub(crate) progress_weights: Vec<f64>,
}

/// Main entry point. Tries progressively looser hit-count thresholds
/// (p50, p75, p90, p95, max+1), reusing precomputed data across attempts.
pub fn solve(game: &Game, parallel: bool, exhaustive: bool) -> SolveResult {
    let config = PruningConfig::default();

    let (board, order, data) = prepare_solver(game, &config);
    let thresholds = data.hit_count_thresholds.clone();

    let mut total_nodes = 0u64;
    let mut last_progress = 0.0;
    for &threshold in &thresholds {
        data.hit_count_threshold.store(threshold, Ordering::Relaxed);
        let result = if parallel {
            run_parallel(&board, &order, &data, &config, exhaustive)
        } else {
            run_serial(&board, &order, &data, &config)
        };
        total_nodes += result.nodes_visited;
        last_progress = result.progress;
        if result.solution.is_some() {
            return SolveResult { nodes_visited: total_nodes, ..result };
        }
    }

    SolveResult {
        solution: None,
        nodes_visited: total_nodes,
        progress: last_progress,
    }
}

/// Build sorted placements, skip tables, and all precomputed pruning data.
fn prepare_solver(game: &Game, config: &PruningConfig) -> (crate::core::board::Board, Vec<usize>, SolverData) {
    let board = game.board().clone();
    let pieces = game.pieces();
    let h = board.height();
    let w = board.width();
    let n = pieces.len();

    let mut indexed: Vec<(usize, Vec<(usize, usize, Bitboard)>)> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.placements(h, w)))
        .collect();
    indexed.sort_by(|(i, a_pl), (j, b_pl)| {
        a_pl.len()
            .cmp(&b_pl.len())
            .then_with(|| pieces[*j].perimeter().cmp(&pieces[*i].perimeter()))
            .then_with(|| pieces[*j].cell_count().cmp(&pieces[*i].cell_count()))
            .then_with(|| pieces[*i].shape().limbs().cmp(&pieces[*j].shape().limbs()))
    });

    let order: Vec<usize> = indexed.iter().map(|(i, _)| *i).collect();
    let all_placements: Vec<Vec<(usize, usize, Bitboard)>> =
        indexed.into_iter().map(|(_, p)| p).collect();

    let skip_tables: Vec<Option<Vec<bool>>> = (0..n).map(|i| {
        if i == 0 { return None; }
        let prev_pl = &all_placements[i - 1];
        let curr_pl = &all_placements[i];
        let num_prev = prev_pl.len();
        let num_curr = curr_pl.len();
        let mut table = vec![false; num_prev * num_curr];
        let mut seen = std::collections::HashSet::new();
        let mut any_skips = false;
        for a in 0..num_prev {
            let mask_a = prev_pl[a].2;
            for b in 0..num_curr {
                let mask_b = curr_pl[b].2;
                let key = (mask_a & mask_b, mask_a ^ mask_b);
                if !seen.insert(key) {
                    table[a * num_curr + b] = true;
                    any_skips = true;
                }
            }
        }
        if any_skips { Some(table) } else { None }
    }).collect();

    let single_cell_start = (0..n)
        .rposition(|i| pieces[order[i]].cell_count() != 1)
        .map(|i| i + 1)
        .unwrap_or(0);

    let m = board.m();
    let data = precompute::build_solver_data(
        &board, pieces, &order, all_placements, skip_tables,
        single_cell_start, h, w, m,
    );

    (board, order, data)
}

/// Serial backtrack with pre-built data.
fn run_serial(
    board: &crate::core::board::Board,
    order: &[usize],
    data: &SolverData,
    config: &PruningConfig,
) -> SolveResult {
    let n = data.all_placements.len();
    let nodes = Cell::new(0u64);
    let mut sorted_solution = Vec::with_capacity(n);

    let found = backtrack::backtrack(
        board,
        prune::hit_count::HitCounter::new(),
        data,
        0,
        usize::MAX,
        &mut sorted_solution,
        &nodes,
        config,
    );

    let solution = if found {
        let mut solution = vec![(0, 0); n];
        for (sorted_idx, &(row, col)) in sorted_solution.iter().enumerate() {
            solution[order[sorted_idx]] = (row, col);
        }
        Some(solution)
    } else {
        None
    };

    SolveResult {
        solution,
        nodes_visited: nodes.get(),
        progress: 0.0,
    }
}

/// Kept for tests that call it directly.
pub fn solve_with_config(game: &Game, config: &PruningConfig) -> SolveResult {
    let (board, order, data) = prepare_solver(game, config);
    run_serial(&board, &order, &data, config)
}

/// Parallel backtrack with pre-built data.
fn run_parallel(
    board: &crate::core::board::Board,
    order: &[usize],
    data: &SolverData,
    config: &PruningConfig,
    exhaustive: bool,
) -> SolveResult {
    let n = data.all_placements.len();

    let wq = backtrack::WorkQueue::new();
    wq.push(backtrack::StealableTask {
        board: board.clone(),
        hits: prune::hit_count::HitCounter::new(),
        prefix: Vec::new(),
        depth: 0,
        prev_placement: usize::MAX,
    });

    let num_threads = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);

    let abort = AtomicBool::new(false);
    let result: Mutex<Option<Vec<(usize, usize)>>> = Mutex::new(None);
    let total_nodes = std::sync::atomic::AtomicU64::new(0);
    let active_count = std::sync::atomic::AtomicUsize::new(0);
    let idle_count = std::sync::atomic::AtomicUsize::new(0);
    let progress = std::sync::atomic::AtomicU64::new(0f64.to_bits());
    let workers_alive = std::sync::atomic::AtomicUsize::new(num_threads);

    let total_space: f64 = data.all_placements.iter()
        .map(|p| p.len() as f64)
        .product();
    eprintln!("search space: {:.3e}", total_space);

    let solve_start = std::time::Instant::now();

    std::thread::scope(|s| {
        s.spawn(|| {
            let bar_width = 30;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if abort.load(Ordering::Relaxed)
                    || workers_alive.load(Ordering::Relaxed) == 0 { break; }

                let p = f64::from_bits(progress.load(Ordering::Relaxed)).min(1.0);
                let pct = p * 100.0;
                let nodes_so_far = total_nodes.load(Ordering::Relaxed);
                let elapsed = solve_start.elapsed().as_secs_f64();

                let filled = (p * bar_width as f64) as usize;
                let bar: String = (0..bar_width).map(|i| if i < filled { '#' } else { ' ' }).collect();

                let nodes_str = format_count(nodes_so_far);
                eprint!("\r\x1b[K[{}] {:.1}%  {}  {:.1}s", bar, pct, nodes_str, elapsed);
            }
            eprint!("\r\x1b[K");
        });

        for _ in 0..num_threads {
            s.spawn(|| {
                let nodes = Cell::new(0u64);
                let mut solution = Vec::with_capacity(n);

                loop {
                    if abort.load(Ordering::Relaxed) { break; }

                    let task = wq.pop().or_else(|| {
                        idle_count.fetch_add(1, Ordering::Relaxed);
                        let t = wq.wait_for_task(&abort, &active_count);
                        idle_count.fetch_sub(1, Ordering::Relaxed);
                        t
                    });
                    let task = match task {
                        Some(t) => t,
                        None => break,
                    };
                    active_count.fetch_add(1, Ordering::SeqCst);

                    solution.clear();
                    solution.extend_from_slice(&task.prefix);
                    nodes.set(0);

                    let found = backtrack::backtrack_stealing(
                        &task.board,
                        task.hits,
                        data,
                        task.depth,
                        task.prev_placement,
                        &mut solution,
                        &nodes,
                        config,
                        &abort,
                        &wq,
                        &idle_count,
                        exhaustive,
                        &progress,
                    );

                    active_count.fetch_sub(1, Ordering::SeqCst);
                    total_nodes.fetch_add(nodes.get(), Ordering::Relaxed);

                    if found {
                        if !exhaustive {
                            abort.store(true, Ordering::Relaxed);
                        }
                        let mut guard = result.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some(solution.clone());
                        }
                    }
                }
                workers_alive.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });

    let result = result.into_inner().unwrap();
    let nodes_visited = total_nodes.load(Ordering::Relaxed);

    let solution = result.map(|sorted_solution| {
        let mut solution = vec![(0, 0); n];
        for (sorted_idx, &(row, col)) in sorted_solution.iter().enumerate() {
            solution[order[sorted_idx]] = (row, col);
        }
        solution
    });

    let final_progress = f64::from_bits(progress.load(Ordering::Relaxed));

    SolveResult {
        solution,
        nodes_visited,
        progress: final_progress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::game::Game;
    use crate::core::piece::Piece;

    fn verify_solution(game: &Game, solution: &Solution) {
        let mut board = game.board().clone();
        for (i, &(row, col)) in solution.iter().enumerate() {
            let mask = game.pieces()[i].placed_at(row, col);
            board.apply_piece(mask);
        }
        assert!(board.is_solved(), "solution did not solve the board");
    }

    #[test]
    fn test_trivial_solve() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        let sol = solve(&game, false, false).solution.unwrap();
        assert_eq!(sol.len(), 1);
        assert_eq!(sol[0], (0, 0));
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_two_pieces() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece, piece]);
        let sol = solve(&game, false, false).solution.unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_no_solution() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn test_all_single_cells() {
        // 3x3, m=2. Board all 1s. Nine 1x1 pieces.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 9]);
        let sol = solve(&game, false, false).solution.unwrap();
        assert_eq!(sol.len(), 9);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_single_cells_m3() {
        // 3x3, m=3. Cell (0,0)=1 needs 2 hits, cell (0,1)=2 needs 1 hit. 3 pieces total.
        let grid: &[&[u8]] = &[&[1, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 3]);
        let sol = solve(&game, false, false).solution.unwrap();
        assert_eq!(sol.len(), 3);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_single_cells_insufficient() {
        // 3x3, m=2. Two 1s but only one piece.
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn test_mixed_then_single() {
        // Mix of multi-cell and single-cell pieces.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let big = Piece::from_grid(&[&[true, true], &[true, false]]); // L-shape, 3 cells
        let small = Piece::from_grid(&[&[true]]); // 1x1
        let game = Game::new(board, vec![big, small]);
        let sol = solve(&game, false, false).solution.unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_generated_game_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
        let game = crate::generate::generate_for_level(1, &mut rng).unwrap();
        let sol = solve(&game, false, false).solution.unwrap();
        assert_eq!(sol.len(), game.pieces().len());
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_generated_level_5_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(123);
        let game = crate::generate::generate_for_level(5, &mut rng).unwrap();
        let sol = solve(&game, false, false).solution.unwrap();
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_total_deficit_pruning() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 9);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn test_solution_maps_to_original_order() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let p0 = Piece::from_grid(&[&[true]]);
        let p1 = Piece::from_grid(&[&[true, true]]);
        let game = Game::new(board, vec![p0, p1]);
        let sol = solve(&game, false, false).solution.unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_coverage_pruning_unreachable() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true], &[true], &[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn test_generated_levels_solvable() {
        for level in [1, 5, 10, 20, 25, 30] {
            let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
            let game = crate::generate::generate_for_level(level, &mut rng).unwrap();
            let result = solve(&game, false, false);
            assert!(result.solution.is_some(), "level {level} should be solvable");
            verify_solution(&game, &result.solution.unwrap());
        }
    }

    /// Fuzz test: generate many random games across a variety of board sizes, M values,
    /// and piece counts. Every generated game is guaranteed solvable by construction.
    /// Verify the solver finds a valid solution for each.
    #[test]
    fn test_fuzz_soundness() {
        use rayon::prelude::*;
        use crate::generate::generate_game;
        use crate::level::LevelSpec;

        // Test configurations: (M, rows, cols, num_pieces)
        let configs: Vec<(u8, u8, u8, u8)> = vec![
            // Small boards
            (2, 3, 3, 4), (2, 3, 3, 8),
            (3, 3, 3, 3), (3, 3, 3, 7),
            // Medium boards
            (2, 4, 3, 5), (2, 4, 3, 8),
            (2, 4, 4, 6), (2, 4, 4, 10),
            (3, 4, 3, 6), (3, 4, 4, 8),
            (4, 4, 4, 6), (4, 4, 4, 10),
            // Larger boards (fewer configs, lower piece counts)
            (2, 6, 6, 8), (3, 6, 6, 8), (4, 6, 6, 8), (5, 6, 6, 6),
        ];

        let seeds: Vec<u64> = (0..5).collect();

        let failures: Vec<String> = configs
            .par_iter()
            .flat_map(|&(m, rows, cols, shapes)| {
                let spec = LevelSpec {
                    level: 0,
                    shifts: m,
                    rows,
                    columns: cols,
                    shapes,
                };
                seeds.par_iter().filter_map(move |&seed| {
                    let mut rng =
                        <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                    let game = generate_game(&spec, &mut rng);
                    let result = solve(&game, false, false);
                    match result.solution {
                        None => Some(format!(
                            "FAIL: no solution found for M={} {}x{} pieces={} seed={}",
                            m, rows, cols, shapes, seed
                        )),
                        Some(ref s) => {
                            // Verify the solution is correct.
                            let mut board = game.board().clone();
                            for (i, &(row, col)) in s.iter().enumerate() {
                                let mask = game.pieces()[i].placed_at(row, col);
                                board.apply_piece(mask);
                            }
                            if !board.is_solved() {
                                Some(format!(
                                    "FAIL: invalid solution for M={} {}x{} pieces={} seed={}",
                                    m, rows, cols, shapes, seed
                                ))
                            } else {
                                None
                            }
                        }
                    }
                }).collect::<Vec<_>>()
            })
            .collect();

        if !failures.is_empty() {
            for f in &failures[..failures.len().min(20)] {
                eprintln!("{}", f);
            }
            panic!("{} fuzz test failures (showing first 20)", failures.len());
        }
    }

    // --- Per-prune effectiveness and soundness tests ---

    /// Helper: generate games from a set of configs, solve with given pruning config,
    /// verify soundness, return total nodes visited.
    fn fuzz_with_config(
        config: &PruningConfig,
        configs: &[(u8, u8, u8, u8)],
        seeds: &[u64],
    ) -> (u64, usize) {
        use crate::generate::generate_game;
        use crate::level::LevelSpec;

        let mut total_nodes = 0u64;
        let mut failures = 0usize;
        for &(m, rows, cols, shapes) in configs {
            let spec = LevelSpec {
                level: 0, shifts: m, rows, columns: cols, shapes,
            };
            for &seed in seeds {
                let mut rng =
                    <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = generate_game(&spec, &mut rng);
                let result = solve_with_config(&game, config);
                total_nodes += result.nodes_visited;
                match &result.solution {
                    None => failures += 1,
                    Some(s) => {
                        let mut board = game.board().clone();
                        for (i, &(row, col)) in s.iter().enumerate() {
                            let mask = game.pieces()[i].placed_at(row, col);
                            board.apply_piece(mask);
                        }
                        if !board.is_solved() {
                            failures += 1;
                        }
                    }
                }
            }
        }
        (total_nodes, failures)
    }

    /// Small configs suitable for brute-force comparison.
    fn small_configs() -> Vec<(u8, u8, u8, u8)> {
        vec![
            (2, 3, 3, 4), (2, 3, 3, 8),
            (3, 3, 3, 3), (3, 3, 3, 5),
            (2, 4, 3, 5), (2, 4, 4, 6),
            (3, 4, 3, 6), (3, 4, 4, 8),
            (4, 3, 3, 3), (4, 4, 3, 4),
        ]
    }

    fn test_seeds() -> Vec<u64> {
        (0..5).collect()
    }

    #[test]
    fn test_prune_active_planes() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.active_planes = true);

        let (nodes_without, fail_without) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "active_planes prune caused failures");
        assert!(nodes_with <= nodes_without,
            "active_planes should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_prune_total_deficit_global() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.total_deficit_global = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "total_deficit_global prune caused failures");
        assert!(nodes_with <= nodes_without,
            "total_deficit_global should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_prune_total_deficit_rowcol() {
        let configs = small_configs();
        let seeds = test_seeds();
        // Enable global so the rowcol check has something to build on.
        let baseline = PruningConfig::none().only(|c| c.total_deficit_global = true);
        let with_prune = PruningConfig::none().only(|c| {
            c.total_deficit_global = true;
            c.total_deficit_rowcol = true;
        });

        let (nodes_baseline, _) = fuzz_with_config(&baseline, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "total_deficit_rowcol prune caused failures");
        assert!(nodes_with <= nodes_baseline,
            "total_deficit_rowcol should reduce nodes: {} vs {}", nodes_with, nodes_baseline);
    }

    #[test]
    fn test_prune_total_deficit_diagonal() {
        let configs = small_configs();
        let seeds = test_seeds();
        let baseline = PruningConfig::none().only(|c| c.total_deficit_global = true);
        let with_prune = PruningConfig::none().only(|c| {
            c.total_deficit_global = true;
            c.total_deficit_diagonal = true;
        });

        let (nodes_baseline, _) = fuzz_with_config(&baseline, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "total_deficit_diagonal prune caused failures");
        assert!(nodes_with <= nodes_baseline,
            "total_deficit_diagonal should reduce nodes: {} vs {}", nodes_with, nodes_baseline);
    }

    #[test]
    fn test_prune_total_deficit_rowcol_soundness_stress() {
        let configs = vec![
            (2, 4, 4, 10), (2, 4, 4, 14),
            (3, 4, 4, 8), (3, 4, 4, 12),
            (2, 6, 6, 8), (2, 6, 6, 12),
            (3, 6, 6, 8), (4, 6, 6, 8),
        ];
        let seeds: Vec<u64> = (0..5).collect();
        let config = PruningConfig::none().only(|c| {
            c.total_deficit_global = true;
            c.total_deficit_rowcol = true;
        });
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "total_deficit_rowcol stress test had {} failures", failures);
    }

    #[test]
    fn test_prune_total_deficit_diagonal_soundness_stress() {
        let configs = vec![
            (2, 4, 4, 10), (2, 4, 4, 14),
            (3, 4, 4, 8), (3, 4, 4, 12),
            (2, 6, 6, 8), (2, 6, 6, 12),
            (3, 6, 6, 8), (4, 6, 6, 8),
        ];
        let seeds: Vec<u64> = (0..5).collect();
        let config = PruningConfig::none().only(|c| {
            c.total_deficit_global = true;
            c.total_deficit_diagonal = true;
        });
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "total_deficit_diagonal stress test had {} failures", failures);
    }

    #[test]
    fn test_prune_coverage() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.coverage = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "coverage prune caused failures");
        assert!(nodes_with <= nodes_without,
            "coverage should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_prune_jaggedness() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.jaggedness = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "jaggedness prune caused failures");
        assert!(nodes_with <= nodes_without,
            "jaggedness should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_prune_cell_locking() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.cell_locking = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "cell_locking prune caused failures");
        assert!(nodes_with <= nodes_without,
            "cell_locking should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_prune_single_cell_endgame() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.single_cell_endgame = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "single_cell_endgame caused failures");
        assert!(nodes_with <= nodes_without,
            "single_cell_endgame should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_all_prunes_sound() {
        // Full config should solve everything the no-prune config solves.
        let configs = small_configs();
        let seeds = test_seeds();
        let (_, fail_all) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        assert_eq!(fail_all, 0, "all prunes combined caused failures");
    }

    #[test]
    fn test_parity_partition_soundness() {
        let configs = vec![
            (2, 3, 3, 4), (2, 3, 3, 6), (2, 3, 3, 8),
            (3, 3, 3, 5), (3, 3, 3, 7),
            (2, 4, 3, 5), (2, 4, 3, 8),
            (2, 4, 4, 6), (2, 4, 4, 10),
            (3, 4, 4, 8), (3, 4, 4, 12),
            (4, 4, 4, 6), (4, 4, 4, 10),
        ];
        let seeds = test_seeds();
        let (_, failures) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        assert_eq!(failures, 0, "parity partition caused {} failures", failures);
    }

    #[test]
    fn test_parity_partition_soundness_stress() {
        let configs = vec![
            (2, 6, 6, 8), (2, 6, 6, 12),
            (3, 6, 6, 8), (4, 6, 6, 8),
            (5, 6, 6, 6),
        ];
        let seeds: Vec<u64> = (0..5).collect();
        let (_, failures) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        assert_eq!(failures, 0, "parity partition stress test had {} failures", failures);
    }

    #[test]
    fn test_parity_partition_effectiveness() {
        let configs = small_configs();
        let seeds = test_seeds();
        let (nodes_with, _) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        let mut no_parity = PruningConfig::default();
        no_parity.total_deficit_global = false;
        let (nodes_without, _) = fuzz_with_config(&no_parity, &configs, &seeds);
        assert!(nodes_with <= nodes_without,
            "parity partitions should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_pair_skip_tables_non_identical() {
        let configs = vec![
            (2, 3, 3, 4), (2, 3, 3, 6), (2, 3, 3, 8),
            (2, 4, 3, 5), (2, 4, 3, 8),
            (2, 4, 4, 6), (2, 4, 4, 10),
            (3, 4, 4, 8), (3, 4, 4, 12),
        ];
        let seeds: Vec<u64> = (0..5).collect();

        let (_, failures) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        assert_eq!(failures, 0, "pair skip tables caused {} failures", failures);
    }

    #[test]
    fn test_pair_skip_tables_soundness_stress() {
        let configs = vec![
            (2, 4, 4, 10), (2, 4, 4, 14),
            (3, 4, 4, 8), (3, 4, 4, 12),
            (2, 6, 6, 8), (2, 6, 6, 12),
            (3, 6, 6, 8), (4, 6, 6, 8),
        ];
        let seeds: Vec<u64> = (0..5).collect();

        let (_, failures) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        assert_eq!(failures, 0, "pair skip stress test had {} failures", failures);
    }

    // --- Pair-merge reduction tests ---

    #[test]
    fn test_pair_merge_basic() {
        let p1x2 = Piece::from_grid(&[&[true, true]]);
        let p1x1 = Piece::from_grid(&[&[true]]);

        let h = 3u8;
        let w = 3u8;
        let pl_a = p1x2.placements(h, w);
        let pl_b = p1x1.placements(h, w);
        let mut cells = std::collections::HashSet::new();
        for &(_, _, ma) in &pl_a {
            for &(_, _, mb) in &pl_b {
                let xor = ma ^ mb;
                if xor.count_ones() == 1 {
                    cells.insert(xor.lowest_set_bit());
                }
            }
        }
        assert_eq!(cells.len(), 9, "1x2 + 1x1 should produce all 9 cells on 3x3");
    }

    #[test]
    fn test_pair_merge_soundness() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let p1x2 = Piece::from_grid(&[&[true, true]]);
        let p1x1 = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![p1x2, p1x1, p1x1, p1x1]);
        let result = solve(&game, false, false);
        assert!(result.solution.is_some(), "pair-merge game should solve");
        verify_solution(&game, result.solution.as_ref().unwrap());
    }

    #[test]
    fn test_pair_merge_soundness_stress() {
        use rand::SeedableRng;

        let configs = vec![
            (2, 4, 4, 10), (2, 4, 4, 14), (2, 6, 6, 12),
        ];

        for &(m, h, w, n) in &configs {
            for seed in 0..10u64 {
                let spec = crate::level::LevelSpec {
                    level: 99, shifts: m, rows: h, columns: w, shapes: n,
                };
                let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = crate::generate::generate_game(&spec, &mut rng);
                let result = solve(&game, false, false);
                if let Some(ref sol) = result.solution {
                    verify_solution(&game, sol);
                }
            }
        }
    }

    #[test]
    fn test_pair_merge_no_false_positive() {
        let grid: &[&[u8]] = &[
            &[1, 1, 0],
            &[1, 0, 0],
            &[0, 0, 0],
        ];
        let board = Board::from_grid(grid, 2);
        let p_l = Piece::from_grid(&[&[true, true], &[true, false]]);
        let p1x1 = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![p_l, p1x1, p1x1]);
        let result = solve(&game, false, false);
        assert!(result.solution.is_some());
        verify_solution(&game, result.solution.as_ref().unwrap());
    }

    // --- Subset reachability no-false-zero-effect test ---

    #[test]
    fn test_subset_no_false_zero_effect() {
        use rand::SeedableRng;

        let configs = vec![
            (2, 4, 4, 10), (3, 4, 4, 8), (2, 6, 6, 12),
        ];
        for &(m, h, w, n) in &configs {
            for seed in 0..20u64 {
                let spec = crate::level::LevelSpec {
                    level: 99, shifts: m, rows: h, columns: w, shapes: n,
                };
                let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = crate::generate::generate_game(&spec, &mut rng);
                let result = solve(&game, false, false);
                if let Some(ref sol) = result.solution {
                    verify_solution(&game, sol);
                }
            }
        }
    }

    // --- Cancellation reduction test ---

    #[test]
    fn test_cancellation_reduction() {
        let board = Board::new_solved(3, 3, 2);
        let p = Piece::from_grid(&[&[true, true], &[true, false]]);
        let game = Game::new(board, vec![p, p, p, p]);
        let result = solve(&game, false, false);
        assert!(result.solution.is_some(), "4 identical pieces on solved board should cancel");
        verify_solution(&game, result.solution.as_ref().unwrap());
    }

    // --- Progress indicator tests ---
    // In exhaustive mode, the parallel solver must explore the entire naive
    // search space, so progress should sum to exactly 1.0.

    fn assert_progress_complete(result: &SolveResult, label: &str) {
        let p = result.progress;
        assert!(
            (p - 1.0).abs() < 1e-9,
            "{}: expected progress ≈ 1.0, got {:.15} (diff={:.2e})",
            label, p, (p - 1.0).abs()
        );
    }

    #[test]
    fn test_progress_exhaustive_trivial_1_piece() {
        // 3x3, M=2, one 1x1 piece on a board with cell (0,0)=1.
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "trivial_1_piece");
    }

    #[test]
    fn test_progress_exhaustive_two_pieces() {
        // 3x3, M=2, two 1x1 pieces.
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece, piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "two_pieces");
    }

    #[test]
    fn test_progress_exhaustive_no_solution() {
        // No solution: 3x3, M=3, one 1x1 piece but two cells need hits.
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_none());
        assert_progress_complete(&result, "no_solution");
    }

    #[test]
    fn test_progress_exhaustive_multi_cell_pieces() {
        // 3x3, M=2, mix of multi-cell pieces.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let big = Piece::from_grid(&[&[true, true], &[true, false]]);
        let small = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![big, small]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "multi_cell_pieces");
    }

    #[test]
    fn test_progress_exhaustive_m3() {
        // 3x3, M=3, three 1x1 pieces: cell (0,0)=1 needs 2 hits, (0,1)=2 needs 1 hit.
        let grid: &[&[u8]] = &[&[1, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 3]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "m3");
    }

    #[test]
    fn test_progress_exhaustive_generated_levels() {
        // Test on several generated puzzles to cover diverse piece shapes.
        use crate::generate::generate_for_level;
        for (level, seed) in [(1, 42u64), (2, 99), (3, 7), (5, 123)] {
            let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
            let game = generate_for_level(level, &mut rng).unwrap();
            let result = solve(&game, true, true);
            assert!(result.solution.is_some(), "level {} seed {} unsolved", level, seed);
            assert_progress_complete(&result, &format!("generated_level_{}_seed_{}", level, seed));
        }
    }

    #[test]
    fn test_progress_exhaustive_all_single_cells() {
        // 3x3, M=2, nine 1x1 pieces on all-1s board (single-cell endgame path).
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 9]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "all_single_cells");
    }

    #[test]
    fn test_progress_exhaustive_duplicate_pieces() {
        // Duplicate pieces trigger skip tables; verify progress still sums to 1.0.
        let grid: &[&[u8]] = &[&[1, 1, 0], &[1, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true, true]]);
        let game = Game::new(board, vec![piece, piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "duplicate_pieces");
    }

}
