use std::cell::Cell;

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::coverage::{has_sufficient_coverage, precompute_suffix_coverage, CoverageCounter};
use crate::game::Game;

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
    pub min_flips: bool,
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
            min_flips: true,
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
            min_flips: false,
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
    reaches: &[Bitboard],
    perimeters: &[u32],
    cell_counts: &[u32],
    m: u8,
    piece_idx: usize,
) -> bool {

    // Non-zero region, excluding locked cells (which are walls).
    let mut nz = Bitboard::ZERO;
    for d in 1..m {
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
        for d in 1..m {
            comp_min_flips += (m - d) as u32 * (board.plane(d) & component).count_ones();
        }

        // Component jaggedness.
        let h_pairs = component & (component >> 1);
        let v_pairs = component & (component >> 15);
        let total_pairs = h_pairs.count_ones() + v_pairs.count_ones();
        let mut matching = 0u32;
        for d in 0..m {
            let p = board.plane(d) & component;
            matching += (p & (p >> 1) & h_pairs).count_ones();
            matching += (p & (p >> 15) & v_pairs).count_ones();
        }
        let comp_jaggedness = total_pairs - matching;

        // Sum perimeter and cell_counts of remaining pieces that can reach this component.
        let mut reachable_perimeter = 0u32;
        let mut reachable_bits = 0u32;
        for pi in piece_idx..reaches.len() {
            if !(reaches[pi] & component).is_zero() {
                reachable_perimeter += perimeters[pi];
                reachable_bits += cell_counts[pi];
            }
        }

        // Per-component pruning checks.
        if comp_jaggedness > reachable_perimeter {
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

/// Try solving a reduced puzzle by removing cancellable groups of M identical pieces.
/// For M=2: pairs cancel. For M=3: triples cancel. Etc.
/// If the reduced puzzle is solvable, reconstruct the full solution.
/// Otherwise, fall back to solving the full puzzle.
fn solve_with_cancellation(game: &Game, config: &PruningConfig) -> SolveResult {
    let m = game.board().m() as usize;
    let pieces = game.pieces();

    // Count pieces per shape, preserving original indices.
    let mut shape_groups: Vec<(crate::piece::Piece, Vec<usize>)> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        if let Some(group) = shape_groups.iter_mut().find(|(s, _)| s == piece) {
            group.1.push(i);
        } else {
            shape_groups.push((*piece, vec![i]));
        }
    }

    // Check if any group has M+ pieces (cancellable).
    let has_cancellable = shape_groups.iter().any(|(_, indices)| indices.len() >= m);
    if !has_cancellable {
        return solve_with_config(game, config);
    }

    // Build reduced piece list: keep K % M pieces per group.
    let mut kept_indices: Vec<usize> = Vec::new();
    let mut cancelled_groups: Vec<(crate::piece::Piece, Vec<usize>)> = Vec::new();

    for (shape, indices) in &shape_groups {
        let keep = indices.len() % m;
        let cancel = indices.len() - keep;
        for &idx in &indices[..keep] {
            kept_indices.push(idx);
        }
        if cancel > 0 {
            cancelled_groups.push((*shape, indices[keep..].to_vec()));
        }
    }

    if cancelled_groups.is_empty() {
        return solve_with_config(game, config);
    }

    // Build reduced game.
    let reduced_pieces: Vec<crate::piece::Piece> = kept_indices.iter().map(|&i| pieces[i]).collect();

    if reduced_pieces.is_empty() {
        // All pieces cancel — check if board is already solved.
        if game.board().is_solved() {
            let mut solution = vec![(0usize, 0usize); pieces.len()];
            let h = game.board().height();
            let w = game.board().width();
            for (shape, indices) in &cancelled_groups {
                let placements = shape.placements(h, w);
                if let Some(&(r, c, _)) = placements.first() {
                    for &idx in indices {
                        solution[idx] = (r, c);
                    }
                }
            }
            return SolveResult {
                solution: Some(solution),
                nodes_visited: 1,
            };
        }
        // Board not solved and no pieces left in reduced — fall back to full solve.
        return solve_with_config(game, config);
    }

    let reduced_game = Game::new(game.board().clone(), reduced_pieces);
    let mut reduced_result = solve_with_config(&reduced_game, config);

    if let Some(ref reduced_sol) = reduced_result.solution {
        // Reconstruct full solution.
        let mut full_solution = vec![(0usize, 0usize); pieces.len()];

        // Map reduced solution back to original indices.
        for (reduced_idx, &(row, col)) in reduced_sol.iter().enumerate() {
            let orig_idx = kept_indices[reduced_idx];
            full_solution[orig_idx] = (row, col);
        }

        // Assign cancelled pieces: groups of M at the same valid position.
        let h = game.board().height();
        let w = game.board().width();
        for (shape, indices) in &cancelled_groups {
            let placements = shape.placements(h, w);
            if let Some(&(r, c, _)) = placements.first() {
                for &idx in indices {
                    full_solution[idx] = (r, c);
                }
            }
        }

        return SolveResult {
            solution: Some(full_solution),
            nodes_visited: reduced_result.nodes_visited,
        };
    }

    // Reduced puzzle failed — solve the full puzzle.
    let mut full_result = solve_with_config(game, config);
    full_result.nodes_visited += reduced_result.nodes_visited;
    full_result
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

    // Find where trailing 1x1 pieces start (they're sorted last = most placements).
    let single_cell_start = (0..n)
        .rposition(|i| pieces[order[i]].cell_count() != 1)
        .map(|i| i + 1)
        .unwrap_or(0);

    // Precompute suffix sums/maxes of piece properties.
    let mut remaining_bits = vec![0u32; n + 1];
    let mut remaining_perimeter = vec![0u32; n + 1];
    let mut remaining_max_row_thick = vec![0u32; n + 1];
    let mut remaining_max_col_thick = vec![0u32; n + 1];
    let mut suffix_max_height = vec![0u8; n + 1]; // max piece height among remaining
    let mut suffix_max_width = vec![0u8; n + 1];  // max piece width among remaining
    for i in (0..n).rev() {
        remaining_bits[i] = remaining_bits[i + 1] + pieces[order[i]].cell_count();
        remaining_perimeter[i] = remaining_perimeter[i + 1] + pieces[order[i]].perimeter();
        remaining_max_row_thick[i] = remaining_max_row_thick[i + 1] + pieces[order[i]].max_row_thickness();
        remaining_max_col_thick[i] = remaining_max_col_thick[i + 1] + pieces[order[i]].max_col_thickness();
        suffix_max_height[i] = suffix_max_height[i + 1].max(pieces[order[i]].height());
        suffix_max_width[i] = suffix_max_width[i + 1].max(pieces[order[i]].width());
    }

    // Precompute per-row and per-col suffix budgets.
    // For each board row r and piece i, the max cells piece i can deliver to row r
    // depends on which piece-rows can align with board-row r.
    let bh = h as usize;
    let bw = w as usize;
    let mut row_budget = vec![vec![0u32; bh]; n + 1]; // row_budget[piece_idx][row]
    let mut col_budget = vec![vec![0u32; bw]; n + 1]; // col_budget[piece_idx][col]
    for i in (0..n).rev() {
        let piece = &pieces[order[i]];
        let ph = piece.height() as usize;
        let pw = piece.width() as usize;

        // Compute row thicknesses for this piece.
        let mut row_thick = [0u32; 5];
        for pr in 0..ph {
            let row_bits = (piece.shape() >> (pr as u32 * 15)).limbs[0] & ((1u64 << pw) - 1);
            row_thick[pr] = row_bits.count_ones();
        }

        // Compute col thicknesses for this piece.
        let mut col_thick = [0u32; 5];
        for pc in 0..pw {
            for pr in 0..ph {
                if piece.shape().get_bit((pr * 15 + pc) as u32) {
                    col_thick[pc] += 1;
                }
            }
        }

        for r in 0..bh {
            // Which piece-rows can land on board-row r?
            let p_min = if r + ph > bh { r + ph - bh } else { 0 };
            let p_max = r.min(ph - 1);
            let mut max_t = 0u32;
            for p in p_min..=p_max {
                if row_thick[p] > max_t {
                    max_t = row_thick[p];
                }
            }
            row_budget[i][r] = row_budget[i + 1][r] + max_t;
        }

        for c in 0..bw {
            let q_min = if c + pw > bw { c + pw - bw } else { 0 };
            let q_max = c.min(pw - 1);
            let mut max_t = 0u32;
            for q in q_min..=q_max {
                if col_thick[q] > max_t {
                    max_t = col_thick[q];
                }
            }
            col_budget[i][c] = col_budget[i + 1][c] + max_t;
        }
    }

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
    let sorted_cell_counts: Vec<u32> = (0..n).map(|i| pieces[order[i]].cell_count()).collect();

    let m = board.m();

    // Precompute column masks for per-col budget checks.
    let mut col_masks = vec![Bitboard::ZERO; bw];
    for c in 0..bw {
        for r in 0..bh {
            col_masks[c].set_bit((r * 15 + c) as u32);
        }
    }

    let nodes = Cell::new(0u64);
    let mut sorted_solution = Vec::with_capacity(n);
    let found = backtrack(
        &board,
        &all_placements,
        &reaches,
        &sorted_perimeters,
        &sorted_cell_counts,
        &remaining_bits,
        &remaining_perimeter,
        &remaining_max_row_thick,
        &remaining_max_col_thick,
        &suffix_max_height,
        &suffix_max_width,
        &row_budget,
        &col_budget,
        &col_masks,
        &suffix_coverage,
        &is_dup_of_prev,
        m,
        h,
        w,
        single_cell_start,
        0,
        0,
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

fn backtrack(
    board: &Board,
    all_placements: &[Vec<(usize, usize, Bitboard)>],
    reaches: &[Bitboard],
    perimeters: &[u32],
    cell_counts: &[u32],
    remaining_bits: &[u32],
    remaining_perimeter: &[u32],
    remaining_max_row_thick: &[u32],
    remaining_max_col_thick: &[u32],
    suffix_max_height: &[u8],
    suffix_max_width: &[u8],
    row_budget: &[Vec<u32>],
    col_budget: &[Vec<u32>],
    col_masks: &[Bitboard],
    suffix_coverage: &[CoverageCounter],
    is_dup_of_prev: &[bool],
    m: u8,
    h: u8,
    w: u8,
    single_cell_start: usize,
    piece_idx: usize,
    min_placement: usize,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    config: &PruningConfig,
) -> bool {
    nodes.set(nodes.get() + 1);

    if piece_idx == all_placements.len() {
        return board.is_solved();
    }

    // If all remaining pieces are 1x1, solve directly.
    if config.single_cell_endgame && piece_idx >= single_cell_start {
        let num_remaining = all_placements.len() - piece_idx;
        return solve_single_cells(board, m, h, w, num_remaining, solution);
    }

    let remaining = all_placements.len() - piece_idx;

    // Prune: each piece can eliminate at most one active plane.
    if config.active_planes && board.active_planes() as usize > remaining {
        return false;
    }

    // Prune: if remaining piece bits can't cover the minimum flips needed.
    let min_flips = board.min_flips_needed();
    if config.min_flips && remaining_bits[piece_idx] < min_flips {
        return false;
    }

    // Prune: per-row/col min_flips budget.
    // Each row/col's needed flips must be achievable by the remaining pieces'
    // max contribution to that specific row/col (accounting for piece shape and position).
    // Additionally, use DP to find the max-weight independent set of rows/cols
    // spaced ≥ 5 apart (no piece can serve two), and check against global thickness budget.
    if config.min_flips {
        let row_mask_base = (1u64 << w) - 1;
        let mut row_weights = [0u32; 14];
        for r in 0..h as usize {
            for d in 1..m {
                let plane_row = (board.plane(d) >> (r as u32 * 15)).limbs[0] & row_mask_base;
                row_weights[r] += (m - d) as u32 * plane_row.count_ones();
            }
            // Per-row position-aware check.
            if row_budget[piece_idx][r] < row_weights[r] {
                return false;
            }
        }

        let mut col_weights = [0u32; 14];
        for c in 0..w as usize {
            for d in 1..m {
                col_weights[c] += (m - d) as u32 * (board.plane(d) & col_masks[c]).count_ones();
            }
            // Per-col position-aware check.
            if col_budget[piece_idx][c] < col_weights[c] {
                return false;
            }
        }

        // DP: max weight independent set of rows with spacing >= max_piece_height.
        // Tighter spacing when remaining pieces are short.
        {
            let gap = suffix_max_height[piece_idx] as usize;
            if gap > 0 {
                let mut dp = [0u32; 14];
                for r in 0..h as usize {
                    let take = row_weights[r] + if r >= gap { dp[r - gap] } else { 0 };
                    let skip = if r > 0 { dp[r - 1] } else { 0 };
                    dp[r] = take.max(skip);
                }
                if remaining_max_row_thick[piece_idx] < dp[h as usize - 1] {
                    return false;
                }
            }
        }

        // DP: max weight independent set of columns with spacing >= max_piece_width.
        {
            let gap = suffix_max_width[piece_idx] as usize;
            if gap > 0 {
                let mut dp = [0u32; 14];
                for c in 0..w as usize {
                    let take = col_weights[c] + if c >= gap { dp[c - gap] } else { 0 };
                    let skip = if c > 0 { dp[c - 1] } else { 0 };
                    dp[c] = take.max(skip);
                }
                if remaining_max_col_thick[piece_idx] < dp[w as usize - 1] {
                    return false;
                }
            }
        }
    }

    // Prune: insufficient coverage per cell.
    if config.coverage && !has_sufficient_coverage(board, &suffix_coverage[piece_idx], m) {
        return false;
    }

    // Prune: jaggedness exceeds total remaining perimeter.
    if config.jaggedness && board.jaggedness() > remaining_perimeter[piece_idx] {
        return false;
    }

    // Compute locked mask: cells at 0 where remaining coverage < M.
    let locked_mask = if config.cell_locking {
        board.plane(0) & !suffix_coverage[piece_idx].coverage_ge(m)
    } else {
        Bitboard::ZERO
    };

    // Prune: per-component checks (jaggedness, min_flips).
    if config.component_checks && piece_idx < 4 {
        if !check_components(
            board, locked_mask, reaches, perimeters, cell_counts,
            m, piece_idx,
        ) {
            return false;
        }
    }

    let placements = &all_placements[piece_idx];
    let mut board = board.clone();
    for (pl_idx, &(row, col, mask)) in placements.iter().enumerate() {
        // Duplicate symmetry breaking.
        if config.duplicate_pruning && pl_idx < min_placement {
            continue;
        }

        // Skip placements that touch locked cells.
        if !(mask & locked_mask).is_zero() {
            continue;
        }

        board.apply_piece(mask);
        solution.push((row, col));

        let next_min = if config.duplicate_pruning
            && piece_idx + 1 < all_placements.len()
            && is_dup_of_prev[piece_idx + 1]
        {
            pl_idx
        } else {
            0
        };

        if backtrack(
            &board,
            all_placements,
            reaches,
            perimeters,
            cell_counts,
            remaining_bits,
            remaining_perimeter,
            remaining_max_row_thick,
            remaining_max_col_thick,
            suffix_max_height,
            suffix_max_width,
            row_budget,
            col_budget,
            col_masks,
            suffix_coverage,
            is_dup_of_prev,
            m,
            h,
            w,
            single_cell_start,
            piece_idx + 1,
            next_min,
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
    fn test_prune_min_flips() {
        let configs = small_configs();
        let seeds = test_seeds();
        let no_prune = PruningConfig::none();
        let with_prune = PruningConfig::none().only(|c| c.min_flips = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, fail_with) = fuzz_with_config(&with_prune, &configs, &seeds);

        assert_eq!(fail_with, 0, "min_flips prune caused failures");
        assert!(nodes_with <= nodes_without,
            "min_flips should reduce nodes: {} vs {}", nodes_with, nodes_without);
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
}
