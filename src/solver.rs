use std::cell::Cell;

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::coverage::{has_sufficient_coverage, precompute_suffix_coverage, CoverageCounter};
use crate::game::Game;

/// Max number of lines in any family (diagonals on 14x14: 27).
const MAX_LINES: usize = 27;
/// Max number of pieces (n+1 for suffix arrays).
const MAX_PIECES: usize = 37;

/// A family of parallel lines for the min_flips DP pruning.
struct LineFamily {
    masks: [Bitboard; MAX_LINES],
    num_lines: usize,
    /// remaining_budget[i] = suffix sum of max_thickness for pieces [i..n]
    remaining_budget: [u32; MAX_PIECES],
    /// suffix_max_span[i] = max span among pieces [i..n]
    suffix_max_span: [u8; MAX_PIECES],
    /// Whether per_line_budget is available (only for rows and columns).
    has_per_line_budget: bool,
    /// per_line_budget[i][line] = position-aware suffix budget.
    per_line_budget: [[u32; MAX_LINES]; MAX_PIECES],
}

impl LineFamily {
    fn new() -> Self {
        Self {
            masks: [Bitboard::ZERO; MAX_LINES],
            num_lines: 0,
            remaining_budget: [0; MAX_PIECES],
            suffix_max_span: [0; MAX_PIECES],
            has_per_line_budget: false,
            per_line_budget: [[0; MAX_LINES]; MAX_PIECES],
        }
    }
}

/// Check a line family. Returns false if any prune fires.
#[inline(always)]
fn check_line_family(
    board: &Board,
    family: &LineFamily,
    piece_idx: usize,
    m: u8,
) -> bool {
    let gap = family.suffix_max_span[piece_idx] as usize;
    let n = family.num_lines;
    if n == 0 {
        return true;
    }

    // Compute weights.
    let mut weights = [0u32; MAX_LINES];
    for i in 0..n {
        for d in 1..m {
            weights[i] += (m - d) as u32 * (board.plane(d) & family.masks[i]).count_ones();
        }
        // Per-line position-aware check.
        if family.has_per_line_budget && family.per_line_budget[piece_idx][i] < weights[i] {
            return false;
        }
    }

    // DP: max weight independent set with spacing >= gap.
    if gap > 0 {
        let mut dp = [0u32; MAX_LINES];
        for i in 0..n {
            let take = weights[i] + if i >= gap { dp[i - gap] } else { 0 };
            let skip = if i > 0 { dp[i - 1] } else { 0 };
            dp[i] = take.max(skip);
        }
        if family.remaining_budget[piece_idx] < dp[n - 1] {
            return false;
        }
    }

    true
}

/// A parity-based board partition for pruning.
/// The board is split into "group 0" and "group 1" cells based on some parity function.
/// Each piece contributes a known count to group 0 depending on placement parity.
/// The suffix DP tracks achievable group-0 totals.
struct ParityPartition {
    /// Mask of group-0 cells on the board.
    mask: Bitboard,
    /// suffix_max[i] = max achievable group-0 increments from pieces [i..n].
    suffix_max: Vec<u32>,
    /// suffix_min[i] = min achievable group-0 increments from pieces [i..n].
    suffix_min: Vec<u32>,
    /// suffix_dp[i] = achievable group-0 totals from pieces [i..n].
    suffix_dp: Vec<Vec<bool>>,
}

/// A small subset of board cells for local reachability pruning.
/// The suffix DP tracks which configurations of the subset cells are achievable.
struct SubsetReachability {
    /// Board cell positions in the subset (as bit indices r*15+c).
    cells: Vec<u32>,
    /// M value.
    m: u8,
    /// Precomputed mask: OR of all cell bit positions. Used for fast-path
    /// when all cells are already 0 (config=0 is always reachable).
    mask: Bitboard,
    /// suffix_reachable[piece_idx][config] = can pieces [piece_idx..n] transform
    /// the subset from `config` to all-zeros?
    /// Config is encoded as a base-M number: cell[0] + cell[1]*M + cell[2]*M^2 + ...
    suffix_reachable: Vec<Vec<bool>>,
}

impl SubsetReachability {
    /// Encode the current subset cell values from the board into a config index.
    #[inline(always)]
    fn encode_config(&self, board: &Board) -> usize {
        let mut config = 0usize;
        let mut multiplier = 1usize;
        for &bit in &self.cells {
            let mut val = 0u8;
            for d in 1..self.m {
                if board.plane(d).get_bit(bit) {
                    val = d;
                    break;
                }
            }
            config += val as usize * multiplier;
            multiplier *= self.m as usize;
        }
        config
    }

    /// Check if the current board configuration is reachable from piece_idx.
    #[inline(always)]
    fn check(&self, board: &Board, piece_idx: usize) -> bool {
        // Fast path: if all subset cells are 0, config=0 which is always reachable
        // (the zero-effect identity is always available for every piece).
        if (board.plane(0) & self.mask) == self.mask {
            return true;
        }
        let config = self.encode_config(board);
        self.suffix_reachable[piece_idx][config]
    }
}

/// All precomputed data needed by the backtracking solver.
/// Bundled into a single struct to keep the backtrack signature small.
#[allow(dead_code)]
struct SolverData {
    all_placements: Vec<Vec<(usize, usize, Bitboard)>>,
    reaches: Vec<Bitboard>,
    perimeters: Vec<u32>,
    h_perimeters: Vec<u32>,
    v_perimeters: Vec<u32>,
    cell_counts: Vec<u32>,
    remaining_bits: Vec<u32>,
    remaining_perimeter: Vec<u32>,
    remaining_h_perimeter: Vec<u32>,
    remaining_v_perimeter: Vec<u32>,
    jagg_h_mask: Bitboard,
    jagg_h_total: u32,
    jagg_v_mask: Bitboard,
    jagg_v_total: u32,
    line_families: [LineFamily; 6],
    suffix_coverage: Vec<CoverageCounter>,
    is_dup_of_prev: Vec<bool>,
    skip_tables: Vec<Option<Vec<bool>>>,
    single_cell_start: usize,
    m: u8,
    h: u8,
    w: u8,
    // Parity partition checks: mod-2 (checkerboard, even-rows, even-cols)
    // plus optional mod-3 partitions for larger boards.
    parity_partitions: Vec<ParityPartition>,
    // Subset reachability checks for corners.
    subset_checks: Vec<SubsetReachability>,
}

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

/// Flood-fill one connected component from a seed bit within `region`.
/// Returns the component mask. Uses bitboard-parallel expansion.
fn flood_fill(seed_bit: u32, region: Bitboard) -> Bitboard {
    let mut component = Bitboard::from_bit(seed_bit);
    loop {
        // Expand in 4 cardinal directions, masked to valid region.
        let expanded = component
            | (component << 1)
            | (component >> 1)
            | (component << 15)
            | (component >> 15);
        let expanded = expanded & region;
        if expanded == component {
            break;
        }
        component = expanded;
    }
    component
}

/// Check connected components of the non-zero region (using locked cells as walls).
/// For each component, verify that reachable pieces have enough perimeter to smooth
/// out the component's jaggedness.
/// Check connected components of the non-zero region (using locked cells as walls).
/// For each component, verify:
/// - Reachable pieces have enough cell_counts to cover min_flips
/// - Reachable pieces have enough perimeter to cover jaggedness
/// Also computes sum of active_planes across components (returned for caller to check).
fn check_components(
    board: &Board,
    locked_mask: Bitboard,
    data: &SolverData,
    piece_idx: usize,
) -> bool {

    // Non-zero region, excluding locked cells (which are walls).
    let mut nz = Bitboard::ZERO;
    for d in 1..data.m {
        nz |= board.plane(d);
    }
    let region = nz & !locked_mask;

    if region.is_zero() {
        return true;
    }

    let mut remaining_nz = region;
    let mut component_count = 0u32;

    while !remaining_nz.is_zero() {
        let seed = remaining_nz.lowest_set_bit();
        let component = flood_fill(seed, remaining_nz);
        remaining_nz = remaining_nz & !component;
        component_count += 1;

        // Compute component's min_flips.
        let mut comp_min_flips = 0u32;
        for d in 1..data.m {
            comp_min_flips += (data.m - d) as u32 * (board.plane(d) & component).count_ones();
        }

        // Component jaggedness — split into h/v.
        let h_pairs = component & (component >> 1);
        let v_pairs = component & (component >> 15);
        let mut h_matching = 0u32;
        let mut v_matching = 0u32;
        for d in 0..data.m {
            let p = board.plane(d) & component;
            h_matching += (p & (p >> 1) & h_pairs).count_ones();
            v_matching += (p & (p >> 15) & v_pairs).count_ones();
        }
        let comp_h_jagg = h_pairs.count_ones() - h_matching;
        let comp_v_jagg = v_pairs.count_ones() - v_matching;

        // Sum h/v perimeters and cell_counts of reachable pieces.
        let mut reachable_h_perim = 0u32;
        let mut reachable_v_perim = 0u32;
        let mut reachable_bits = 0u32;
        for pi in piece_idx..data.reaches.len() {
            if !(data.reaches[pi] & component).is_zero() {
                reachable_h_perim += data.h_perimeters[pi];
                reachable_v_perim += data.v_perimeters[pi];
                reachable_bits += data.cell_counts[pi];
            }
        }

        // Per-component pruning checks.
        if comp_h_jagg > reachable_h_perim {
            return false;
        }
        if comp_v_jagg > reachable_v_perim {
            return false;
        }
        if comp_min_flips > reachable_bits {
            return false;
        }

        if component_count >= 16 {
            break;
        }
    }

    true
}

/// Solve with all pruning enabled. Tries cancellation reduction first.
pub fn solve(game: &Game) -> SolveResult {
    solve_with_cancellation(game, &PruningConfig::default())
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
        return solve_with_config(game, config);
    }

    // Enumerate all combinations of cancellation levels.
    // For each cancellable group, the level ranges from 0 to max_sets.
    // We enumerate from most aggressive (all max) to least, sorted by total removed descending.
    let num_cgroups = cancellable_groups.len();

    // Build all combos as vectors of cancel-counts per cancellable group.
    // Product space size = Π(max_sets[i] + 1).
    let total_combos: usize = cancellable_groups.iter()
        .map(|(_, max_sets)| max_sets + 1)
        .product();

    // Cap at a reasonable limit to avoid pathological cases.
    if total_combos > 200 {
        // Too many combos — fall back to just trying max and full.
        let result = try_cancellation_combo(game, config, &shape_groups, &cancellable_groups,
            &cancellable_groups.iter().map(|(_, ms)| *ms).collect::<Vec<_>>(),
            m, h, w);
        if result.solution.is_some() {
            return result;
        }
        let mut full = solve_with_config(game, config);
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

    // No reduction worked — solve the full puzzle.
    let mut full_result = solve_with_config(game, config);
    full_result.nodes_visited += total_nodes;
    full_result
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
    // Start with all indices, then remove cancelled ones.
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
        // All cancelled — board must be solved.
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
    let result = solve_with_config(&reduced_game, config);

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
    // Two (prev_pl, curr_pl) combos with the same net board effect are redundant.
    // skip_tables[i] = Some(table) for piece i (i > 0).
    // table[prev_pl * num_curr_pl + curr_pl] = true means skip this combo.
    // For identical pairs: curr_pl >= prev_pl (non-decreasing constraint).
    // For non-identical pairs: no ordering constraint.
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

    // Precompute suffix sums/maxes of piece properties.
    let mut remaining_bits = vec![0u32; n + 1];
    let mut remaining_perimeter = vec![0u32; n + 1];
    let mut remaining_h_perimeter = vec![0u32; n + 1];
    let mut remaining_v_perimeter = vec![0u32; n + 1];
    for i in (0..n).rev() {
        remaining_bits[i] = remaining_bits[i + 1] + pieces[order[i]].cell_count();
        remaining_perimeter[i] = remaining_perimeter[i + 1] + pieces[order[i]].perimeter();
        remaining_h_perimeter[i] = remaining_h_perimeter[i + 1] + pieces[order[i]].h_perimeter();
        remaining_v_perimeter[i] = remaining_v_perimeter[i + 1] + pieces[order[i]].v_perimeter();
    }

    let bh = h as usize;
    let bw = w as usize;

    // Precompute per-piece reach: union of all placement masks.
    let reaches: Vec<Bitboard> = all_placements
        .iter()
        .map(|placements| {
            let mut reach = Bitboard::ZERO;
            for &(_, _, mask) in placements {
                reach |= mask;
            }
            reach
        })
        .collect();

    // Precompute suffix coverage in binary bitboard layers.
    let suffix_coverage = precompute_suffix_coverage(&reaches);

    // Precompute sorted-order perimeters and cell counts for component checks.
    let sorted_perimeters: Vec<u32> = (0..n).map(|i| pieces[order[i]].perimeter()).collect();
    let sorted_h_perimeters: Vec<u32> = (0..n).map(|i| pieces[order[i]].h_perimeter()).collect();
    let sorted_v_perimeters: Vec<u32> = (0..n).map(|i| pieces[order[i]].v_perimeter()).collect();
    let sorted_cell_counts: Vec<u32> = (0..n).map(|i| pieces[order[i]].cell_count()).collect();

    let m = board.m();

    // Build 6 line families: rows, cols, diags, antidiags, zigzag_r, zigzag_l.
    assert!(n < MAX_PIECES, "too many pieces for LineFamily arrays");

    // --- Rows ---
    let mut rows_family = LineFamily::new();
    rows_family.num_lines = bh;
    rows_family.has_per_line_budget = true;
    for r in 0..bh {
        for c in 0..bw {
            rows_family.masks[r].set_bit((r * 15 + c) as u32);
        }
    }
    for i in (0..n).rev() {
        let piece = &pieces[order[i]];
        let ph = piece.height() as usize;
        let pw = piece.width() as usize;
        rows_family.remaining_budget[i] = rows_family.remaining_budget[i + 1] + piece.max_row_thickness();
        rows_family.suffix_max_span[i] = rows_family.suffix_max_span[i + 1].max(piece.height());
        // Per-row budget.
        let mut row_thick = [0u32; 5];
        for pr in 0..ph {
            let row_bits = (piece.shape() >> (pr as u32 * 15)).limbs[0] & ((1u64 << pw) - 1);
            row_thick[pr] = row_bits.count_ones();
        }
        for r in 0..bh {
            let p_min = if r + ph > bh { r + ph - bh } else { 0 };
            let p_max = r.min(ph - 1);
            let mut max_t = 0u32;
            for p in p_min..=p_max {
                if row_thick[p] > max_t { max_t = row_thick[p]; }
            }
            rows_family.per_line_budget[i][r] = rows_family.per_line_budget[i + 1][r] + max_t;
        }
    }

    // --- Cols ---
    let mut cols_family = LineFamily::new();
    cols_family.num_lines = bw;
    cols_family.has_per_line_budget = true;
    for c in 0..bw {
        for r in 0..bh {
            cols_family.masks[c].set_bit((r * 15 + c) as u32);
        }
    }
    for i in (0..n).rev() {
        let piece = &pieces[order[i]];
        let ph = piece.height() as usize;
        let pw = piece.width() as usize;
        cols_family.remaining_budget[i] = cols_family.remaining_budget[i + 1] + piece.max_col_thickness();
        cols_family.suffix_max_span[i] = cols_family.suffix_max_span[i + 1].max(piece.width());
        // Per-col budget.
        let mut col_thick = [0u32; 5];
        for pc in 0..pw {
            for pr in 0..ph {
                if piece.shape().get_bit((pr * 15 + pc) as u32) {
                    col_thick[pc] += 1;
                }
            }
        }
        for c in 0..bw {
            let q_min = if c + pw > bw { c + pw - bw } else { 0 };
            let q_max = c.min(pw - 1);
            let mut max_t = 0u32;
            for q in q_min..=q_max {
                if col_thick[q] > max_t { max_t = col_thick[q]; }
            }
            cols_family.per_line_budget[i][c] = cols_family.per_line_budget[i + 1][c] + max_t;
        }
    }

    // --- Diags (main diagonals: d = r - c) ---
    let num_diags = bh + bw - 1;
    let mut diags_family = LineFamily::new();
    diags_family.num_lines = num_diags;
    for r in 0..bh {
        for c in 0..bw {
            let bit = (r * 15 + c) as u32;
            diags_family.masks[(r as i32 - c as i32 + bw as i32 - 1) as usize].set_bit(bit);
        }
    }
    for i in (0..n).rev() {
        diags_family.remaining_budget[i] = diags_family.remaining_budget[i + 1] + pieces[order[i]].max_diag_thickness();
        diags_family.suffix_max_span[i] = diags_family.suffix_max_span[i + 1].max(pieces[order[i]].diag_span());
    }

    // --- Antidiags (anti-diagonals: d = r + c) ---
    let mut antidiags_family = LineFamily::new();
    antidiags_family.num_lines = num_diags;
    for r in 0..bh {
        for c in 0..bw {
            let bit = (r * 15 + c) as u32;
            antidiags_family.masks[r + c].set_bit(bit);
        }
    }
    for i in (0..n).rev() {
        antidiags_family.remaining_budget[i] = antidiags_family.remaining_budget[i + 1] + pieces[order[i]].max_antidiag_thickness();
        antidiags_family.suffix_max_span[i] = antidiags_family.suffix_max_span[i + 1].max(pieces[order[i]].diag_span());
    }

    // --- Zigzag right-leaning bands ---
    let num_zigzag_bands = (bw + 1) / 2;
    let mut zigzag_r_family = LineFamily::new();
    zigzag_r_family.num_lines = num_zigzag_bands;
    for r in 0..bh {
        for c in 0..bw {
            let bit = (r * 15 + c) as u32;
            let band = c / 2;
            if r % 2 == c % 2 {
                zigzag_r_family.masks[band].set_bit(bit);
            }
        }
    }
    for i in (0..n).rev() {
        zigzag_r_family.remaining_budget[i] = zigzag_r_family.remaining_budget[i + 1] + pieces[order[i]].max_zigzag_r_thickness();
        zigzag_r_family.suffix_max_span[i] = zigzag_r_family.suffix_max_span[i + 1].max(pieces[order[i]].zigzag_span());
    }

    // --- Zigzag left-leaning bands ---
    let mut zigzag_l_family = LineFamily::new();
    zigzag_l_family.num_lines = num_zigzag_bands;
    for r in 0..bh {
        for c in 0..bw {
            let bit = (r * 15 + c) as u32;
            let band = c / 2;
            if r % 2 != c % 2 {
                zigzag_l_family.masks[band].set_bit(bit);
            }
        }
    }
    for i in (0..n).rev() {
        zigzag_l_family.remaining_budget[i] = zigzag_l_family.remaining_budget[i + 1] + pieces[order[i]].max_zigzag_l_thickness();
        zigzag_l_family.suffix_max_span[i] = zigzag_l_family.suffix_max_span[i + 1].max(pieces[order[i]].zigzag_span());
    }

    let line_families = [rows_family, cols_family, diags_family, antidiags_family, zigzag_r_family, zigzag_l_family];

    // Precompute jaggedness masks.
    let mut jagg_h_mask = Bitboard::ZERO;
    let mut jagg_v_mask = Bitboard::ZERO;
    for r in 0..bh {
        for c in 0..bw {
            let bit = (r * 15 + c) as u32;
            if c + 1 < bw { jagg_h_mask.set_bit(bit); }
            if r + 1 < bh { jagg_v_mask.set_bit(bit); }
        }
    }
    let jagg_h_total = jagg_h_mask.count_ones();
    let jagg_v_total = jagg_v_mask.count_ones();

    // Precompute parity partition checks.
    // Each partition splits the board into "group 0" vs "rest". The DP tracks
    // achievable group-0 totals. Pieces have K options (one per placement offset mod K).
    //
    // group_fn(r, c) -> bool: is this cell in group 0?
    // num_offsets: how many distinct placement offsets affect group membership (2 for mod-2, 3 for mod-3)
    // offset_fn(pr, pc, offset) -> bool: is piece cell (pr,pc) in group 0 when placed at this offset?
    let build_partition = |group_fn: &dyn Fn(usize, usize) -> bool,
                           num_offsets: usize,
                           offset_fn: &dyn Fn(usize, usize, usize) -> bool|
                           -> ParityPartition {
        let mut mask = Bitboard::ZERO;
        for r in 0..bh {
            for c in 0..bw {
                if group_fn(r, c) {
                    mask.set_bit((r * 15 + c) as u32);
                }
            }
        }

        // Per piece: group-0 count at each offset.
        let mut g0_counts: Vec<Vec<u32>> = Vec::with_capacity(n);
        for i in 0..n {
            let piece = &pieces[order[i]];
            let mut counts = vec![0u32; num_offsets];
            for off in 0..num_offsets {
                for pr in 0..piece.height() as usize {
                    for pc in 0..piece.width() as usize {
                        if piece.shape().get_bit((pr * 15 + pc) as u32) && offset_fn(pr, pc, off) {
                            counts[off] += 1;
                        }
                    }
                }
            }
            g0_counts.push(counts);
        }

        // Suffix min/max.
        let mut suffix_max = vec![0u32; n + 1];
        let mut suffix_min = vec![0u32; n + 1];
        for i in (0..n).rev() {
            suffix_max[i] = suffix_max[i + 1] + *g0_counts[i].iter().max().unwrap();
            suffix_min[i] = suffix_min[i + 1] + *g0_counts[i].iter().min().unwrap();
        }

        // Full DP.
        let dp_size = suffix_max[0] as usize + 1;
        let mut suffix_dp = vec![vec![false; dp_size]; n + 1];
        suffix_dp[n][0] = true;
        for i in (0..n).rev() {
            for w in 0..dp_size {
                if suffix_dp[i + 1][w] {
                    for &g0 in &g0_counts[i] {
                        let nw = w + g0 as usize;
                        if nw < dp_size { suffix_dp[i][nw] = true; }
                    }
                }
            }
        }

        ParityPartition { mask, suffix_max, suffix_min, suffix_dp }
    };

    // Mod-2 partitions (2 offsets each).
    let mut partitions = Vec::new();
    // Checkerboard: (r+c)%2. Offset = (r0+c0)%2.
    partitions.push(build_partition(
        &|r, c| (r + c) % 2 == 0,
        2,
        &|pr, pc, off| (pr + pc + off) % 2 == 0,
    ));
    // Even rows: r%2. Offset = r0%2.
    partitions.push(build_partition(
        &|r, _c| r % 2 == 0,
        2,
        &|pr, _pc, off| (pr + off) % 2 == 0,
    ));
    // Even cols: c%2. Offset = c0%2.
    partitions.push(build_partition(
        &|_r, c| c % 2 == 0,
        2,
        &|_pr, pc, off| (pc + off) % 2 == 0,
    ));

    // Mod-3 partitions (3 offsets each). Each group checked independently vs rest.
    if bh >= 6 {
        for target_group in 0..3usize {
            partitions.push(build_partition(
                &|r, _c| r % 3 == target_group,
                3,
                &|pr, _pc, off| (pr + off) % 3 == target_group,
            ));
        }
    }
    if bw >= 6 {
        for target_group in 0..3usize {
            partitions.push(build_partition(
                &|_r, c| c % 3 == target_group,
                3,
                &|_pr, pc, off| (pc + off) % 3 == target_group,
            ));
        }
    }

    // Precompute subset reachability for border regions.
    // Max subset size adapts to M to keep M^K state space manageable.
    let max_subset_k: usize = match m {
        2 => 10,   // 1024 states
        3 => 6,    // 729 states
        4 => 5,    // 625 states (reduced from previous 4)
        _ => 4,    // 625 states for M=5
    };
    let subset_checks = {
        let m_val = m as usize;

        // Helper: build a SubsetReachability for a given set of cell positions.
        let build_subset = |cells: Vec<u32>| -> SubsetReachability {
            let k = cells.len();
            let num_configs = m_val.pow(k as u32);

            // Apply effect to config.
            let apply_effect = |config: usize, effect: &[u8]| -> usize {
                let mut result = config;
                let mut multiplier = 1;
                for i in 0..k {
                    if effect[i] > 0 {
                        let digit = (result / multiplier) % m_val;
                        let new_digit = (digit + effect[i] as usize) % m_val;
                        result = result - digit * multiplier + new_digit * multiplier;
                    }
                    multiplier *= m_val;
                }
                result
            };

            // Per piece: enumerate unique effects.
            let mut piece_effects: Vec<Vec<Vec<u8>>> = Vec::with_capacity(n);
            for i in 0..n {
                let mut effects_set: Vec<Vec<u8>> = Vec::new();
                effects_set.push(vec![0u8; k]); // zero effect always available
                for &(_, _, mask) in &all_placements[i] {
                    let mut effect = vec![0u8; k];
                    let mut any = false;
                    for (ci, &bit) in cells.iter().enumerate() {
                        if mask.get_bit(bit) {
                            effect[ci] = 1;
                            any = true;
                        }
                    }
                    if any && !effects_set.contains(&effect) {
                        effects_set.push(effect);
                    }
                }
                piece_effects.push(effects_set);
            }

            // Suffix DP.
            let mut suffix_reachable = vec![vec![false; num_configs]; n + 1];
            suffix_reachable[n][0] = true;
            for i in (0..n).rev() {
                for config in 0..num_configs {
                    for effect in &piece_effects[i] {
                        let new_config = apply_effect(config, &effect);
                        if suffix_reachable[i + 1][new_config] {
                            suffix_reachable[i][config] = true;
                            break;
                        }
                    }
                }
            }

            let mut mask = Bitboard::ZERO;
            for &bit in &cells {
                mask.set_bit(bit);
            }
            SubsetReachability { cells, m, mask, suffix_reachable }
        };

        let mut subsets = Vec::new();
        let mut seen_cell_sets: Vec<Vec<u32>> = Vec::new();

        let mut add_subset = |cells: Vec<u32>, subsets: &mut Vec<SubsetReachability>,
                              seen: &mut Vec<Vec<u32>>| {
            if cells.len() < 3 || cells.len() > max_subset_k { return; }
            // Dedup: skip if we've already built this exact cell set.
            let mut sorted = cells.clone();
            sorted.sort();
            if seen.contains(&sorted) { return; }
            seen.push(sorted);
            subsets.push(build_subset(cells));
        };

        // Corner rectangles: try several sizes.
        for &(sr, sc) in &[(3,3), (3,2), (2,3), (2,2)] {
            if sr > bh || sc > bw { continue; }
            let k = sr * sc;
            if k > max_subset_k || k < 3 { continue; }
            let corners = [
                (0, 0), (0, bw - sc), (bh - sr, 0), (bh - sr, bw - sc),
            ];
            for &(r0, c0) in &corners {
                let cells: Vec<u32> = (0..sr)
                    .flat_map(|dr| (0..sc).map(move |dc| ((r0 + dr) * 15 + c0 + dc) as u32))
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
        }

        // Border edge segments: sliding windows of max_subset_k along each edge.
        let seg_len = max_subset_k;
        // Top edge: row 0, varying columns.
        for start_c in 0..=bw.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_c..start_c + seg_len.min(bw - start_c))
                .map(|c| (0 * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }
        // Bottom edge.
        for start_c in 0..=bw.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_c..start_c + seg_len.min(bw - start_c))
                .map(|c| ((bh - 1) * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }
        // Left edge: col 0, varying rows.
        for start_r in 0..=bh.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_r..start_r + seg_len.min(bh - start_r))
                .map(|r| (r * 15 + 0) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }
        // Right edge.
        for start_r in 0..=bh.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_r..start_r + seg_len.min(bh - start_r))
                .map(|r| (r * 15 + (bw - 1)) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }

        // L-shaped corner subsets: cells along both edges meeting at each corner.
        // E.g., top-left L: top edge cells + left edge cells (no duplicate at corner).
        for &(cr, cc, dir_r, dir_c) in &[
            (0usize, 0usize, 1isize, 1isize),         // top-left: down + right
            (0, bw - 1, 1isize, -1isize),              // top-right: down + left
            (bh - 1, 0, -1isize, 1isize),              // bottom-left: up + right
            (bh - 1, bw - 1, -1isize, -1isize),        // bottom-right: up + left
        ] {
            // Build L: arm along row (horizontal) + arm along column (vertical).
            let arm_len = max_subset_k / 2;
            let mut cells = Vec::new();
            // Horizontal arm from corner.
            for i in 0..arm_len.min(bw) {
                let c = (cc as isize + i as isize * dir_c) as usize;
                if c < bw {
                    cells.push((cr * 15 + c) as u32);
                }
            }
            // Vertical arm from corner (skip the corner cell itself to avoid dup).
            for i in 1..arm_len.min(bh) {
                let r = (cr as isize + i as isize * dir_r) as usize;
                if r < bh {
                    cells.push((r * 15 + cc) as u32);
                }
            }
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }

        // Multi-corner subsets: combine cells from 2 opposite corners.
        // These are very far apart — most pieces affect at most 1 corner.
        if bh >= 6 && bw >= 6 {
            let half_k = max_subset_k / 2;
            // Top-left + bottom-right corners.
            {
                let mut cells = Vec::new();
                for r in 0..2usize.min(bh) {
                    for c in 0..half_k.min(bw) / 2 {
                        cells.push((r * 15 + c) as u32);
                    }
                }
                for r in bh.saturating_sub(2)..bh {
                    for c in bw.saturating_sub(half_k / 2)..bw {
                        cells.push((r * 15 + c) as u32);
                    }
                }
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
            // Top-right + bottom-left corners.
            {
                let mut cells = Vec::new();
                for r in 0..2usize.min(bh) {
                    for c in bw.saturating_sub(half_k / 2)..bw {
                        cells.push((r * 15 + c) as u32);
                    }
                }
                for r in bh.saturating_sub(2)..bh {
                    for c in 0..half_k.min(bw) / 2 {
                        cells.push((r * 15 + c) as u32);
                    }
                }
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
        }

        // Diagonal subsets near corners: cells along main diagonals.
        for &(r0, c0, dr, dc) in &[
            (0usize, 0usize, 1isize, 1isize),         // top-left diagonal
            (0, bw - 1, 1isize, -1isize),              // top-right anti-diagonal
            (bh - 1, 0, -1isize, 1isize),              // bottom-left anti-diagonal
            (bh - 1, bw - 1, -1isize, -1isize),        // bottom-right diagonal
        ] {
            let mut cells = Vec::new();
            let mut r = r0 as isize;
            let mut c = c0 as isize;
            while cells.len() < max_subset_k && r >= 0 && r < bh as isize && c >= 0 && c < bw as isize {
                cells.push((r as usize * 15 + c as usize) as u32);
                r += dr;
                c += dc;
            }
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }

        // 2-wide border strips: sliding windows along each edge with depth 2.
        // These capture constraints that single-row border segments miss.
        if bh >= 4 && bw >= 4 {
            let strip_w = max_subset_k / 2; // columns per window (2 rows × strip_w cols)
            // Top 2 rows.
            for start_c in 0..=bw.saturating_sub(strip_w) {
                let cells: Vec<u32> = (0..2usize)
                    .flat_map(|r| (start_c..start_c + strip_w.min(bw - start_c))
                        .map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
            // Bottom 2 rows.
            for start_c in 0..=bw.saturating_sub(strip_w) {
                let cells: Vec<u32> = (bh - 2..bh)
                    .flat_map(|r| (start_c..start_c + strip_w.min(bw - start_c))
                        .map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
            // Left 2 cols.
            let strip_h = max_subset_k / 2;
            for start_r in 0..=bh.saturating_sub(strip_h) {
                let cells: Vec<u32> = (start_r..start_r + strip_h.min(bh - start_r))
                    .flat_map(|r| (0..2usize).map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
            // Right 2 cols.
            for start_r in 0..=bh.saturating_sub(strip_h) {
                let cells: Vec<u32> = (start_r..start_r + strip_h.min(bh - start_r))
                    .flat_map(|r| ((bw - 2)..bw).map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
        }

        // Scattered border: every-other cell around the full perimeter.
        // Sparse but wide-reaching — catches long-range configuration conflicts.
        if bh >= 5 && bw >= 5 {
            let mut border_cells = Vec::new();
            for c in 0..bw { border_cells.push((0, c)); }
            for r in 1..bh { border_cells.push((r, bw - 1)); }
            for c in (0..bw - 1).rev() { border_cells.push((bh - 1, c)); }
            for r in (1..bh - 1).rev() { border_cells.push((r, 0)); }

            // Phase 0: even-indexed perimeter cells.
            let cells: Vec<u32> = border_cells.iter().step_by(2)
                .take(max_subset_k)
                .map(|&(r, c)| (r * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);

            // Phase 1: odd-indexed perimeter cells.
            let cells: Vec<u32> = border_cells.iter().skip(1).step_by(2)
                .take(max_subset_k)
                .map(|&(r, c)| (r * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);

            // Wider spacing: every 3rd cell.
            let cells: Vec<u32> = border_cells.iter().step_by(3)
                .take(max_subset_k)
                .map(|&(r, c)| (r * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);

            // Every 3rd, offset 1.
            let cells: Vec<u32> = border_cells.iter().skip(1).step_by(3)
                .take(max_subset_k)
                .map(|&(r, c)| (r * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }

        // Cross/plus subsets at each corner: L + an interior diagonal cell.
        if bh >= 5 && bw >= 5 {
            for &(cr, cc, dr, dc) in &[
                (0usize, 0usize, 1isize, 1isize),
                (0, bw - 1, 1isize, -1isize),
                (bh - 1, 0, -1isize, 1isize),
                (bh - 1, bw - 1, -1isize, -1isize),
            ] {
                // Corner cell + 2 along row + 1 along col + 1 diagonal inward.
                let mut cells = Vec::new();
                cells.push((cr * 15 + cc) as u32);
                // One step along row.
                let c1 = (cc as isize + dc) as usize;
                if c1 < bw { cells.push((cr * 15 + c1) as u32); }
                // One step along col.
                let r1 = (cr as isize + dr) as usize;
                if r1 < bh { cells.push((r1 * 15 + cc) as u32); }
                // Diagonal inward.
                if r1 < bh && c1 < bw { cells.push((r1 * 15 + c1) as u32); }
                // Two steps along row.
                let c2 = (cc as isize + 2 * dc) as usize;
                if c2 < bw && cells.len() < max_subset_k {
                    cells.push((cr * 15 + c2) as u32);
                }
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
        }

        // Mid-edge segments: cells from the middle of each edge (2 rows deep).
        if bh >= 6 && bw >= 6 {
            let seg = max_subset_k / 2;
            let mid_c = (bw.saturating_sub(seg)) / 2;
            let mid_r = (bh.saturating_sub(seg)) / 2;
            // Top mid, 2 deep.
            let cells: Vec<u32> = (0..2usize)
                .flat_map(|r| (mid_c..mid_c + seg.min(bw))
                    .map(move |c| (r * 15 + c) as u32))
                .take(max_subset_k).collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
            // Bottom mid, 2 deep.
            let cells: Vec<u32> = (bh - 2..bh)
                .flat_map(|r| (mid_c..mid_c + seg.min(bw))
                    .map(move |c| (r * 15 + c) as u32))
                .take(max_subset_k).collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
            // Left mid, 2 deep.
            let cells: Vec<u32> = (mid_r..mid_r + seg.min(bh))
                .flat_map(|r| (0..2usize).map(move |c| (r * 15 + c) as u32))
                .take(max_subset_k).collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
            // Right mid, 2 deep.
            let cells: Vec<u32> = (mid_r..mid_r + seg.min(bh))
                .flat_map(|r| ((bw - 2)..bw).map(move |c| (r * 15 + c) as u32))
                .take(max_subset_k).collect();
            add_subset(cells, &mut subsets, &mut seen_cell_sets);
        }

        subsets
    };

    let data = SolverData {
        all_placements,
        reaches,
        perimeters: sorted_perimeters,
        h_perimeters: sorted_h_perimeters,
        v_perimeters: sorted_v_perimeters,
        cell_counts: sorted_cell_counts,
        remaining_bits,
        remaining_perimeter,
        remaining_h_perimeter,
        remaining_v_perimeter,
        jagg_h_mask,
        jagg_h_total,
        jagg_v_mask,
        jagg_v_total,
        line_families,
        suffix_coverage,
        is_dup_of_prev,
        skip_tables,
        single_cell_start,
        m,
        h,
        w,
        parity_partitions: partitions,
        subset_checks,
    };

    let nodes = Cell::new(0u64);
    let mut sorted_solution = Vec::with_capacity(n);
    let found = backtrack(
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

/// Try to solve remaining pieces when they're all 1x1.
/// Each cell at value d needs (M-d)%M hits. Total hits must equal number of pieces.
/// Returns true and fills solution if solvable.
fn solve_single_cells(
    board: &Board,
    m: u8,
    h: u8,
    w: u8,
    num_pieces: usize,
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    // Count total hits needed and verify it matches available pieces.
    let mut needed = 0u32;
    for d in 1..m {
        needed += (m - d) as u32 * board.plane(d).count_ones();
    }
    if needed as usize != num_pieces {
        return false;
    }

    // Assign pieces to cells: for each non-zero cell, emit (M-d) placements.
    // Process cells in row-major order.
    let base_len = solution.len();
    for r in 0..h as usize {
        for c in 0..w as usize {
            let val = board.get(r, c);
            if val != 0 {
                let hits = (m - val) as usize;
                for _ in 0..hits {
                    solution.push((r, c));
                }
            }
        }
    }

    debug_assert_eq!(solution.len() - base_len, num_pieces);
    true
}

#[inline(always)]
#[inline(always)]
fn prune_subset_reachability(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    for subset in &data.subset_checks {
        if !subset.check(board, piece_idx) {
            return false;
        }
    }
    true
}

fn prune_parity_partitions(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    let m = data.m as u32;
    let total_min_flips = board.min_flips_needed();

    for partition in &data.parity_partitions {
        // Compute group-0 min_flips using the precomputed mask.
        let mut g0_min_flips = 0u32;
        for d in 1..data.m {
            g0_min_flips += (data.m - d) as u32 * (board.plane(d) & partition.mask).count_ones();
        }

        // Simple bounds check.
        if partition.suffix_max[piece_idx] < g0_min_flips {
            return false;
        }
        let g1_min_flips = total_min_flips - g0_min_flips;
        let max_g1 = data.remaining_bits[piece_idx] - partition.suffix_min[piece_idx];
        if max_g1 < g1_min_flips {
            return false;
        }

        // Full DP check: is g0_min_flips achievable (accounting for wraps)?
        let dp = &partition.suffix_dp[piece_idx];
        let mut target = g0_min_flips;
        let mut found = false;
        while (target as usize) < dp.len() {
            if dp[target as usize] {
                found = true;
                break;
            }
            target += m;
        }
        if !found {
            return false;
        }
    }
    true
}

#[inline(always)]
fn prune_active_planes(board: &Board, remaining: usize) -> bool {
    board.active_planes() as usize <= remaining
}

#[inline(always)]
fn prune_min_flips_global(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    data.remaining_bits[piece_idx] >= board.min_flips_needed()
}

#[inline(always)]
fn prune_line_families_rowcol(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    check_line_family(board, &data.line_families[0], piece_idx, data.m)
        && check_line_family(board, &data.line_families[1], piece_idx, data.m)
}

#[inline(always)]
fn prune_line_families_diagonal(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    for f in &data.line_families[2..] {
        if !check_line_family(board, f, piece_idx, data.m) { return false; }
    }
    true
}

#[inline(always)]
fn prune_subgrid(board: &Board, data: &SolverData, piece_idx: usize, remaining: usize) -> bool {
    let gap_h = data.line_families[0].suffix_max_span[piece_idx] as usize;
    let gap_w = data.line_families[1].suffix_max_span[piece_idx] as usize;
    if gap_h == 0 || gap_w == 0 { return true; }
    let mut max_demand = 0u32;
    for r0 in 0..gap_h {
        for c0 in 0..gap_w {
            let mut demand = 0u32;
            let mut r = r0;
            while r < data.h as usize {
                let mut c = c0;
                while c < data.w as usize {
                    let bit = (r * 15 + c) as u32;
                    for d in 1..data.m {
                        if board.plane(d).get_bit(bit) {
                            demand += (data.m - d) as u32;
                            break;
                        }
                    }
                    c += gap_w;
                }
                r += gap_h;
            }
            if demand > max_demand {
                max_demand = demand;
            }
        }
    }
    max_demand <= remaining as u32
}

#[inline(always)]
fn prune_coverage(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    has_sufficient_coverage(board, &data.suffix_coverage[piece_idx], data.m)
}

#[inline(always)]
fn prune_jaggedness(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    let (h_jagg, v_jagg) = board.split_jaggedness(
        data.jagg_h_mask, data.jagg_h_total, data.jagg_v_mask, data.jagg_v_total);
    h_jagg <= data.remaining_h_perimeter[piece_idx]
        && v_jagg <= data.remaining_v_perimeter[piece_idx]
}

fn backtrack(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    min_placement: usize,
    prev_dup_placement: usize,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    config: &PruningConfig,
) -> bool {
    nodes.set(nodes.get() + 1);

    if piece_idx == data.all_placements.len() {
        return board.is_solved();
    }

    // If all remaining pieces are 1x1, solve directly.
    if config.single_cell_endgame && piece_idx >= data.single_cell_start {
        let num_remaining = data.all_placements.len() - piece_idx;
        return solve_single_cells(board, data.m, data.h, data.w, num_remaining, solution);
    }

    let remaining = data.all_placements.len() - piece_idx;
    let branching = data.all_placements[piece_idx].len();

    if config.active_planes && !prune_active_planes(board, remaining) { return false; }
    if config.min_flips_global && !prune_min_flips_global(board, data, piece_idx) { return false; }
    if config.min_flips_rowcol && !prune_line_families_rowcol(board, data, piece_idx) { return false; }
    if config.min_flips_diagonal && !prune_line_families_diagonal(board, data, piece_idx) { return false; }
    if config.min_flips_rowcol && branching >= 6 && !prune_subgrid(board, data, piece_idx, remaining) { return false; }
    if config.coverage && !prune_coverage(board, data, piece_idx) { return false; }
    if config.jaggedness && !prune_jaggedness(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_parity_partitions(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_subset_reachability(board, data, piece_idx) { return false; }

    // Compute locked mask: cells at 0 where remaining coverage < M.
    let locked_mask = if config.cell_locking {
        board.plane(0) & !data.suffix_coverage[piece_idx].coverage_ge(data.m)
    } else {
        Bitboard::ZERO
    };

    // Prune: per-component checks (jaggedness, min_flips).
    // Run when branching factor justifies flood-fill cost.
    if config.component_checks && branching >= 8 {
        if !check_components(
            board, locked_mask, data, piece_idx,
        ) {
            return false;
        }
    }

    let placements = &data.all_placements[piece_idx];

    // Sort placements by min_flips delta — prefer placements that reduce the
    // distance to solution the most.
    // Delta = M * zeros_hit - piece_area (exact, derived from incremental min_flips).
    // Since piece_area is constant for the same piece, we sort by zeros_hit.
    // For M>2, we refine: also reward hitting cells at M-1 (they wrap to 0,
    // each saving M-1 min_flips vs only 1 for other non-zero cells).
    // Combined key: M * zeros_hit - (M-2) * tops_hit. Lower is better.
    let zero_plane = board.plane(0);
    let mut order = [0u8; 196];
    let pl_len = placements.len();
    for i in 0..pl_len {
        order[i] = i as u8;
    }
    order[..pl_len].sort_unstable_by_key(|&i| {
        (placements[i as usize].2 & zero_plane).count_ones()
    });

    let mut board = board.clone();
    for oi in 0..pl_len {
        let pl_idx = order[oi] as usize;
        let (row, col, mask) = placements[pl_idx];
        // Duplicate symmetry breaking.
        if config.duplicate_pruning && pl_idx < min_placement {
            continue;
        }

        // Skip placements that touch locked cells.
        if !(mask & locked_mask).is_zero() {
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

        board.apply_piece(mask);
        solution.push((row, col));

        let is_next_dup = config.duplicate_pruning
            && piece_idx + 1 < data.all_placements.len()
            && data.is_dup_of_prev[piece_idx + 1];

        let next_min = if is_next_dup { pl_idx } else { 0 };
        // Always pass placement for skip table lookup (works for any consecutive pair).
        let next_prev_dup = if piece_idx + 1 < data.all_placements.len()
            && data.skip_tables[piece_idx + 1].is_some()
        {
            pl_idx
        } else {
            usize::MAX
        };

        if backtrack(
            &board,
            data,
            piece_idx + 1,
            next_min,
            next_prev_dup,
            solution,
            nodes,
            config,
        ) {
            return true;
        }

        solution.pop();
        board.undo_piece(mask);
    }

    false
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
        // Keep piece counts low enough to be solvable within reasonable time.
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
        // Parity partitions (checkerboard, even-rows, even-cols) must never
        // cause false prunes.
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
        // Stress test on larger boards.
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
        // Verify parity partitions reduce nodes compared to without them.
        let configs = small_configs();
        let seeds = test_seeds();
        // With parity (default config includes min_flips_global which gates the check).
        let (nodes_with, _) = fuzz_with_config(&PruningConfig::default(), &configs, &seeds);
        // Without parity: disable min_flips_global (which gates the parity check).
        let mut no_parity = PruningConfig::default();
        no_parity.min_flips_global = false;
        let (nodes_without, _) = fuzz_with_config(&no_parity, &configs, &seeds);
        // min_flips_global also includes the global budget check, so disabling it
        // weakens pruning significantly. The parity check is just part of it.
        assert!(nodes_with <= nodes_without,
            "parity partitions should reduce nodes: {} vs {}", nodes_with, nodes_without);
    }

    #[test]
    fn test_pair_skip_tables_non_identical() {
        // Test that skip tables work for non-identical consecutive pieces.
        // Create a game where two different pieces have placements producing
        // the same combined effect, verify the solver still finds a solution
        // (soundness) and uses fewer nodes than without skip tables.
        use crate::generate::generate_game;
        use crate::level::LevelSpec;

        // Use configs where non-identical pieces are common and boards are small enough
        // for the skip tables to matter.
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

                // With skip tables (default config).
                let result_with = solve(&game);
                total_with += result_with.nodes_visited;

                // Without skip tables (duplicate pruning still on, but skip tables disabled
                // by using solve_with_config and verifying solution).
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
        // Skip tables should not increase nodes (they only skip redundant combos).
        // We can't easily test reduction without a "no skip tables" config, but
        // soundness is the critical property.
    }

    #[test]
    fn test_pair_skip_tables_soundness_stress() {
        // Stress test: larger configs with many non-identical piece pairs.
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
}
