use std::simd::{u16x16, cmp::SimdOrd, num::SimdUint};
use std::time::Instant;

use super::game::SubgameGame;

/// Result of a subgame solve attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgameSolveResult {
    /// A solution was found. Contains the placement positions (one per piece).
    Solved(Vec<usize>),
    /// No solution exists.
    Unsolvable,
    /// Solver exceeded its deadline without completing.
    Timeout,
}

/// Statistics tracked during solving.
#[derive(Debug, Clone, Default)]
pub struct SolverStats {
    /// Total number of nodes visited in the search tree.
    pub nodes_visited: u64,
    /// Number of branches pruned by total deficit check.
    pub deficit_prunes: u64,
    /// Number of branches pruned by per-cell count satisfiability check.
    pub count_sat_prunes: u64,
    /// Number of branches pruned by parity partition check.
    pub parity_prunes: u64,
    /// Number of branches pruned by subset satisfiability (mod-M reachability).
    pub subset_sat_prunes: u64,
}

/// Configuration for subgame solver pruning techniques.
#[derive(Debug, Clone, Copy)]
pub struct SubgamePruningConfig {
    /// Total deficit check: remaining piece cells must equal remaining deficit.
    pub total_deficit: bool,
    /// Per-cell count satisfiability: max contribution from remaining pieces
    /// must cover each cell's deficit.
    pub count_sat: bool,
    /// Single-cell endgame: skip backtracking when all remaining pieces are [1].
    pub single_cell_endgame: bool,
    /// Parity partition: split cells into even/odd groups, check if group
    /// deficits are reachable via suffix DP.
    pub parity: bool,
    /// Subset satisfiability: mod-M reachability DP on small cell subsets.
    pub subset_sat: bool,
}

impl Default for SubgamePruningConfig {
    fn default() -> Self {
        Self {
            total_deficit: true,
            count_sat: true,
            single_cell_endgame: true,
            parity: true,
            subset_sat: true,
        }
    }
}

impl SubgamePruningConfig {
    /// All pruning disabled.
    pub fn none() -> Self {
        Self {
            total_deficit: false,
            count_sat: false,
            single_cell_endgame: false,
            parity: false,
            subset_sat: false,
        }
    }

    /// Enable only the specified pruning options.
    pub fn only(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }
}

/// A single parity partition for 1D subgame pruning.
///
/// Splits cells into two groups by a function of position index.
/// Precomputes a suffix DP of reachable group-0 contribution totals.
struct SubgameParityPartition {
    /// Which cells are in group 0 (indexed by cell position).
    group0: [bool; 16],
    /// suffix_dp[depth] = bitset of reachable group-0 totals from pieces depth..n.
    /// Indexed as suffix_dp[depth][total].
    suffix_dp: Vec<Vec<bool>>,
    /// suffix_max[depth] = max achievable group-0 total from pieces depth..n.
    suffix_max: Vec<u32>,
}

impl SubgameParityPartition {
    /// Build a partition from a group membership function and piece placements.
    fn build(
        game: &SubgameGame,
        group_fn: impl Fn(usize) -> bool,
    ) -> Self {
        let board_len = game.board().len() as usize;
        let n = game.pieces().len();

        let mut group0 = [false; 16];
        for i in 0..board_len {
            group0[i] = group_fn(i);
        }

        // For each piece, collect distinct group-0 contribution values
        // across all its valid placements.
        let mut g0_options: Vec<Vec<u32>> = Vec::with_capacity(n);
        for p in 0..n {
            let placements = game.placements_for(p);
            let mut vals: Vec<u32> = placements.iter().map(|&(_pos, shifted)| {
                let arr = shifted.to_array();
                let mut sum = 0u32;
                for i in 0..board_len {
                    if group0[i] {
                        sum += arr[i] as u32;
                    }
                }
                sum
            }).collect();
            vals.sort_unstable();
            vals.dedup();
            g0_options.push(vals);
        }

        // Compute suffix max.
        let mut suffix_max = vec![0u32; n + 1];
        for i in (0..n).rev() {
            suffix_max[i] = suffix_max[i + 1] + g0_options[i].iter().copied().max().unwrap_or(0);
        }

        // Build suffix DP.
        let dp_size = suffix_max[0] as usize + 1;
        let mut suffix_dp = vec![vec![false; dp_size]; n + 1];
        suffix_dp[n][0] = true;
        for i in (0..n).rev() {
            for v in 0..dp_size {
                if suffix_dp[i + 1][v] {
                    for &g0 in &g0_options[i] {
                        let nv = v + g0 as usize;
                        if nv < dp_size {
                            suffix_dp[i][nv] = true;
                        }
                    }
                }
            }
        }

        SubgameParityPartition { group0, suffix_dp, suffix_max }
    }

    /// Check if the current board's group-0 deficit is reachable from `depth`.
    #[inline(always)]
    fn check(&self, cells: &[u16; 16], board_len: usize, depth: usize, m: u32) -> bool {
        let g0_deficit: u32 = (0..board_len)
            .filter(|&i| self.group0[i])
            .map(|i| cells[i] as u32)
            .sum();

        // Quick bounds check.
        if g0_deficit > self.suffix_max[depth] {
            return false;
        }

        // DP check: is any reachable value v such that v >= g0_deficit
        // and (v - g0_deficit) % M == 0?
        let dp = &self.suffix_dp[depth];
        let mut target = g0_deficit as usize;
        while target < dp.len() {
            if dp[target] {
                return true;
            }
            target += m as usize;
        }
        false
    }
}

/// Mod-M reachability DP for a subset of 1D board cells.
///
/// Precomputes which mod-M configurations of the subset are achievable with
/// remaining pieces. Cell values are reduced mod M before encoding, making
/// this a relaxation that handles subgame values >= M.
struct SubgameSubsetReachability {
    /// Cell indices in the subset (0-indexed into the 1D board).
    cells: Vec<usize>,
    /// M^k where k = cells.len().
    num_configs: usize,
    /// Flat reachability table: `reachable[depth * num_configs + config]`.
    /// 1 = reachable, 0 = unreachable. (n+1) layers.
    reachable: Vec<u8>,
    /// Earliest depth where some config is unreachable.
    first_useful: usize,
}

impl SubgameSubsetReachability {
    /// Encode the current board state for this subset using mod-M values.
    #[inline(always)]
    fn encode_config(&self, cells_arr: &[u16; 16], m: usize) -> usize {
        let mut config = 0usize;
        let mut multiplier = 1usize;
        for &cell_idx in &self.cells {
            let digit = (cells_arr[cell_idx] as usize) % m;
            config += digit * multiplier;
            multiplier *= m;
        }
        config
    }

    /// Check if the current board's subset config is reachable from `depth`.
    #[inline(always)]
    fn check(&self, cells_arr: &[u16; 16], m: usize, depth: usize) -> bool {
        if depth < self.first_useful {
            return true;
        }
        let config = self.encode_config(cells_arr, m);
        self.reachable[depth * self.num_configs + config] != 0
    }

    /// Build a subset reachability DP for the given cell indices.
    fn build(game: &SubgameGame, cells: Vec<usize>) -> Self {
        let k = cells.len();
        let m = game.board().m() as usize;
        let n = game.pieces().len();
        let num_configs = m.pow(k as u32);

        // Apply a mod-M effect to a config: decrement each cell by effect[j] (mod M).
        let apply_effect = |config: usize, effect: &[u8]| -> usize {
            let mut result = config;
            let mut multiplier = 1;
            for i in 0..k {
                if effect[i] > 0 {
                    let digit = (result / multiplier) % m;
                    let new_digit = (digit + m - effect[i] as usize) % m;
                    result = result - digit * multiplier + new_digit * multiplier;
                }
                multiplier *= m;
            }
            result
        };

        // Per piece: enumerate unique mod-M effects on this subset.
        let mut piece_effects: Vec<Vec<Vec<u8>>> = Vec::with_capacity(n);
        for p in 0..n {
            let mut effects_set: Vec<Vec<u8>> = Vec::new();
            let placements = game.placements_for(p);
            for &(_pos, shifted) in placements {
                let arr = shifted.to_array();
                let effect: Vec<u8> = cells.iter()
                    .map(|&ci| (arr[ci] % m as u16) as u8)
                    .collect();
                if !effects_set.contains(&effect) {
                    effects_set.push(effect);
                }
            }
            piece_effects.push(effects_set);
        }

        // Backward suffix DP.
        let total = (n + 1) * num_configs;
        let mut reachable = vec![0u8; total];
        // Base: after all pieces, only config 0 (all zeros mod M) is reachable.
        reachable[n * num_configs] = 1;
        for i in (0..n).rev() {
            let next_base = (i + 1) * num_configs;
            let cur_base = i * num_configs;
            for config in 0..num_configs {
                for effect in &piece_effects[i] {
                    let new_config = apply_effect(config, effect);
                    if reachable[next_base + new_config] != 0 {
                        reachable[cur_base + config] = 1;
                        break;
                    }
                }
            }
        }

        // Find earliest depth where some config is unreachable.
        let mut first_useful = n;
        'outer: for i in 0..n {
            let base = i * num_configs;
            for config in 0..num_configs {
                if reachable[base + config] == 0 {
                    first_useful = i;
                    break 'outer;
                }
            }
        }

        SubgameSubsetReachability { cells, num_configs, reachable, first_useful }
    }
}

/// Subgame solver with backtracking and pruning.
///
/// The solver returns the first solution found, or `Unsolvable` if none exists.
pub struct SubgameSolver {
    /// The subgame to solve.
    game: SubgameGame,
    /// Placement positions chosen so far (one per placed piece).
    solution: Vec<usize>,
    /// Solver statistics.
    stats: SolverStats,
    /// Pruning configuration.
    config: SubgamePruningConfig,
    /// Suffix max-contribution per cell: `max_contrib_suffix[d]` is the SIMD vector
    /// where lane `i` = sum of max contributions to cell `i` from pieces `d..n`.
    max_contrib_suffix: Vec<u16x16>,
    /// Index where all remaining pieces are single-cell ([1] profile).
    /// At this depth, if total deficit matches we can return true immediately.
    endgame_start: usize,
    /// Parity partitions for reachability pruning.
    parity_partitions: Vec<SubgameParityPartition>,
    /// Subset satisfiability checks (mod-M reachability).
    subset_checks: Vec<SubgameSubsetReachability>,
    /// Optional deadline for timeout support.
    deadline: Option<Instant>,
}

impl SubgameSolver {
    /// Create a new solver for the given subgame with all pruning enabled.
    pub fn new(game: SubgameGame) -> Self {
        Self::with_config(game, SubgamePruningConfig::default())
    }

    /// Create a solver with the given pruning configuration.
    pub fn with_config(game: SubgameGame, config: SubgamePruningConfig) -> Self {
        let n = game.pieces().len();
        let max_contrib_suffix = if config.count_sat {
            Self::build_max_contrib_suffix(&game)
        } else {
            vec![u16x16::splat(u16::MAX); n + 1]
        };
        let endgame_start = if config.single_cell_endgame {
            let pieces = game.pieces();
            let mut i = n;
            while i > 0 && pieces[i - 1].len() == 1 && pieces[i - 1].cell_count() == 1 {
                i -= 1;
            }
            i
        } else {
            n // never triggers
        };
        let parity_partitions = if config.parity {
            Self::build_parity_partitions(&game)
        } else {
            vec![]
        };
        let subset_checks = if config.subset_sat {
            Self::build_subset_checks(&game)
        } else {
            vec![]
        };
        Self {
            game,
            solution: Vec::with_capacity(n),
            stats: SolverStats::default(),
            config,
            max_contrib_suffix,
            endgame_start,
            parity_partitions,
            subset_checks,
            deadline: None,
        }
    }

    /// Set a deadline after which the solver will abort.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Build parity partitions for the 1D board.
    fn build_parity_partitions(game: &SubgameGame) -> Vec<SubgameParityPartition> {
        let board_len = game.board().len() as usize;
        let mut partitions = Vec::new();

        // Even/odd partition (always useful).
        partitions.push(SubgameParityPartition::build(game, |i| i % 2 == 0));

        // Mod-3 partitions (useful for boards >= 6 cells).
        if board_len >= 6 {
            partitions.push(SubgameParityPartition::build(game, |i| i % 3 == 0));
            partitions.push(SubgameParityPartition::build(game, |i| i % 3 == 1));
        }

        partitions
    }

    /// Build subset reachability checks for the 1D board.
    fn build_subset_checks(game: &SubgameGame) -> Vec<SubgameSubsetReachability> {
        let m = game.board().m() as usize;
        let board_len = game.board().len() as usize;

        let k_max = match m {
            2 => 14,
            3 => 8,
            4 => 5,
            _ => 4,
        };

        let mut seen: Vec<Vec<usize>> = Vec::new();
        let mut checks: Vec<SubgameSubsetReachability> = Vec::new();

        let mut add = |cells: Vec<usize>| {
            let k = cells.len();
            if k < 2 || k > k_max || k > board_len {
                return;
            }
            let mut sorted = cells.clone();
            sorted.sort_unstable();
            sorted.dedup();
            if sorted.len() != k { return; } // had duplicates
            if *sorted.last().unwrap() >= board_len { return; }
            if seen.contains(&sorted) { return; }
            seen.push(sorted.clone());
            checks.push(SubgameSubsetReachability::build(game, sorted));
        };

        // Full board for M=2 (or any M where board fits).
        if board_len <= k_max {
            add((0..board_len).collect());
        }

        // Sliding windows of k_max contiguous cells.
        if board_len > k_max {
            for start in 0..=(board_len - k_max) {
                add((start..start + k_max).collect());
            }
        }

        // For larger M, also add smaller windows for coverage.
        if m >= 3 {
            let k_small = k_max.min(board_len).saturating_sub(1).max(2);
            if k_small < k_max && board_len > k_small {
                for start in 0..=(board_len - k_small) {
                    add((start..start + k_small).collect());
                }
            }
        }

        // Endpoint subsets: first and last few cells.
        if board_len > k_max {
            let half = k_max / 2;
            let mut endpoint_cells: Vec<usize> = (0..half).collect();
            endpoint_cells.extend((board_len - half)..board_len);
            add(endpoint_cells);
        }

        checks
    }

    /// Precompute suffix sums of per-cell max contributions.
    ///
    /// For each piece, compute the element-wise max across all its placements.
    /// Then `max_contrib_suffix[d][i]` = sum over pieces `d..n` of that per-piece max at cell `i`.
    fn build_max_contrib_suffix(game: &SubgameGame) -> Vec<u16x16> {
        let n = game.pieces().len();
        // Per-piece max contribution at each cell.
        let mut per_piece_max: Vec<u16x16> = Vec::with_capacity(n);
        for p in 0..n {
            let placements = game.placements_for(p);
            let mut max_vec = u16x16::splat(0);
            for &(_pos, shifted) in placements {
                max_vec = max_vec.simd_max(shifted);
            }
            per_piece_max.push(max_vec);
        }

        // Build suffix sums: suffix[n] = 0, suffix[d] = suffix[d+1] + per_piece_max[d]
        let mut suffix = vec![u16x16::splat(0); n + 1];
        for d in (0..n).rev() {
            suffix[d] = suffix[d + 1] + per_piece_max[d];
        }
        suffix
    }

    /// Check count satisfiability: can remaining pieces (from `depth` onward)
    /// cover every cell's current deficit?
    #[inline(always)]
    fn check_count_sat(&self, depth: usize) -> bool {
        let board_cells = self.game.board().cells();
        let max_reachable = self.max_contrib_suffix[depth];
        // For each active cell: max_reachable[i] >= board_cells[i]
        // Equivalently: saturating_sub(board, max_reachable) == 0 for all lanes
        let shortfall = board_cells.saturating_sub(max_reachable);
        shortfall == u16x16::splat(0)
    }

    /// Solve the subgame. Returns the result and solver statistics.
    pub fn solve(mut self) -> (SubgameSolveResult, SolverStats) {
        let total_cells = self.game.remaining_cells_from(0);
        let total_deficit = self.game.board().total_deficit();
        let m = self.game.board().m() as u32;

        // Feasibility: need enough cells, and excess must be a multiple of M
        // (each wrap consumes exactly M extra cells).
        if total_cells < total_deficit || (total_cells - total_deficit) % m != 0 {
            return (SubgameSolveResult::Unsolvable, self.stats);
        }

        if self.game.board().is_solved() && self.game.pieces().is_empty() {
            return (SubgameSolveResult::Solved(vec![]), self.stats);
        }

        let found = self.backtrack(0);
        if found {
            (SubgameSolveResult::Solved(self.solution.clone()), self.stats)
        } else if self.deadline.map_or(false, |dl| Instant::now() >= dl) {
            (SubgameSolveResult::Timeout, self.stats)
        } else {
            (SubgameSolveResult::Unsolvable, self.stats)
        }
    }

    /// Recursive backtracking search.
    ///
    /// `depth` is the index of the current piece to place.
    fn backtrack(&mut self, depth: usize) -> bool {
        self.stats.nodes_visited += 1;

        // Periodic deadline check.
        if self.stats.nodes_visited & 0xFFF == 0 {
            if let Some(dl) = self.deadline {
                if Instant::now() >= dl {
                    return false;
                }
            }
        }

        // Base case: all pieces placed.
        if depth >= self.game.pieces().len() {
            return self.game.board().is_solved();
        }

        // --- Pruning: total deficit check ---
        // With wrapping, deficit can only grow. If remaining cells < deficit,
        // it's infeasible. The mod-M invariant is checked once at the root.
        if self.config.total_deficit {
            let remaining_cells = self.game.remaining_cells_from(depth);
            let remaining_deficit = self.game.board().total_deficit();
            if remaining_cells < remaining_deficit {
                self.stats.deficit_prunes += 1;
                return false;
            }
        }

        // --- Endgame: all remaining pieces are single-cell ---
        // Only valid when remaining cells == remaining deficit (no wrapping needed).
        if depth >= self.endgame_start {
            let n_remaining = self.game.pieces().len() - depth;
            if n_remaining as u32 == self.game.board().total_deficit() {
                let board_len = self.game.board().len() as usize;
                let cells = self.game.board().cells().to_array();
                for i in 0..board_len {
                    if cells[i] > n_remaining as u16 {
                        return false;
                    }
                }
                // Fill in solution: assign pieces greedily to cells by deficit.
                for i in 0..board_len {
                    for _ in 0..cells[i] {
                        self.solution.push(i);
                    }
                }
                return true;
            }
        }

        // --- Pruning: per-cell count satisfiability ---
        if !self.check_count_sat(depth) {
            self.stats.count_sat_prunes += 1;
            return false;
        }

        // --- Pruning: parity partition reachability ---
        if !self.parity_partitions.is_empty() {
            let cells = self.game.board().cells().to_array();
            let board_len = self.game.board().len() as usize;
            let m = self.game.board().m() as u32;
            for partition in &self.parity_partitions {
                if !partition.check(&cells, board_len, depth, m) {
                    self.stats.parity_prunes += 1;
                    return false;
                }
            }
        }

        // --- Pruning: subset satisfiability (mod-M reachability) ---
        if !self.subset_checks.is_empty() {
            let cells = self.game.board().cells().to_array();
            let m = self.game.board().m() as usize;
            for check in &self.subset_checks {
                if !check.check(&cells, m, depth) {
                    self.stats.subset_sat_prunes += 1;
                    return false;
                }
            }
        }

        // Try each placement for the current piece.
        let placements = self.game.placements_for(depth).to_vec();
        for &(pos, shifted) in &placements {
            let wrap_add = self.game.board_mut().apply_piece(shifted);
            self.solution.push(pos);

            if self.backtrack(depth + 1) {
                return true;
            }

            self.solution.pop();
            self.game.board_mut().undo_piece(shifted, wrap_add);
        }

        false
    }
}

/// Convenience function: check if a subgame is solvable.
pub fn is_solvable(game: SubgameGame) -> bool {
    let solver = SubgameSolver::new(game);
    matches!(solver.solve().0, SubgameSolveResult::Solved(_))
}

/// Convenience function: solve a subgame and return the result with stats.
pub fn solve(game: SubgameGame) -> (SubgameSolveResult, SolverStats) {
    SubgameSolver::new(game).solve()
}

/// Solve without count-satisfiability or endgame pruning (for benchmarking).
pub fn solve_baseline(game: SubgameGame) -> (SubgameSolveResult, SolverStats) {
    SubgameSolver::with_config(game, SubgamePruningConfig::none().only(|c| {
        c.total_deficit = true;
    })).solve()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subgame::board::SubgameBoard;
    use crate::subgame::piece::SubgamePiece;
    use crate::subgame::game::SubgameGame;
    use crate::core::board::Board;
    use crate::core::piece::Piece;
    use crate::game::Game;

    #[test]
    fn test_solve_trivial() {
        // Board [1], piece [1] -> place at 0
        let board = SubgameBoard::from_cells(&[1], 2);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, stats) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0]));
        assert!(stats.nodes_visited >= 1);
    }

    #[test]
    fn test_solve_two_pieces() {
        // Board [2, 2], two pieces with profile [1, 1]
        // Both placed at position 0: [2,2] - [1,1] - [1,1] = [0,0]
        let board = SubgameBoard::from_cells(&[2, 2], 2);
        let piece = SubgamePiece::from_profile(&[1, 1]);
        let game = SubgameGame::new(board, vec![piece, piece]);
        let (result, _) = solve(game);
        match result {
            SubgameSolveResult::Solved(positions) => {
                assert_eq!(positions.len(), 2);
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn test_solve_unsolvable_deficit_mismatch() {
        // Board [3], piece [2] -> total deficit 3 != piece cells 2
        let board = SubgameBoard::from_cells(&[3], 2);
        let piece = SubgamePiece::from_profile(&[2]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, _) = solve(game);
        assert_eq!(result, SubgameSolveResult::Unsolvable);
    }

    #[test]
    fn test_solve_unsolvable_no_valid_placement() {
        // Board [1, 3], piece [2, 2] -> deficit matches (4 = 4) but wrapping
        // makes it unsolvable (cell 0 wraps, increasing deficit).
        let board = SubgameBoard::from_cells(&[1, 3], 2);
        let piece = SubgamePiece::from_profile(&[2, 2]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, _stats) = solve(game);
        assert_eq!(result, SubgameSolveResult::Unsolvable);
    }

    #[test]
    fn test_solve_multiple_placements() {
        // Board [1, 0, 1], piece [1] twice
        // First piece at pos 0, second at pos 2 (or vice versa)
        let board = SubgameBoard::from_cells(&[1, 0, 1], 2);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece, piece]);
        let (result, _) = solve(game);
        match result {
            SubgameSolveResult::Solved(positions) => {
                assert_eq!(positions.len(), 2);
                let mut sorted = positions.clone();
                sorted.sort();
                assert_eq!(sorted, vec![0, 2]);
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn test_solve_larger_board() {
        // Board [2, 2, 2, 2], two pieces with profile [1, 1, 1, 1]
        let board = SubgameBoard::from_cells(&[2, 2, 2, 2], 2);
        let piece = SubgamePiece::from_profile(&[1, 1, 1, 1]);
        let game = SubgameGame::new(board, vec![piece, piece]);
        let (result, _) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0, 0]));
    }

    #[test]
    fn test_design_counterexample_both_subgames_solvable() {
        // From DESIGN.md: 3x3, M=3, three 1x3 horizontal bars.
        // Both subgames should be solvable even though full game is not.
        let grid: &[&[u8]] = &[&[0, 1, 2], &[2, 0, 1], &[1, 2, 0]];
        let board = Board::from_grid(grid, 3);
        let bar = Piece::from_grid(&[&[true, true, true]]);
        let game = Game::new(board, vec![bar, bar, bar]);

        let row_sg = to_row_subgame(&game);
        let col_sg = to_col_subgame(&game);

        assert!(is_solvable(row_sg));
        assert!(is_solvable(col_sg));
    }

    #[test]
    fn test_solve_stats_tracking() {
        let board = SubgameBoard::from_cells(&[1, 1], 2);
        let piece = SubgamePiece::from_profile(&[1, 1]);
        let game = SubgameGame::new(board, vec![piece]);
        let (result, stats) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0]));
        assert!(stats.nodes_visited >= 2); // root + base case
    }

    #[test]
    fn test_is_solvable_convenience() {
        let board = SubgameBoard::from_cells(&[1], 2);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece]);
        assert!(is_solvable(game));
    }

    #[test]
    fn test_solve_generated_game_row_subgame() {
        // Generate a game by working backwards from a solved board, then check
        // that the row subgame is solvable.
        let board = Board::new_solved(3, 3, 2);
        let p1 = Piece::from_grid(&[&[true, true], &[true, false]]);
        let p2 = Piece::from_grid(&[&[true]]);

        // Build a game by undoing pieces from a solved board.
        let mut b = board;
        let mask1 = p1.placed_at(0, 0);
        b.undo_piece(mask1);
        let mask2 = p2.placed_at(1, 1);
        b.undo_piece(mask2);
        let game = Game::new(b, vec![p1, p2]);

        let row_sg = to_row_subgame(&game);
        assert!(is_solvable(row_sg));
    }

    #[test]
    fn test_solve_generated_game_col_subgame() {
        let board = Board::new_solved(3, 3, 2);
        let p1 = Piece::from_grid(&[&[true, true], &[true, false]]);
        let p2 = Piece::from_grid(&[&[true]]);

        let mut b = board;
        let mask1 = p1.placed_at(0, 0);
        b.undo_piece(mask1);
        let mask2 = p2.placed_at(1, 1);
        b.undo_piece(mask2);
        let game = Game::new(b, vec![p1, p2]);

        let col_sg = to_col_subgame(&game);
        assert!(is_solvable(col_sg));
    }

    #[test]
    fn test_solve_with_different_profiles() {
        // Board [3, 2, 1], pieces: [2, 1] and [1, 1, 1]
        // Piece 0 at pos 0: [3,2,1]-[2,1,0]=[1,1,1], then piece 1 at pos 0: [1,1,1]-[1,1,1]=[0,0,0]
        let board = SubgameBoard::from_cells(&[3, 2, 1], 2);
        let p1 = SubgamePiece::from_profile(&[2, 1]);
        let p2 = SubgamePiece::from_profile(&[1, 1, 1]);
        let game = SubgameGame::new(board, vec![p1, p2]);
        let (result, _) = solve(game);
        assert_eq!(result, SubgameSolveResult::Solved(vec![0, 0]));
    }

    // --- Fuzz infrastructure ---

    use rand::SeedableRng;
    use crate::generate::generate_game;
    use crate::level::LevelSpec;
    use crate::subgame::generate::{to_row_subgame, to_col_subgame, piece_row_profile, piece_col_profile, board_row_deficits, board_col_deficits};

    /// Generate a full 2D game, sort pieces like the main solver, then project
    /// to row and column subgames.
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

        // Project sorted pieces to row and column profiles.
        let row_profiles: Vec<SubgamePiece> = sorted_pieces.iter().map(|p| piece_row_profile(p)).collect();
        let col_profiles: Vec<SubgamePiece> = sorted_pieces.iter().map(|p| piece_col_profile(p)).collect();

        let row_board = crate::subgame::generate::board_row_deficits(game.board());
        let col_board = crate::subgame::generate::board_col_deficits(game.board());

        let row_sg = SubgameGame::new(row_board, row_profiles);
        let col_sg = SubgameGame::new(col_board, col_profiles);
        (row_sg, col_sg)
    }

    /// Verify that a solution is correct: applying placements zeroes the board.
    fn verify_solution(game: &SubgameGame, positions: &[usize]) -> bool {
        let mut board = game.board().clone();
        for (i, &pos) in positions.iter().enumerate() {
            let placements = game.placements_for(i);
            let shifted = match placements.iter().find(|&&(p, _)| p == pos) {
                Some(&(_, s)) => s,
                None => return false,
            };
            board.apply_piece(shifted);
        }
        board.is_solved()
    }

    /// Fuzz helper: generate full 2D games, project to subgames, solve with
    /// given config, verify soundness, return total nodes.
    fn fuzz_with_config(
        config: &SubgamePruningConfig,
        configs: &[(u8, u8, u8, u8)], // (m, rows, cols, shapes)
        seeds: &[u64],
    ) -> (u64, usize) {
        let mut total_nodes = 0u64;
        let mut failures = 0usize;

        for &(m, rows, cols, shapes) in configs {
            let spec = LevelSpec {
                level: 0, shifts: m, rows, columns: cols, shapes,
            };
            for &seed in seeds {
                let (row_sg, col_sg) = generate_subgames(&spec, seed);

                // Test row subgame.
                let solver = SubgameSolver::with_config(row_sg.clone(), *config);
                let (result, stats) = solver.solve();
                total_nodes += stats.nodes_visited;
                match result {
                    SubgameSolveResult::Solved(ref positions) => {
                        if !verify_solution(&row_sg, positions) {
                            failures += 1;
                        }
                    }
                    SubgameSolveResult::Unsolvable | SubgameSolveResult::Timeout => {}
                }

                // Test column subgame.
                let solver = SubgameSolver::with_config(col_sg.clone(), *config);
                let (result, stats) = solver.solve();
                total_nodes += stats.nodes_visited;
                match result {
                    SubgameSolveResult::Solved(ref positions) => {
                        if !verify_solution(&col_sg, positions) {
                            failures += 1;
                        }
                    }
                    SubgameSolveResult::Unsolvable | SubgameSolveResult::Timeout => {}
                }
            }
        }
        (total_nodes, failures)
    }

    fn fuzz_configs() -> Vec<(u8, u8, u8, u8)> {
        vec![
            (2, 3, 3, 4), (2, 3, 3, 6), (2, 3, 3, 8),
            (3, 3, 3, 3), (3, 3, 3, 5),
            (2, 4, 3, 5), (2, 4, 3, 8),
            (3, 4, 3, 6),
            (2, 4, 4, 6), (2, 4, 4, 10),
            (3, 4, 4, 8),
            (4, 3, 3, 3), (4, 3, 3, 5),
        ]
    }

    fn fuzz_seeds() -> Vec<u64> {
        (0..5).collect()
    }

    // --- Hand-crafted pruning tests ---

    /// Solve with a given config, return (nodes, result).
    fn solve_with(game: SubgameGame, config: SubgamePruningConfig) -> (u64, SubgameSolveResult) {
        let solver = SubgameSolver::with_config(game, config);
        let (result, stats) = solver.solve();
        (stats.nodes_visited, result)
    }

    // -- count_sat: unsolvable, concentrated deficit --
    // Board [6,4], 5x[1,1]. Total cells = 10 = deficit.
    // Each [1,1] contributes max 1 to cell 0 (only pos 0 touches cell 0).
    // 5 pieces can contribute at most 5 to cell 0, but deficit is 6.
    // Count-sat catches this at the root. Baseline must place pieces
    // one by one until underflow.
    #[test]
    fn test_count_sat_unsolvable_concentrated_deficit() {
        let board = SubgameBoard::from_cells(&[6, 4], 2);
        let piece = SubgamePiece::from_profile(&[1, 1]);
        let game = SubgameGame::new(board, vec![piece; 5]);

        let base = baseline_config();
        let with_cs = baseline_config().only(|c| c.count_sat = true);

        let (nodes_base, res_base) = solve_with(game.clone(), base);
        let (nodes_cs, res_cs) = solve_with(game, with_cs);

        assert_eq!(res_base, SubgameSolveResult::Unsolvable);
        assert_eq!(res_cs, SubgameSolveResult::Unsolvable);
        // Baseline places pieces greedily until cell 1 underflows.
        // Count-sat prunes at the root (max 5 to cell 0, need 6).
        assert!(
            nodes_cs < nodes_base,
            "count-sat should reduce nodes: {} vs baseline {}", nodes_cs, nodes_base,
        );
    }

    // -- count_sat: unsolvable, wider board --
    // Board [10,1,1,1,1,1,1], 8x[1,1]. Total = 16 = deficit.
    // Max contribution to cell 0: each [1,1] can give 1 (at pos 0). 8 pieces → max 8.
    // Deficit at cell 0 = 10. 8 < 10 → count-sat prunes at root.
    // Baseline: places [1,1] at pos 0 repeatedly until underflow at cell 1.
    #[test]
    fn test_count_sat_unsolvable_wide_board() {
        let board = SubgameBoard::from_cells(&[10, 1, 1, 1, 1, 1, 1], 2);
        let piece = SubgamePiece::from_profile(&[1, 1]);
        let game = SubgameGame::new(board, vec![piece; 8]);

        let base = baseline_config();
        let with_cs = baseline_config().only(|c| c.count_sat = true);

        let (nodes_base, _) = solve_with(game.clone(), base);
        let (nodes_cs, res_cs) = solve_with(game, with_cs);

        assert_eq!(res_cs, SubgameSolveResult::Unsolvable);
        assert!(
            nodes_cs < nodes_base,
            "count-sat should reduce nodes: {} vs baseline {}", nodes_cs, nodes_base,
        );
    }

    // -- count_sat: prunes wrong branch on solvable game --
    // Board [3,1,1,1,2] (len 5). Pieces: [1,1,1] (len 3), [1,1,1], [1,1].
    // Built from: [1,1,1]@pos0, [1,1,1]@pos2, [1,1]@pos3 = [1,1,1,0,0]+[0,0,1,1,1]+[0,0,0,1,1]
    // = [1,1,2,2,2]. Hmm, let me just construct and test empirically.
    // The key: after placing [1,1,1] at pos 0 (wrong), board has high deficit
    // at cell 4 that remaining pieces can't cover, and count-sat catches it.
    #[test]
    fn test_count_sat_prunes_wrong_branch_solvable() {
        // Build from placements: [1,1,1]@2, [1,1,1]@0, [1,1]@3
        // Board = [1,1,1,0,0] + [0,0,1,1,1] + [0,0,0,1,1] = [1,1,2,2,2]
        let board = SubgameBoard::from_cells(&[1, 1, 2, 2, 2], 2);
        let p3 = SubgamePiece::from_profile(&[1, 1, 1]);
        let p2 = SubgamePiece::from_profile(&[1, 1]);
        // Sorted: [1,1,1] x2 first, then [1,1]
        let game = SubgameGame::new(board, vec![p3, p3, p2]);

        let base = baseline_config();
        let with_cs = baseline_config().only(|c| c.count_sat = true);

        let (nodes_base, res_base) = solve_with(game.clone(), base);
        let (nodes_cs, res_cs) = solve_with(game, with_cs);

        assert!(matches!(res_base, SubgameSolveResult::Solved(_)));
        assert!(matches!(res_cs, SubgameSolveResult::Solved(_)));
        assert!(
            nodes_cs <= nodes_base,
            "count-sat should not increase nodes: {} vs baseline {}", nodes_cs, nodes_base,
        );
    }

    // -- endgame: all single-cell pieces --
    // Board [2,3,1], 6x[1]. Without endgame, solver recurses through all 6
    // pieces. With endgame, resolves in 1 node.
    #[test]
    fn test_endgame_all_single_cell() {
        let board = SubgameBoard::from_cells(&[2, 3, 1], 2);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece; 6]);

        let base = baseline_config();
        let with_eg = baseline_config().only(|c| c.single_cell_endgame = true);

        let (nodes_base, res_base) = solve_with(game.clone(), base);
        let (nodes_eg, res_eg) = solve_with(game, with_eg);

        assert!(matches!(res_base, SubgameSolveResult::Solved(_)));
        assert!(matches!(res_eg, SubgameSolveResult::Solved(_)));
        // Endgame resolves at depth 0 in 1 node. Baseline must recurse.
        assert_eq!(nodes_eg, 1, "endgame should solve in 1 node");
        assert!(
            nodes_eg < nodes_base,
            "endgame should reduce nodes: {} vs baseline {}", nodes_eg, nodes_base,
        );
    }

    // -- endgame: mixed pieces, single-cell tail --
    // Board built from: [2,1]@pos0, [1]@pos0, [1]@pos1, [1]@pos2
    // = [2,1,0] + [1,0,0] + [0,1,0] + [0,0,1] = [3,2,1]
    // Sorted: [2,1], [1], [1], [1]. Endgame at index 1.
    // With endgame: after placing [2,1], endgame handles remaining 3x[1].
    // Without: must recurse through all 3 single-cell pieces.
    #[test]
    fn test_endgame_mixed_pieces_single_cell_tail() {
        let board = SubgameBoard::from_cells(&[3, 2, 1], 2);
        let p21 = SubgamePiece::from_profile(&[2, 1]);
        let p1 = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![p21, p1, p1, p1]);

        let base = baseline_config();
        let with_eg = baseline_config().only(|c| c.single_cell_endgame = true);

        let (nodes_base, res_base) = solve_with(game.clone(), base);
        let (nodes_eg, res_eg) = solve_with(game, with_eg);

        assert!(matches!(res_base, SubgameSolveResult::Solved(_)));
        assert!(matches!(res_eg, SubgameSolveResult::Solved(_)));
        assert!(
            nodes_eg < nodes_base,
            "endgame should reduce nodes: {} vs baseline {}", nodes_eg, nodes_base,
        );
    }

    // -- endgame: large single-cell tail --
    // Board [1,1,1,1,1,1,1,1] (len 8), 8x[1]. Baseline: 9 nodes.
    // Endgame: 1 node.
    #[test]
    fn test_endgame_large_single_cell() {
        let board = SubgameBoard::from_cells(&[1, 1, 1, 1, 1, 1, 1, 1], 2);
        let piece = SubgamePiece::from_profile(&[1]);
        let game = SubgameGame::new(board, vec![piece; 8]);

        let base = baseline_config();
        let with_eg = baseline_config().only(|c| c.single_cell_endgame = true);

        let (nodes_base, _) = solve_with(game.clone(), base);
        let (nodes_eg, res_eg) = solve_with(game, with_eg);

        assert!(matches!(res_eg, SubgameSolveResult::Solved(_)));
        assert_eq!(nodes_eg, 1);
        assert!(
            nodes_eg < nodes_base,
            "endgame should reduce nodes: {} vs baseline {}", nodes_eg, nodes_base,
        );
    }

    // -- count_sat: scaling test --
    // Board [N+1, N-1], Nx[1,1]. Count-sat prunes in 1 node.
    // Baseline: N nodes (places N-1 pieces before underflow on Nth).
    #[test]
    fn test_count_sat_scaling() {
        for n in [5, 10, 20] {
            let board = SubgameBoard::from_cells(&[n as u16 + 1, n as u16 - 1], 2);
            let piece = SubgamePiece::from_profile(&[1, 1]);
            let game = SubgameGame::new(board, vec![piece; n]);

            let base = baseline_config();
            let with_cs = baseline_config().only(|c| c.count_sat = true);

            let (nodes_base, _) = solve_with(game.clone(), base);
            let (nodes_cs, res_cs) = solve_with(game, with_cs);

            assert_eq!(res_cs, SubgameSolveResult::Unsolvable);
            // Count-sat prunes at root; baseline needs ~N nodes.
            assert_eq!(nodes_cs, 1, "count-sat should prune at root for N={n}");
            assert!(
                nodes_base >= n as u64,
                "baseline should visit >= {n} nodes, got {nodes_base}",
            );
        }
    }

    // --- Fuzz: baseline soundness ---

    fn baseline_config() -> SubgamePruningConfig {
        SubgamePruningConfig::none().only(|c| c.total_deficit = true)
    }

    #[test]
    fn test_fuzz_baseline_soundness() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let config = baseline_config();
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "baseline solver produced {} unsound results", failures);
    }

    // --- Fuzz: total deficit pruning ---

    #[test]
    fn test_fuzz_total_deficit_soundness() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let config = SubgamePruningConfig::none().only(|c| c.total_deficit = true);
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "total_deficit prune caused {} failures", failures);
    }

    #[test]
    fn test_fuzz_total_deficit_reduces_nodes() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let no_prune = SubgamePruningConfig::none();
        let with_prune = SubgamePruningConfig::none().only(|c| c.total_deficit = true);

        let (nodes_without, _) = fuzz_with_config(&no_prune, &configs, &seeds);
        let (nodes_with, _) = fuzz_with_config(&with_prune, &configs, &seeds);
        assert!(
            nodes_with <= nodes_without,
            "total_deficit should not increase nodes: {} vs baseline {}",
            nodes_with, nodes_without,
        );
    }

    // --- Fuzz: count-sat pruning ---

    #[test]
    fn test_fuzz_count_sat_soundness() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let config = baseline_config().only(|c| c.count_sat = true);
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "count-sat prune caused {} failures", failures);
    }

    #[test]
    fn test_fuzz_count_sat_reduces_nodes() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let base = baseline_config();
        let with_cs = baseline_config().only(|c| c.count_sat = true);

        let (nodes_base, _) = fuzz_with_config(&base, &configs, &seeds);
        let (nodes_cs, _) = fuzz_with_config(&with_cs, &configs, &seeds);
        assert!(
            nodes_cs <= nodes_base,
            "count-sat should not increase nodes: {} vs baseline {}",
            nodes_cs, nodes_base,
        );
    }

    // --- Fuzz: endgame pruning ---

    #[test]
    fn test_fuzz_endgame_soundness() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let config = baseline_config().only(|c| c.single_cell_endgame = true);
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "endgame prune caused {} failures", failures);
    }

    #[test]
    fn test_fuzz_endgame_reduces_nodes() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let base = baseline_config();
        let with_eg = baseline_config().only(|c| c.single_cell_endgame = true);

        let (nodes_base, _) = fuzz_with_config(&base, &configs, &seeds);
        let (nodes_eg, _) = fuzz_with_config(&with_eg, &configs, &seeds);
        assert!(
            nodes_eg <= nodes_base,
            "endgame should not increase nodes: {} vs baseline {}",
            nodes_eg, nodes_base,
        );
    }

    // --- Fuzz: all optimizations combined ---

    #[test]
    fn test_fuzz_all_optimizations_soundness() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let config = SubgamePruningConfig::default();
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "combined optimizations caused {} failures", failures);
    }

    #[test]
    fn test_fuzz_all_optimizations_reduce_nodes() {
        let configs = fuzz_configs();
        let seeds = fuzz_seeds();
        let base = baseline_config();
        let all = SubgamePruningConfig::default();

        let (nodes_base, _) = fuzz_with_config(&base, &configs, &seeds);
        let (nodes_all, _) = fuzz_with_config(&all, &configs, &seeds);
        assert!(
            nodes_all <= nodes_base,
            "combined optimizations should not increase nodes: {} vs baseline {}",
            nodes_all, nodes_base,
        );
    }

    // --- Fuzz: stress test with larger instances ---

    #[test]
    fn test_fuzz_stress_soundness() {
        let configs = vec![
            (2, 4, 4, 14), (2, 4, 4, 16),
            (3, 4, 4, 12), (3, 4, 4, 14),
            (2, 6, 6, 10), (2, 6, 6, 12),
            (3, 6, 6, 8),
        ];
        let seeds: Vec<u64> = (0..5).collect();
        let config = SubgamePruningConfig::default();
        let (_, failures) = fuzz_with_config(&config, &configs, &seeds);
        assert_eq!(failures, 0, "stress test had {} failures", failures);
    }
}
