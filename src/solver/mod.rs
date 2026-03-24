mod backtrack;
mod precompute;
pub(crate) mod pruning;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::board::MAX_M;
use crate::coverage::CoverageCounter;
use crate::game::Game;

use pruning::*;

/// A solution is a list of (row, col) placements, one per piece in original order.
pub type Solution = Vec<(usize, usize)>;

/// Result of a solve attempt: optional solution + number of nodes visited.
pub struct SolveResult {
    pub solution: Option<Solution>,
    pub nodes_visited: u64,
}

/// Configuration controlling which pruning techniques are enabled.
#[derive(Clone)]
pub struct PruningConfig {
    pub active_planes: bool,
    pub min_flips_global: bool,
    pub min_flips_rowcol: bool,
    pub min_flips_diagonal: bool,
    pub coverage: bool,
    pub jaggedness: bool,
    pub cell_locking: bool,
    pub component_checks: bool,
    pub duplicate_pruning: bool,
    pub single_cell_endgame: bool,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            active_planes: true,
            min_flips_global: true,
            min_flips_rowcol: true,
            min_flips_diagonal: true,
            coverage: true,
            jaggedness: true,
            cell_locking: true,
            component_checks: true,
            duplicate_pruning: true,
            single_cell_endgame: true,
        }
    }
}

impl PruningConfig {
    /// All pruning disabled.
    pub fn none() -> Self {
        Self {
            active_planes: false,
            min_flips_global: false,
            min_flips_rowcol: false,
            min_flips_diagonal: false,
            coverage: false,
            jaggedness: false,
            cell_locking: false,
            component_checks: false,
            duplicate_pruning: false,
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
#[allow(dead_code)]
pub(crate) struct SolverData {
    pub(crate) all_placements: Vec<Vec<(usize, usize, Bitboard)>>,
    pub(crate) reaches: Vec<Bitboard>,
    pub(crate) perimeters: Vec<u32>,
    pub(crate) h_perimeters: Vec<u32>,
    pub(crate) v_perimeters: Vec<u32>,
    pub(crate) cell_counts: Vec<u32>,
    pub(crate) remaining_bits: Vec<u32>,
    pub(crate) remaining_perimeter: Vec<u32>,
    pub(crate) remaining_h_perimeter: Vec<u32>,
    pub(crate) remaining_v_perimeter: Vec<u32>,
    pub(crate) jagg_h_mask: Bitboard,
    pub(crate) jagg_h_total: u32,
    pub(crate) jagg_v_mask: Bitboard,
    pub(crate) jagg_v_total: u32,
    pub(crate) line_families: [LineFamily; 6],
    pub(crate) suffix_coverage: Vec<CoverageCounter>,
    pub(crate) is_dup_of_prev: Vec<bool>,
    pub(crate) skip_tables: Vec<Option<Vec<bool>>>,
    pub(crate) single_cell_start: usize,
    pub(crate) m: u8,
    pub(crate) h: u8,
    pub(crate) w: u8,
    pub(crate) parity_partitions: Vec<ParityPartition>,
    pub(crate) subset_checks: Vec<SubsetReachability>,
    pub(crate) weight_tuple_checks: Vec<WeightTupleReachability>,
}

/// Solve with all pruning enabled. Tries cancellation reduction first.
pub fn solve(game: &Game) -> SolveResult {
    solve_with_cancellation(game, &PruningConfig::default())
}

/// Always use parallel solver.
fn solve_dispatch(game: &Game, config: &PruningConfig) -> SolveResult {
    solve_with_config_parallel(game, config)
}

/// Try solving reduced puzzles by removing cancellable groups of M identical pieces.
/// Exhaustively tries all combinations of per-group cancellation levels, from most
/// aggressive to least. Each group of K identical pieces can cancel 0, M, 2M, ...,
/// floor(K/M)*M pieces. The product space is typically small (<50 combos).
/// Falls back to the full puzzle if no reduction works.
fn solve_with_cancellation(game: &Game, config: &PruningConfig) -> SolveResult {
    let m = game.board().m() as usize;
    let pieces = game.pieces();
    let h = game.board().height();
    let w = game.board().width();

    // Count pieces per shape, preserving original indices.
    let mut shape_groups: Vec<(crate::piece::Piece, Vec<usize>)> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        if let Some(group) = shape_groups.iter_mut().find(|(s, _)| s == piece) {
            group.1.push(i);
        } else {
            shape_groups.push((*piece, vec![i]));
        }
    }

    // Only consider groups with M+ pieces (cancellable).
    // cancellable_groups[i] = (group_index, max_sets_to_cancel)
    let mut cancellable_groups: Vec<(usize, usize)> = Vec::new();
    for (g, (_, indices)) in shape_groups.iter().enumerate() {
        let max_sets = indices.len() / m;
        if max_sets > 0 {
            cancellable_groups.push((g, max_sets));
        }
    }

    if cancellable_groups.is_empty() {
        return solve_with_config_parallel(game, config);
    }

    // Enumerate all combinations of cancellation levels.
    let num_cgroups = cancellable_groups.len();

    // Build all combos as vectors of cancel-counts per cancellable group.
    let total_combos: usize = cancellable_groups.iter()
        .map(|(_, max_sets)| max_sets + 1)
        .product();

    // Cap at a reasonable limit to avoid pathological cases.
    if total_combos > 200 {
        // Too many combos -- fall back to just trying max and full.
        let result = try_cancellation_combo(game, config, &shape_groups, &cancellable_groups,
            &cancellable_groups.iter().map(|(_, ms)| *ms).collect::<Vec<_>>(),
            m, h, w);
        if result.solution.is_some() {
            return result;
        }
        let mut full = solve_with_config_parallel(game, config);
        full.nodes_visited += result.nodes_visited;
        return full;
    }

    // Generate all combos, sort by total pieces removed (descending = most aggressive first).
    let mut combos: Vec<Vec<usize>> = Vec::with_capacity(total_combos);
    let mut current = vec![0usize; num_cgroups];
    loop {
        // Skip the all-zeros combo (that's the full puzzle fallback).
        if current.iter().any(|&c| c > 0) {
            combos.push(current.clone());
        }
        // Increment like a mixed-radix counter.
        let mut carry = true;
        for i in 0..num_cgroups {
            if carry {
                current[i] += 1;
                if current[i] > cancellable_groups[i].1 {
                    current[i] = 0;
                } else {
                    carry = false;
                }
            }
        }
        if carry { break; }
    }

    // Sort: most total cancellations first (smaller puzzles tried first).
    combos.sort_unstable_by(|a, b| {
        let total_a: usize = a.iter().sum();
        let total_b: usize = b.iter().sum();
        total_b.cmp(&total_a)
    });

    let mut total_nodes = 0u64;

    for combo in &combos {
        let result = try_cancellation_combo(game, config, &shape_groups, &cancellable_groups,
            combo, m, h, w);
        total_nodes += result.nodes_visited;
        if result.solution.is_some() {
            return SolveResult {
                solution: result.solution,
                nodes_visited: total_nodes,
            };
        }
    }

    // For M=2: try pair-merge reduction before full solve.
    if m == 2 && pieces.len() >= 4 {
        let result = try_pair_merge(game, config);
        if result.solution.is_some() {
            return SolveResult {
                solution: result.solution,
                nodes_visited: total_nodes + result.nodes_visited,
            };
        }
        total_nodes += result.nodes_visited;
    }

    // No reduction worked -- solve the full puzzle.
    // Use parallel for large puzzles (cancellation already tried above).
    let mut full_result = {
        let n = pieces.len();
        let area = h as usize * w as usize;
        solve_with_config_parallel(game, config)
    };
    full_result.nodes_visited += total_nodes;
    full_result
}

/// For M=2: find a pair of pieces that can simulate a 1x1 piece at every board cell.
/// Replace them with a 1x1 piece, solve the reduced game, then reconstruct.
fn try_pair_merge(game: &Game, config: &PruningConfig) -> SolveResult {
    use std::collections::HashSet;

    let pieces = game.pieces();
    let h = game.board().height();
    let w = game.board().width();
    let n = pieces.len();
    let board_area = h as usize * w as usize;

    let all_pl: Vec<Vec<(usize, usize, Bitboard)>> = pieces.iter()
        .map(|p| p.placements(h, w))
        .collect();

    // For each pair, compute producible single-cell positions.
    let mut best_pair: Option<(usize, usize, Vec<Option<(usize, usize)>>)> = None;

    for i in 0..n {
        for j in (i + 1)..n {
            let mut witnesses: Vec<Option<(usize, usize)>> = vec![None; board_area];
            let mut count = 0usize;

            for (pa, &(_, _, ma)) in all_pl[i].iter().enumerate() {
                for (pb, &(_, _, mb)) in all_pl[j].iter().enumerate() {
                    let xor = ma ^ mb;
                    if xor.count_ones() == 1 {
                        let bit = xor.lowest_set_bit();
                        let r = bit / 15;
                        let c = bit % 15;
                        if r < h as u32 && c < w as u32 {
                            let cell_idx = r as usize * w as usize + c as usize;
                            if witnesses[cell_idx].is_none() {
                                witnesses[cell_idx] = Some((pa, pb));
                                count += 1;
                                if count == board_area { break; }
                            }
                        }
                    }
                }
                if count == board_area { break; }
            }

            if count == board_area {
                best_pair = Some((i, j, witnesses));
                break;
            } else if count > 0 {
                if best_pair.as_ref().map_or(true, |(_, _, w)| {
                    count > w.iter().filter(|x| x.is_some()).count()
                }) {
                    best_pair = Some((i, j, witnesses));
                }
            }
        }
        if best_pair.as_ref().map_or(false, |(_, _, w)| {
            w.iter().filter(|x| x.is_some()).count() == board_area
        }) {
            break;
        }
    }

    // Only proceed if the pair covers ALL board cells.
    let (pi, pj, witnesses) = match best_pair {
        Some((i, j, ref w)) if w.iter().all(|x| x.is_some()) => {
            (i, j, w.clone())
        }
        _ => return SolveResult { solution: None, nodes_visited: 0 },
    };

    // Build reduced game: replace pieces[pi] and pieces[pj] with a 1x1 piece.
    let p1x1 = crate::piece::Piece::from_grid(&[&[true]]);
    let mut reduced_pieces: Vec<crate::piece::Piece> = Vec::with_capacity(n - 1);
    let mut idx_map: Vec<usize> = Vec::with_capacity(n - 1);
    let mut merged_reduced_idx = 0;
    for k in 0..n {
        if k == pi {
            reduced_pieces.push(p1x1);
            idx_map.push(usize::MAX);
            merged_reduced_idx = reduced_pieces.len() - 1;
        } else if k == pj {
            continue;
        } else {
            reduced_pieces.push(pieces[k]);
            idx_map.push(k);
        }
    }

    let reduced_game = Game::new(game.board().clone(), reduced_pieces);
    let result = solve_with_cancellation(&reduced_game, config);

    if let Some(ref reduced_sol) = result.solution {
        let mut full_sol = vec![(0, 0); n];
        for (ri, &(row, col)) in reduced_sol.iter().enumerate() {
            if ri == merged_reduced_idx {
                let cell_idx = row * w as usize + col;
                let (pa, pb) = witnesses[cell_idx].unwrap();
                full_sol[pi] = (all_pl[pi][pa].0, all_pl[pi][pa].1);
                full_sol[pj] = (all_pl[pj][pb].0, all_pl[pj][pb].1);
            } else {
                full_sol[idx_map[ri]] = (row, col);
            }
        }
        return SolveResult {
            solution: Some(full_sol),
            nodes_visited: result.nodes_visited,
        };
    }

    SolveResult { solution: None, nodes_visited: result.nodes_visited }
}

/// Try a specific cancellation combo. Returns SolveResult.
fn try_cancellation_combo(
    game: &Game,
    config: &PruningConfig,
    shape_groups: &[(crate::piece::Piece, Vec<usize>)],
    cancellable_groups: &[(usize, usize)],
    combo: &[usize], // cancel_sets per cancellable group
    m: usize,
    h: u8,
    w: u8,
) -> SolveResult {
    let pieces = game.pieces();

    // Build kept and cancelled index lists.
    let mut cancelled_per_group: Vec<Option<(crate::piece::Piece, Vec<usize>)>> =
        vec![None; shape_groups.len()];
    for (ci, &(g, _)) in cancellable_groups.iter().enumerate() {
        let cancel_count = combo[ci] * m;
        if cancel_count > 0 {
            let indices = &shape_groups[g].1;
            let keep = indices.len() - cancel_count;
            cancelled_per_group[g] = Some((shape_groups[g].0, indices[keep..].to_vec()));
        }
    }

    let mut kept_indices: Vec<usize> = Vec::new();
    for (g, (_, indices)) in shape_groups.iter().enumerate() {
        let cancel_count = cancelled_per_group[g].as_ref()
            .map(|(_, ci)| ci.len()).unwrap_or(0);
        let keep = indices.len() - cancel_count;
        for &idx in &indices[..keep] {
            kept_indices.push(idx);
        }
    }

    if kept_indices.is_empty() {
        // All cancelled -- board must be solved.
        if game.board().is_solved() {
            let mut solution = vec![(0usize, 0usize); pieces.len()];
            for cpg in &cancelled_per_group {
                if let Some((shape, indices)) = cpg {
                    let placements = shape.placements(h, w);
                    if let Some(&(r, c, _)) = placements.first() {
                        for &idx in indices {
                            solution[idx] = (r, c);
                        }
                    }
                }
            }
            return SolveResult { solution: Some(solution), nodes_visited: 1 };
        }
        return SolveResult { solution: None, nodes_visited: 1 };
    }

    let reduced_pieces: Vec<crate::piece::Piece> =
        kept_indices.iter().map(|&i| pieces[i]).collect();
    let reduced_game = Game::new(game.board().clone(), reduced_pieces);
    let result = solve_dispatch(&reduced_game, config);

    if let Some(ref reduced_sol) = result.solution {
        let mut full_solution = vec![(0usize, 0usize); pieces.len()];
        for (ri, &(row, col)) in reduced_sol.iter().enumerate() {
            full_solution[kept_indices[ri]] = (row, col);
        }
        for cpg in &cancelled_per_group {
            if let Some((shape, indices)) = cpg {
                let placements = shape.placements(h, w);
                if let Some(&(r, c, _)) = placements.first() {
                    for &idx in indices {
                        full_solution[idx] = (r, c);
                    }
                }
            }
        }
        return SolveResult {
            solution: Some(full_solution),
            nodes_visited: result.nodes_visited,
        };
    }

    SolveResult { solution: None, nodes_visited: result.nodes_visited }
}

/// Backtracking solver with configurable pruning.
pub fn solve_with_config(game: &Game, config: &PruningConfig) -> SolveResult {
    let board = game.board().clone();
    let pieces = game.pieces();
    let h = board.height();
    let w = board.width();

    // Build (original_index, placements) and sort: fewer placements first.
    // Secondary sort by shape to group duplicates together.
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
            .then_with(|| pieces[*i].shape().limbs.cmp(&pieces[*j].shape().limbs))
    });

    let order: Vec<usize> = indexed.iter().map(|(i, _)| *i).collect();
    let all_placements: Vec<Vec<(usize, usize, Bitboard)>> =
        indexed.into_iter().map(|(_, p)| p).collect();

    let n = pieces.len();

    // Detect which pieces are duplicates of their predecessor (same shape).
    let is_dup_of_prev: Vec<bool> = (0..n)
        .map(|i| i > 0 && pieces[order[i]] == pieces[order[i - 1]])
        .collect();

    // Precompute pair skip tables for ALL consecutive piece pairs.
    let skip_tables: Vec<Option<Vec<bool>>> = (0..n).map(|i| {
        if i == 0 {
            return None;
        }
        let prev_pl = &all_placements[i - 1];
        let curr_pl = &all_placements[i];
        let num_prev = prev_pl.len();
        let num_curr = curr_pl.len();
        let is_dup = is_dup_of_prev[i];
        let mut table = vec![false; num_prev * num_curr];
        let mut seen = std::collections::HashSet::new();
        let mut any_skips = false;
        for a in 0..num_prev {
            let mask_a = prev_pl[a].2;
            let b_start = if is_dup { a } else { 0 };
            for b in b_start..num_curr {
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

    // Find where trailing 1x1 pieces start (they're sorted last = most placements).
    let single_cell_start = (0..n)
        .rposition(|i| pieces[order[i]].cell_count() != 1)
        .map(|i| i + 1)
        .unwrap_or(0);

    let m = board.m();

    let data = precompute::build_solver_data(
        pieces,
        &order,
        all_placements,
        is_dup_of_prev,
        skip_tables,
        single_cell_start,
        h,
        w,
        m,
    );

    let nodes = Cell::new(0u64);
    let mut sorted_solution = Vec::with_capacity(n);

    let found = backtrack::backtrack(
        &board,
        &data,
        0,
        0,
        usize::MAX, // no prev dup placement
        &mut sorted_solution,
        &nodes,
        config,
    );

    let solution = if found {
        // Map solution back to original piece order.
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
    }
}

/// Hash key for a board state: all plane limbs concatenated.
/// Board doesn't implement Hash, so we build a key from the planes' limbs.
#[derive(Clone, PartialEq, Eq, Hash)]
struct BoardKey {
    limbs: [u64; MAX_M * 4],
}

impl BoardKey {
    fn from_board(board: &Board) -> Self {
        let mut limbs = [0u64; MAX_M * 4];
        for d in 0..board.m() as usize {
            let plane = board.plane(d as u8);
            for (k, &l) in plane.limbs.iter().enumerate() {
                limbs[d * 4 + k] = l;
            }
        }
        Self { limbs }
    }
}

/// A single work item for the parallel solver.
struct WorkItem {
    board: Board,
    /// Prefix of the solution (placements for pieces 0..k in sorted order).
    prefix: Vec<(usize, usize)>,
    /// min_placement for piece k (for duplicate symmetry breaking across the boundary).
    min_placement_at_k: usize,
    /// prev_dup_placement for piece k (for skip table at the boundary).
    prev_dup_at_k: usize,
}

/// Parallel dedup solver for large puzzles.
/// 1. Sorts pieces and precomputes solver data (shared across threads).
/// 2. Enumerates all combos of the first K pieces with pruning, groups by board state.
/// 3. Spawns worker threads to search from each unique state.
fn solve_with_config_parallel(game: &Game, config: &PruningConfig) -> SolveResult {
    eprintln!("parallel: n={} area={}", game.pieces().len(), game.board().height() as usize * game.board().width() as usize);
    let board = game.board().clone();
    let pieces = game.pieces();
    let h = board.height();
    let w = board.width();

    // Build (original_index, placements) and sort: fewer placements first.
    // (Same sorting as solve_with_config.)
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
            .then_with(|| pieces[*i].shape().limbs.cmp(&pieces[*j].shape().limbs))
    });

    let order: Vec<usize> = indexed.iter().map(|(i, _)| *i).collect();
    let all_placements: Vec<Vec<(usize, usize, Bitboard)>> =
        indexed.into_iter().map(|(_, p)| p).collect();

    let n = pieces.len();

    let is_dup_of_prev: Vec<bool> = (0..n)
        .map(|i| i > 0 && pieces[order[i]] == pieces[order[i - 1]])
        .collect();

    let skip_tables: Vec<Option<Vec<bool>>> = (0..n).map(|i| {
        if i == 0 {
            return None;
        }
        let prev_pl = &all_placements[i - 1];
        let curr_pl = &all_placements[i];
        let num_prev = prev_pl.len();
        let num_curr = curr_pl.len();
        let is_dup = is_dup_of_prev[i];
        let mut table = vec![false; num_prev * num_curr];
        let mut seen = std::collections::HashSet::new();
        let mut any_skips = false;
        for a in 0..num_prev {
            let mask_a = prev_pl[a].2;
            let b_start = if is_dup { a } else { 0 };
            for b in b_start..num_curr {
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

    // Build solver data first -- we need it for pruning during combo enumeration.
    let t0 = std::time::Instant::now();
    let data = precompute::build_solver_data(
        pieces,
        &order,
        all_placements,
        is_dup_of_prev,
        skip_tables,
        single_cell_start,
        h,
        w,
        m,
    );
    eprintln!("precompute: {:.3?}", t0.elapsed());

    // Choose K: find the smallest K in {2, 3} such that the product of
    // placement counts for pieces 0..K is < 50_000.
    let k = {
        let mut k = 2;
        if n >= 3 {
            let combos_2: usize = data.all_placements[..2].iter().map(|p| p.len()).product();
            if combos_2 >= 50_000 {
                k = 2;
            } else {
                let combos_3: usize = data.all_placements[..3].iter().map(|p| p.len()).product();
                if combos_3 < 50_000 {
                    k = 3;
                }
            }
        }
        k.min(n)
    };

    let mut all_combos: Vec<WorkItem> = Vec::new();
    let t1 = std::time::Instant::now();
    enumerate_combos_pruned(
        &board,
        &data,
        k,
        0,
        0,
        usize::MAX,
        &mut Vec::with_capacity(k),
        &mut all_combos,
        config,
    );
    eprintln!("enumerate: {:.3?}, combos={}, k={}", t1.elapsed(), all_combos.len(), k);

    // Group combos by resulting board state (dedup).
    // For deduped groups, keep only the first combo (with its boundary info).
    let mut state_map: HashMap<BoardKey, usize> = HashMap::new();
    let mut work_items: Vec<WorkItem> = Vec::new();

    for combo in all_combos {
        let key = BoardKey::from_board(&combo.board);
        if let std::collections::hash_map::Entry::Vacant(e) = state_map.entry(key) {
            e.insert(work_items.len());
            work_items.push(combo);
        }
    }

    // Shared abort flag.
    let abort = AtomicBool::new(false);
    let result: Mutex<Option<(Vec<(usize, usize)>, Vec<(usize, usize)>)>> = Mutex::new(None);
    let total_nodes = std::sync::atomic::AtomicU64::new(0);
    let next_work = std::sync::atomic::AtomicUsize::new(0);

    let num_threads = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                let nodes = Cell::new(0u64);
                let mut solution = Vec::with_capacity(n);
                loop {
                    if abort.load(Ordering::Relaxed) {
                        break;
                    }
                    let idx = next_work.fetch_add(1, Ordering::Relaxed);
                    if idx >= work_items.len() {
                        break;
                    }
                    let item = &work_items[idx];

                    solution.clear();
                    nodes.set(0);

                    let found = backtrack::backtrack_abortable(
                        &item.board,
                        &data,
                        k,
                        item.min_placement_at_k,
                        item.prev_dup_at_k,
                        &mut solution,
                        &nodes,
                        config,
                        &abort,
                    );

                    total_nodes.fetch_add(nodes.get(), Ordering::Relaxed);

                    if found {
                        abort.store(true, Ordering::Relaxed);
                        let mut guard = result.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some((item.prefix.clone(), solution.clone()));
                        }
                    }
                }
            });
        }
    });

    let result = result.into_inner().unwrap();
    let nodes_visited = total_nodes.load(Ordering::Relaxed);

    let solution = result.map(|(prefix, suffix)| {
        let mut sorted_solution = prefix;
        sorted_solution.extend_from_slice(&suffix);
        // Map solution back to original piece order.
        let mut solution = vec![(0, 0); n];
        for (sorted_idx, &(row, col)) in sorted_solution.iter().enumerate() {
            solution[order[sorted_idx]] = (row, col);
        }
        solution
    });

    SolveResult {
        solution,
        nodes_visited,
    }
}

/// Recursively enumerate all valid combos of the first `k` pieces.
/// Respects duplicate symmetry breaking and skip tables.
/// At the base case (piece_idx == k), records the combo with boundary info
/// needed to correctly start the backtrack at piece k.
fn enumerate_combos_pruned(
    board: &Board,
    data: &SolverData,
    k: usize,
    piece_idx: usize,
    min_placement: usize,
    prev_dup_placement: usize,
    prefix: &mut Vec<(usize, usize)>,
    results: &mut Vec<WorkItem>,
    config: &PruningConfig,
) {
    if piece_idx == k {
        results.push(WorkItem {
            board: board.clone(),
            prefix: prefix.clone(),
            min_placement_at_k: min_placement,
            prev_dup_at_k: prev_dup_placement,
        });
        return;
    }

    let placements = &data.all_placements[piece_idx];
    let mut board = board.clone();

    for pl_idx in 0..placements.len() {
        // Duplicate symmetry breaking.
        if config.duplicate_pruning && pl_idx < min_placement {
            continue;
        }

        // Skip pair combos with same net effect as a previously tried combo.
        if prev_dup_placement < usize::MAX {
            if let Some(ref table) = data.skip_tables[piece_idx] {
                let num_curr = placements.len();
                if table[prev_dup_placement * num_curr + pl_idx] {
                    continue;
                }
            }
        }

        let (row, col, mask) = placements[pl_idx];

        board.apply_piece(mask);
        prefix.push((row, col));

        let is_next_dup = config.duplicate_pruning
            && piece_idx + 1 < data.all_placements.len()
            && data.is_dup_of_prev[piece_idx + 1];

        let next_min = if is_next_dup { pl_idx } else { 0 };
        let next_prev_dup = if piece_idx + 1 < data.all_placements.len()
            && data.skip_tables[piece_idx + 1].is_some()
        {
            pl_idx
        } else {
            usize::MAX
        };

        enumerate_combos_pruned(
            &board,
            data,
            k,
            piece_idx + 1,
            next_min,
            next_prev_dup,
            prefix,
            results,
            config,
        );

        prefix.pop();
        board.undo_piece(mask);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::game::Game;
    use crate::piece::Piece;

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
        let sol = solve(&game).solution.unwrap();
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
        let sol = solve(&game).solution.unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_no_solution() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 3);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game).solution.is_none());
    }

    #[test]
    fn test_all_single_cells() {
        // 3x3, m=2. Board all 1s. Nine 1x1 pieces.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece; 9]);
        let sol = solve(&game).solution.unwrap();
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
        let sol = solve(&game).solution.unwrap();
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
        assert!(solve(&game).solution.is_none());
    }

    #[test]
    fn test_mixed_then_single() {
        // Mix of multi-cell and single-cell pieces.
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let big = Piece::from_grid(&[&[true, true], &[true, false]]); // L-shape, 3 cells
        let small = Piece::from_grid(&[&[true]]); // 1x1
        let game = Game::new(board, vec![big, small]);
        let sol = solve(&game).solution.unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_generated_game_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
        let game = crate::generate::generate_for_level(1, &mut rng).unwrap();
        let sol = solve(&game).solution.unwrap();
        assert_eq!(sol.len(), game.pieces().len());
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_generated_level_5_solvable() {
        let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(123);
        let game = crate::generate::generate_for_level(5, &mut rng).unwrap();
        let sol = solve(&game).solution.unwrap();
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_min_flips_pruning() {
        let grid: &[&[u8]] = &[&[1, 1, 1], &[1, 1, 1], &[1, 1, 1]];
        let board = Board::from_grid(grid, 2);
        assert_eq!(board.min_flips_needed(), 9);
        let piece = Piece::from_grid(&[&[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game).solution.is_none());
    }

    #[test]
    fn test_solution_maps_to_original_order() {
        let grid: &[&[u8]] = &[&[1, 1, 0], &[1, 0, 0], &[0, 0, 0]];
        let board = Board::from_grid(grid, 2);
        let p0 = Piece::from_grid(&[&[true]]);
        let p1 = Piece::from_grid(&[&[true, true]]);
        let game = Game::new(board, vec![p0, p1]);
        let sol = solve(&game).solution.unwrap();
        assert_eq!(sol.len(), 2);
        verify_solution(&game, &sol);
    }

    #[test]
    fn test_coverage_pruning_unreachable() {
        let grid: &[&[u8]] = &[&[0, 0, 0], &[0, 0, 0], &[0, 0, 1]];
        let board = Board::from_grid(grid, 2);
        let piece = Piece::from_grid(&[&[true], &[true], &[true]]);
        let game = Game::new(board, vec![piece]);
        assert!(solve(&game).solution.is_none());
    }

    #[test]
    fn test_generated_levels_solvable() {
        for level in [1, 5, 10, 20, 25, 30] {
            let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(42);
            let game = crate::generate::generate_for_level(level, &mut rng).unwrap();
            let result = solve(&game);
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
            // Small boards, M=2
            (2, 3, 3, 2), (2, 3, 3, 4), (2, 3, 3, 6), (2, 3, 3, 8),
            // Small boards, M=3
            (3, 3, 3, 3), (3, 3, 3, 5), (3, 3, 3, 7),
            // Medium boards, M=2
            (2, 4, 3, 5), (2, 4, 3, 8), (2, 4, 3, 12),
            (2, 4, 4, 6), (2, 4, 4, 10), (2, 4, 4, 14),
            // Medium boards, M=3
            (3, 4, 3, 6), (3, 4, 3, 10),
            (3, 4, 4, 8), (3, 4, 4, 12),
            // Medium boards, M=4
            (4, 4, 4, 6), (4, 4, 4, 10),
            // Larger boards, M=2
            (2, 6, 6, 8), (2, 6, 6, 12),
            // Larger boards, M=3
            (3, 6, 6, 8), (3, 6, 6, 12),
            // Larger boards, M=4
            (4, 6, 6, 8), (4, 6, 6, 10),
            // Larger boards, M=5
            (5, 6, 6, 6), (5, 6, 6, 8),
            // Big boards, low piece count
            (2, 8, 7, 8), (3, 8, 7, 8), (4, 8, 8, 8),
            (2, 10, 10, 8), (3, 10, 10, 8), (4, 10, 10, 8),
        ];

        let seeds: Vec<u64> = (0..50).collect();

        let failures: Vec<String> = configs
            .par_iter()
            .flat_map(|&(m, rows, cols, shapes)| {
                let spec = LevelSpec {
                    level: 0,
                    shifts: m,
                    rows,
                    columns: cols,
                    shapes,
                    preview: false,
                };
                seeds.par_iter().filter_map(move |&seed| {
                    let mut rng =
                        <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                    let game = generate_game(&spec, &mut rng);
                    let result = solve(&game);
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
                level: 0, shifts: m, rows, columns: cols, shapes, preview: false,
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
            (2, 3, 3, 4), (2, 3, 3, 6), (2, 3, 3, 8),
            (3, 3, 3, 3), (3, 3, 3, 5),
            (2, 4, 3, 5), (2, 4, 3, 8),
            (3, 4, 3, 6),
            (2, 4, 4, 6), (2, 4, 4, 10),
            (3, 4, 4, 8),
        ]
    }

    fn test_seeds() -> Vec<u64> {
        (0..30).collect()
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
    fn test_prune_min_flips_global() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.min_flips_global = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "min_flips_global prune caused failures");
        assert!(nodes_with <= nodes_without,
            "min_flips_global should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_prune_min_flips_rowcol() {
        let configs = small_configs();
        let seeds = test_seeds();
        // Enable global so the rowcol check has something to build on.
        let baseline = PruningConfig::none().only(|c| c.min_flips_global = true);
        let with_prune = PruningConfig::none().only(|c| {
            c.min_flips_global = true;
            c.min_flips_rowcol = true;
        });

        let (nodes_baseline, _) = fuzz_with_config(&baseline, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "min_flips_rowcol prune caused failures");
        assert!(nodes_with <= nodes_baseline,
            "min_flips_rowcol should reduce nodes: {} vs {}", nodes_with, nodes_baseline);
    }

    #[test]
    fn test_prune_min_flips_diagonal() {
        let configs = small_configs();
        let seeds = test_seeds();
        let baseline = PruningConfig::none().only(|c| c.min_flips_global = true);
        let with_prune = PruningConfig::none().only(|c| {
            c.min_flips_global = true;
            c.min_flips_diagonal = true;
        });

        let (nodes_baseline, _) = fuzz_with_config(&baseline, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "min_flips_diagonal prune caused failures");
        assert!(nodes_with <= nodes_baseline,
            "min_flips_diagonal should reduce nodes: {} vs {}", nodes_with, nodes_baseline);
    }

    #[test]
    fn test_prune_min_flips_rowcol_soundness_stress() {
        // Larger configs to stress test row/col pruning soundness.
        let configs = vec![
            (2, 4, 4, 10), (2, 4, 4, 14),
            (3, 4, 4, 8), (3, 4, 4, 12),
            (2, 6, 6, 8), (2, 6, 6, 12),
            (3, 6, 6, 8), (4, 6, 6, 8),
            (2, 8, 7, 8), (3, 8, 7, 8),
        ];
        let seeds: Vec<u64> = (0..50).collect();
        let config = PruningConfig::none().only(|c| {
            c.min_flips_global = true;
            c.min_flips_rowcol = true;
        });
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "min_flips_rowcol stress test had {} failures", failures);
    }

    #[test]
    fn test_prune_min_flips_diagonal_soundness_stress() {
        let configs = vec![
            (2, 4, 4, 10), (2, 4, 4, 14),
            (3, 4, 4, 8), (3, 4, 4, 12),
            (2, 6, 6, 8), (2, 6, 6, 12),
            (3, 6, 6, 8), (4, 6, 6, 8),
            (2, 8, 7, 8), (3, 8, 7, 8),
        ];
        let seeds: Vec<u64> = (0..50).collect();
        let config = PruningConfig::none().only(|c| {
            c.min_flips_global = true;
            c.min_flips_diagonal = true;
        });
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "min_flips_diagonal stress test had {} failures", failures);
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
    fn test_prune_component_checks() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.component_checks = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "component_checks prune caused failures");
        assert!(nodes_with <= nodes_without,
            "component_checks should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_prune_duplicate_pruning() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.duplicate_pruning = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "duplicate_pruning caused failures");
        assert!(nodes_with <= nodes_without,
            "duplicate_pruning should reduce nodes: {} vs {}", nodes_with, nodes_without);
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
            (2, 8, 7, 8), (3, 8, 7, 8),
            (4, 8, 8, 8), (5, 6, 6, 6),
        ];
        let seeds: Vec<u64> = (0..50).collect();
        let (_, failures) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        assert_eq!(failures, 0, "parity partition stress test had {} failures", failures);
    }

    #[test]
    fn test_parity_partition_effectiveness() {
        let configs = small_configs();
        let seeds = test_seeds();
        let (nodes_with, _) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        let mut no_parity = PruningConfig::default();
        no_parity.min_flips_global = false;
        let (nodes_without, _) = fuzz_with_config(&no_parity, &configs, &seeds);
        assert!(nodes_with <= nodes_without,
            "parity partitions should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_pair_skip_tables_non_identical() {
        use crate::generate::generate_game;
        use crate::level::LevelSpec;

        let configs = vec![
            (2, 3, 3, 4), (2, 3, 3, 6), (2, 3, 3, 8),
            (2, 4, 3, 5), (2, 4, 3, 8),
            (2, 4, 4, 6), (2, 4, 4, 10),
            (3, 4, 4, 8), (3, 4, 4, 12),
        ];
        let seeds: Vec<u64> = (0..30).collect();

        let mut total_with = 0u64;
        let mut total_without = 0u64;
        let mut failures = 0usize;

        for &(m, rows, cols, shapes) in &configs {
            let spec = LevelSpec {
                level: 0, shifts: m, rows, columns: cols, shapes, preview: false,
            };
            for &seed in &seeds {
                let mut rng =
                    <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = generate_game(&spec, &mut rng);

                let result_with = solve(&game);
                total_with += result_with.nodes_visited;

                if let Some(ref sol) = result_with.solution {
                    let mut board = game.board().clone();
                    for (i, &(row, col)) in sol.iter().enumerate() {
                        let mask = game.pieces()[i].placed_at(row, col);
                        board.apply_piece(mask);
                    }
                    if !board.is_solved() {
                        failures += 1;
                    }
                } else {
                    failures += 1;
                }
            }
        }

        assert_eq!(failures, 0, "pair skip tables caused {} failures", failures);
    }

    #[test]
    fn test_pair_skip_tables_soundness_stress() {
        use crate::generate::generate_game;
        use crate::level::LevelSpec;

        let configs = vec![
            (2, 4, 4, 10), (2, 4, 4, 14),
            (3, 4, 4, 8), (3, 4, 4, 12),
            (2, 6, 6, 8), (2, 6, 6, 12),
            (3, 6, 6, 8), (4, 6, 6, 8),
        ];
        let seeds: Vec<u64> = (0..50).collect();

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
        let result = solve(&game);
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
                    level: 99, shifts: m, rows: h, columns: w, shapes: n, preview: true,
                };
                let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = crate::generate::generate_game(&spec, &mut rng);
                let result = solve(&game);
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
        let result = solve(&game);
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
                    level: 99, shifts: m, rows: h, columns: w, shapes: n, preview: true,
                };
                let mut rng = <rand::rngs::SmallRng as rand::SeedableRng>::seed_from_u64(seed);
                let game = crate::generate::generate_game(&spec, &mut rng);
                let result = solve(&game);
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
        let result = solve(&game);
        assert!(result.solution.is_some(), "4 identical pieces on solved board should cancel");
        verify_solution(&game, result.solution.as_ref().unwrap());
    }
}
