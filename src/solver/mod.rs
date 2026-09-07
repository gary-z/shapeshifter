mod adaptive;
mod backtrack;
mod likelihood;
mod parallel;
mod precompute;
mod pruning;
mod regional;
pub(crate) mod runtime;

use runtime::{Duration, Instant};

use std::cell::Cell;

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::game::Game;

fn format_count(count: u64) -> String {
    if count >= 1_000_000_000 {
        format!("{:.1}B nodes", count as f64 / 1e9)
    } else if count >= 1_000_000 {
        format!("{:.1}M nodes", count as f64 / 1e6)
    } else if count >= 1_000 {
        format!("{:.1}K nodes", count as f64 / 1e3)
    } else {
        format!("{} nodes", count)
    }
}

/// A list of (row, column) placements in the puzzle's original piece order.
pub type Solution = Vec<(usize, usize)>;
type Placement = (usize, usize, Bitboard);
type PiecePlacements = Vec<Placement>;

pub struct SolveResult {
    pub solution: Option<Solution>,
    /// Backtracking states plus complete assignments evaluated by inference.
    pub nodes_visited: u64,
    /// Final fraction (0.0–1.0) of the naive search space accounted for.
    /// Only meaningful for parallel backtracking; 0.0 for the other searches.
    pub progress: f64,
}

/// A game's placement tables and pruning bounds, ready for search.
///
/// Construct this with [`prepare`] to measure preparation and search separately.
pub struct PreparedSearch {
    board: Board,
    piece_order: Vec<usize>,
    data: SolverData,
    parallel: bool,
    exhaustive: bool,
    guided_frontier: bool,
    adaptive: Option<adaptive::AdaptiveSearch>,
    regional: Option<regional::RegionalSearch>,
}

fn should_use_guided_frontier(game: &Game, parallel: bool, exhaustive: bool) -> bool {
    parallel && !exhaustive && likelihood::ReverseLikelihood::can_precompute(game.board().m())
}

struct SolverData {
    placements: Vec<PiecePlacements>,
    total_deficit: pruning::TotalDeficitBound,
    jaggedness: pruning::JaggednessBound,
    partition_reachability: pruning::PartitionReachability,
    small_component: pruning::SmallComponentBound,
    cell_set_bound: pruning::CellSetBound,
    anchor_placements: Vec<backtrack::AnchorPlacementData>,
    reverse_likelihood: Option<likelihood::ReverseLikelihood>,
    equivalent_pair_skips: Vec<Option<Vec<bool>>>,
    single_cell_suffix_start: usize,
    modulus: u8,
    height: u8,
    width: u8,
    progress_weights: Vec<f64>,
}

/// Solve a game, optionally using all available CPU cores.
///
/// `exhaustive` keeps searching after the first solution and is primarily used
/// to benchmark the bounded search tree.
pub fn solve(game: &Game, parallel: bool, exhaustive: bool) -> SolveResult {
    prepare(game, parallel, exhaustive).solve()
}

/// Build placement tables and bounds without exploring the search tree.
pub fn prepare(game: &Game, parallel: bool, exhaustive: bool) -> PreparedSearch {
    let guided_frontier = should_use_guided_frontier(game, parallel, exhaustive);

    let (board, piece_order, data) = prepare_search(game, guided_frontier);
    let adaptive = (parallel && !exhaustive).then(|| adaptive::AdaptiveSearch::precompute(game));
    let regional = (parallel && !exhaustive).then(|| regional::RegionalSearch::precompute(game));
    PreparedSearch {
        board,
        piece_order,
        data,
        parallel,
        exhaustive,
        guided_frontier,
        adaptive,
        regional,
    }
}

impl PreparedSearch {
    /// Search using the prepared tables. Guided frontier construction is search
    /// work and is deliberately included here, rather than in preparation.
    pub fn solve(&self) -> SolveResult {
        self.solve_until(None)
    }

    #[cfg(feature = "wasm")]
    pub(crate) fn solve_with_budget(&self, budget: Duration) -> SolveResult {
        self.solve_until(Some(Instant::now() + budget))
    }

    fn backtrack(&self, frontier_width: Option<usize>, deadline: Option<Instant>) -> SolveResult {
        macro_rules! dispatch {
            ($m:literal) => {
                if self.parallel {
                    parallel::solve_parallel::<$m>(
                        &self.board,
                        &self.piece_order,
                        &self.data,
                        self.exhaustive,
                        frontier_width,
                        deadline,
                    )
                } else {
                    solve_serial(&self.board, &self.piece_order, &self.data, self.exhaustive)
                }
            };
        }
        match self.data.modulus {
            2 => dispatch!(2),
            3 => dispatch!(3),
            4 => dispatch!(4),
            5 => dispatch!(5),
            _ => unreachable!(),
        }
    }

    fn solve_until(&self, overall_deadline: Option<Instant>) -> SolveResult {
        let phase_deadline = |seconds| {
            let limit = Instant::now() + Duration::from_secs(seconds);
            overall_deadline.map_or(limit, |overall| overall.min(limit))
        };
        let stopped = || {
            runtime::cancelled() || overall_deadline.is_some_and(|limit| Instant::now() >= limit)
        };
        let Self {
            board,
            data,
            parallel,
            exhaustive,
            guided_frontier,
            adaptive,
            regional,
            ..
        } = self;
        let (parallel, exhaustive, guided_frontier) = (*parallel, *exhaustive, *guided_frontier);

        let root_feasible = match data.modulus {
            2 => pruning::state_is_feasible::<2>(board, data, 0),
            3 => pruning::state_is_feasible::<3>(board, data, 0),
            4 => pruning::state_is_feasible::<4>(board, data, 0),
            5 => pruning::state_is_feasible::<5>(board, data, 0),
            _ => unreachable!(),
        };
        if !root_feasible || stopped() {
            return SolveResult {
                solution: None,
                nodes_visited: 0,
                progress: if parallel && !root_feasible { 1.0 } else { 0.0 },
            };
        }
        let mut total_nodes = 0u64;
        if adaptive.is_some() || regional.is_some() {
            // Preserve quick backtracking wins before exploring different trees.
            let result = self.backtrack(None, Some(phase_deadline(5)));
            total_nodes += result.nodes_visited;
            if result.solution.is_some() {
                return SolveResult {
                    nodes_visited: total_nodes,
                    ..result
                };
            }
        }
        if guided_frontier && !stopped() {
            // Build a narrow ranked frontier early so larger trees can reach
            // backtracking within this short attempt. Keep the full beam in fallback.
            let result = self.backtrack(Some(20_000), Some(phase_deadline(2)));
            total_nodes += result.nodes_visited;
            if result.solution.is_some() {
                return SolveResult {
                    nodes_visited: total_nodes,
                    ..result
                };
            }
        }
        if let Some(adaptive) = adaptive.as_ref().filter(|_| !stopped()) {
            let workers = runtime::workers();
            eprintln!("adaptive search: {workers} workers, 25s budget");
            let (solution, nodes) = adaptive.solve(
                phase_deadline(25).saturating_duration_since(Instant::now()),
                workers,
            );
            total_nodes += nodes;
            if solution.is_some() {
                return SolveResult {
                    solution,
                    nodes_visited: total_nodes,
                    progress: 0.0,
                };
            }
        }
        if let Some(regional) = regional.as_ref().filter(|_| !stopped()) {
            let workers = runtime::workers();
            eprintln!("regional search: {workers} workers, 25s budget");
            let (solution, nodes) = regional.solve(
                phase_deadline(25).saturating_duration_since(Instant::now()),
                workers,
            );
            total_nodes += nodes;
            if solution.is_some() {
                return SolveResult {
                    solution,
                    nodes_visited: total_nodes,
                    progress: 0.0,
                };
            }
        }
        let mut last_progress = 0.0;
        let mut first_solution: Option<Solution> = None;
        let attempt_count = 1 + usize::from(guided_frontier);
        for attempt_index in 0..attempt_count {
            if stopped() {
                break;
            }
            let is_guided_attempt = guided_frontier && attempt_index == 0;
            let result = self.backtrack(
                is_guided_attempt.then_some(parallel::GUIDED_FRONTIER_WIDTH),
                // Leave time for root backtracking when the ranked prefix
                // does not lead to a solution.
                if is_guided_attempt {
                    Some(phase_deadline(30))
                } else {
                    overall_deadline
                },
            );
            total_nodes += result.nodes_visited;
            last_progress = result.progress;
            if result.solution.is_some() {
                if !exhaustive {
                    return SolveResult {
                        nodes_visited: total_nodes,
                        ..result
                    };
                }
                if first_solution.is_none() {
                    first_solution = result.solution;
                }
            }
        }

        SolveResult {
            solution: first_solution,
            nodes_visited: total_nodes,
            progress: last_progress,
        }
    }
}

fn prepare_search(game: &Game, guided_frontier: bool) -> (Board, Vec<usize>, SolverData) {
    let board = *game.board();
    let pieces = game.pieces();
    let height = board.height();
    let width = board.width();

    let mut ordered_placements: Vec<(usize, PiecePlacements)> = pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| (index, piece.placements(height, width)))
        .collect();
    ordered_placements.sort_by(|(i, a_placements), (j, b_placements)| {
        a_placements
            .len()
            .cmp(&b_placements.len())
            .then_with(|| pieces[*j].perimeter().cmp(&pieces[*i].perimeter()))
            .then_with(|| pieces[*j].cell_count().cmp(&pieces[*i].cell_count()))
            .then_with(|| pieces[*i].shape().limbs().cmp(&pieces[*j].shape().limbs()))
    });

    let piece_order: Vec<usize> = ordered_placements
        .iter()
        .map(|(original_index, _)| *original_index)
        .collect();
    let placements: Vec<PiecePlacements> = ordered_placements
        .into_iter()
        .map(|(_, placements)| placements)
        .collect();
    let equivalent_pair_skips = build_equivalent_pair_skips(&placements);

    let single_cell_suffix_start = (0..pieces.len())
        .rposition(|index| pieces[piece_order[index]].cell_count() != 1)
        .map(|index| index + 1)
        .unwrap_or(0);

    let data = precompute::build_solver_data(
        &board,
        pieces,
        &piece_order,
        placements,
        equivalent_pair_skips,
        single_cell_suffix_start,
        guided_frontier,
    );

    (board, piece_order, data)
}

fn build_equivalent_pair_skips(placements: &[PiecePlacements]) -> Vec<Option<Vec<bool>>> {
    (0..placements.len())
        .map(|piece_index| {
            if piece_index == 0 {
                return None;
            }

            let previous = &placements[piece_index - 1];
            let current = &placements[piece_index];
            let mut skips = vec![false; previous.len() * current.len()];
            let mut seen_effects = std::collections::HashSet::new();
            let mut has_skips = false;

            for (previous_index, &(_, _, previous_mask)) in previous.iter().enumerate() {
                for (current_index, &(_, _, current_mask)) in current.iter().enumerate() {
                    let combined_effect =
                        (previous_mask & current_mask, previous_mask ^ current_mask);
                    if !seen_effects.insert(combined_effect) {
                        skips[previous_index * current.len() + current_index] = true;
                        has_skips = true;
                    }
                }
            }

            has_skips.then_some(skips)
        })
        .collect()
}

fn backtrack_for_modulus(
    board: &Board,
    data: &SolverData,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    exhaustive: bool,
) -> bool {
    macro_rules! go {
        ($m:literal) => {
            backtrack::backtrack::<$m>(
                backtrack::SearchPosition {
                    board: *board,
                    piece_index: 0,
                    previous_placement: usize::MAX,
                },
                data,
                solution,
                nodes,
                exhaustive,
            )
        };
    }
    match data.modulus {
        2 => go!(2),
        3 => go!(3),
        4 => go!(4),
        5 => go!(5),
        _ => unreachable!("M must be 2..=5"),
    }
}

fn solve_serial(
    board: &Board,
    piece_order: &[usize],
    data: &SolverData,
    exhaustive: bool,
) -> SolveResult {
    let piece_count = data.placements.len();
    let nodes = Cell::new(0u64);
    let mut sorted_solution = Vec::with_capacity(piece_count);

    let found = backtrack_for_modulus(board, data, &mut sorted_solution, &nodes, exhaustive);

    let solution = if found {
        Some(restore_piece_order(&sorted_solution, piece_order))
    } else {
        None
    };

    SolveResult {
        solution,
        nodes_visited: nodes.get(),
        progress: 0.0,
    }
}

fn restore_piece_order(sorted_solution: &[(usize, usize)], piece_order: &[usize]) -> Solution {
    let mut solution = vec![(0, 0); sorted_solution.len()];
    for (sorted_index, &placement) in sorted_solution.iter().enumerate() {
        solution[piece_order[sorted_index]] = placement;
    }
    solution
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;
    use crate::game::Game;

    fn verify_solution(game: &Game, solution: &Solution) {
        let mut board = *game.board();
        for (piece_index, &(row, column)) in solution.iter().enumerate() {
            let mask = game.pieces()[piece_index].placed_at(row, column);
            board.apply_piece(mask);
        }
        assert!(board.is_solved(), "solution did not solve the board");
    }

    #[test]
    fn parallel_search_respects_deadline() {
        use std::time::{Duration, Instant};

        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(50_101);
        let game = crate::generate::generate_for_level(50, &mut rng).unwrap();
        let (board, piece_order, data) = prepare_search(&game, true);
        for guided in [false, true] {
            let start = Instant::now();
            let result = parallel::solve_parallel::<3>(
                &board,
                &piece_order,
                &data,
                false,
                guided.then_some(parallel::GUIDED_FRONTIER_WIDTH),
                Some(start + Duration::from_millis(10)),
            );
            assert!(start.elapsed() < Duration::from_secs(5));
            if let Some(solution) = result.solution {
                verify_solution(&game, &solution);
            }
        }
    }

    #[test]
    fn trivial_solve() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        let solution = solve(&game, false, false).solution.unwrap();
        assert_eq!(solution.len(), 1);
        assert_eq!(solution[0], (0, 0));
        verify_solution(&game, &solution);
    }

    #[test]
    fn two_pieces() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece, piece]);
        let solution = solve(&game, false, false).solution.unwrap();
        assert_eq!(solution.len(), 2);
        verify_solution(&game, &solution);
    }

    #[test]
    fn no_solution() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn solves_single_cell_suffix() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 9]);
        let solution = solve(&game, false, false).solution.unwrap();
        assert_eq!(solution.len(), 9);
        verify_solution(&game, &solution);
    }

    #[test]
    fn solves_single_cell_suffix_with_modulus_three() {
        let grid: &[&[u8]] = &[&[1, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 3]);
        let solution = solve(&game, false, false).solution.unwrap();
        assert_eq!(solution.len(), 3);
        verify_solution(&game, &solution);
    }

    #[test]
    fn rejects_insufficient_single_cell_suffix() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn solves_multi_cell_then_single_cell_piece() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let l_shape = Piece::from_grid(&[&[true, true], &[true, false]]);
        let single_cell = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![l_shape, single_cell]);
        let solution = solve(&game, false, false).solution.unwrap();
        assert_eq!(solution.len(), 2);
        verify_solution(&game, &solution);
    }

    #[test]
    fn generated_game_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
        let game = crate::generate::generate_for_level(1, &mut rng).unwrap();
        let solution = solve(&game, false, false).solution.unwrap();
        assert_eq!(solution.len(), game.pieces().len());
        verify_solution(&game, &solution);
    }

    #[test]
    fn generated_level_5_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(123);
        let game = crate::generate::generate_for_level(5, &mut rng).unwrap();
        let solution = solve(&game, false, false).solution.unwrap();
        verify_solution(&game, &solution);
    }

    #[test]
    fn rejects_insufficient_piece_cells() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.total_deficit(), 9);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn solution_maps_to_original_order() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let single_cell = Piece::from_grid(&[&[true]]);
        let domino = Piece::from_grid(&[&[true, true]]);
        let game = Game::new(board, vec![single_cell, domino]);
        let solution = solve(&game, false, false).solution.unwrap();
        assert_eq!(solution.len(), 2);
        verify_solution(&game, &solution);
    }

    #[test]
    fn rejects_piece_with_unavoidable_extra_cells() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true], &[true], &[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game, false, false).solution.is_none());
    }

    #[test]
    fn generated_levels_solvable() {
        for level in [1, 5, 10, 20, 25, 30] {
            let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
            let game = crate::generate::generate_for_level(level, &mut rng).unwrap();
            let result = solve(&game, false, false);
            assert!(
                result.solution.is_some(),
                "level {level} should be solvable"
            );
            verify_solution(&game, &result.solution.unwrap());
        }
    }

    #[test]
    fn solves_generated_games_across_board_sizes_and_moduli() {
        use crate::generate::generate_game;
        use crate::level::LevelSpec;
        use rayon::prelude::*;

        let configurations: Vec<(u8, u8, u8, u8)> = vec![
            (2, 3, 3, 4),
            (2, 3, 3, 8),
            (3, 3, 3, 3),
            (3, 3, 3, 7),
            (2, 4, 3, 5),
            (2, 4, 3, 8),
            (2, 4, 4, 6),
            (2, 4, 4, 10),
            (3, 4, 3, 6),
            (3, 4, 4, 8),
            (4, 4, 4, 6),
            (4, 4, 4, 10),
            (2, 6, 6, 8),
            (3, 6, 6, 8),
            (4, 6, 6, 8),
            (5, 6, 6, 6),
        ];

        let seeds: Vec<u64> = (0..5).collect();

        let failures: Vec<String> = configurations
            .par_iter()
            .flat_map_iter(|&(modulus, rows, columns, piece_count)| {
                let specification = LevelSpec {
                    level: 0,
                    shifts: modulus,
                    rows,
                    columns,
                    shapes: piece_count,
                };
                seeds.iter().filter_map(move |&seed| {
                    let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                    let game = generate_game(&specification, &mut rng);
                    let result = solve(&game, false, false);
                    match result.solution {
                        None => Some(format!(
                            "FAIL: no solution found for M={} {}x{} pieces={} seed={}",
                            modulus, rows, columns, piece_count, seed
                        )),
                        Some(ref solution) => {
                            let mut board = *game.board();
                            for (piece_index, &(row, column)) in solution.iter().enumerate() {
                                let mask = game.pieces()[piece_index].placed_at(row, column);
                                board.apply_piece(mask);
                            }
                            if !board.is_solved() {
                                Some(format!(
                                    "FAIL: invalid solution for M={} {}x{} pieces={} seed={}",
                                    modulus, rows, columns, piece_count, seed
                                ))
                            } else {
                                None
                            }
                        }
                    }
                })
            })
            .collect();

        if !failures.is_empty() {
            for failure in &failures[..failures.len().min(20)] {
                eprintln!("{}", failure);
            }
            panic!("{} fuzz test failures (showing first 20)", failures.len());
        }
    }

    // Exhaustive progress accounts for every branch in the naive search space.

    fn assert_progress_complete(result: &SolveResult, label: &str) {
        let progress = result.progress;
        assert!(
            (progress - 1.0).abs() < 1e-9,
            "{}: expected progress ≈ 1.0, got {:.15} (diff={:.2e})",
            label,
            progress,
            (progress - 1.0).abs()
        );
    }

    #[test]
    fn exhaustive_progress_for_one_piece() {
        let grid: &[&[u8]] = &[&[1, 0, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "trivial_1_piece");
    }

    #[test]
    fn exhaustive_progress_for_two_pieces() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece, piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "two_pieces");
    }

    #[test]
    fn exhaustive_progress_for_unsolvable_game() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_none());
        assert_progress_complete(&result, "no_solution");
    }

    #[test]
    fn exhaustive_progress_for_mixed_piece_sizes() {
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
    fn exhaustive_progress_with_modulus_three() {
        let grid: &[&[u8]] = &[&[1, 2, 0], &[0, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 3]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "m3");
    }

    #[test]
    fn exhaustive_progress_for_generated_levels() {
        use crate::generate::generate_for_level;
        for (level, seed) in [(1, 42u64), (2, 99), (3, 7), (5, 123)] {
            let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
            let game = generate_for_level(level, &mut rng).unwrap();
            let result = solve(&game, true, true);
            assert!(
                result.solution.is_some(),
                "level {} seed {} unsolved",
                level,
                seed
            );
            assert_progress_complete(&result, &format!("generated_level_{}_seed_{}", level, seed));
        }
    }

    #[test]
    fn exhaustive_progress_for_single_cell_suffix() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 9]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "all_single_cells");
    }

    #[test]
    fn exhaustive_progress_with_equivalent_piece_pairs() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[1, 1, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true, true]]);
        let game = Game::new(board, vec![piece, piece]);
        let result = solve(&game, true, true);
        assert!(result.solution.is_some());
        assert_progress_complete(&result, "duplicate_pieces");
    }
}
