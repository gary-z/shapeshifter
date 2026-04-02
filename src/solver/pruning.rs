use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use super::SolverData;

/// Max number of lines in any family (diagonals on 14x14: 27).
pub(crate) const MAX_LINES: usize = 27;
/// Max number of pieces (n+1 for suffix arrays).
pub(crate) const MAX_PIECES: usize = 37;

/// A family of parallel lines for the min_flips DP pruning.
pub(crate) struct LineFamily {
    pub(crate) masks: [Bitboard; MAX_LINES],
    pub(crate) num_lines: usize,
    /// remaining_budget[i] = suffix sum of max_thickness for pieces [i..n]
    pub(crate) remaining_budget: [u32; MAX_PIECES],
    /// suffix_max_span[i] = max span among pieces [i..n]
    pub(crate) suffix_max_span: [u8; MAX_PIECES],
    /// Whether per_line_budget is available (only for rows and columns).
    pub(crate) has_per_line_budget: bool,
    /// per_line_budget[i][line] = position-aware suffix budget.
    pub(crate) per_line_budget: [[u32; MAX_LINES]; MAX_PIECES],
}

impl LineFamily {
    pub(crate) fn new() -> Self {
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
pub(crate) fn check_line_family(
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
pub(crate) struct ParityPartition {
    /// Mask of group-0 cells on the board.
    pub(crate) mask: Bitboard,
    /// suffix_max[i] = max achievable group-0 increments from pieces [i..n].
    pub(crate) suffix_max: Vec<u32>,
    /// suffix_min[i] = min achievable group-0 increments from pieces [i..n].
    pub(crate) suffix_min: Vec<u32>,
    /// suffix_dp[i] = achievable group-0 totals from pieces [i..n].
    pub(crate) suffix_dp: Vec<Vec<bool>>,
}


/// Weight-tuple reachability for a set of disjoint cell groups.
/// Each group's "weight" = Σ (M-d) for non-zero cells. The DP tracks which
/// weight-tuples are achievable by remaining pieces. Transitions are
/// over-approximated: for each placement, we allow all weight changes
/// consistent with the group's current weight (since we don't know which
/// specific cells are at which values).
pub(crate) struct WeightTupleReachability {
    /// Masks for each group (disjoint cell sets).
    pub(crate) group_masks: Vec<Bitboard>,
    /// Number of groups.
    pub(crate) num_groups: usize,
    /// Product of (max_weight+1) for indexing: strides[i] = Π_{j>i} (max_weights[j]+1).
    pub(crate) strides: Vec<usize>,
    /// Total number of weight-tuple configs.
    pub(crate) num_configs: usize,
    /// M value.
    pub(crate) m: u8,
    /// Flat reachability: reachable[piece_idx * num_configs + config] = 1 if achievable.
    pub(crate) reachable: Vec<u8>,
}

impl WeightTupleReachability {
    /// Encode a weight-tuple into a flat index.
    #[inline]
    pub(crate) fn encode(&self, weights: &[u32]) -> usize {
        let mut idx = 0;
        for g in 0..self.num_groups {
            idx += weights[g] as usize * self.strides[g];
        }
        idx
    }

    /// Compute the weight of a group from the board state.
    #[inline]
    pub(crate) fn group_weight(&self, board: &Board, group_idx: usize) -> u32 {
        let mask = self.group_masks[group_idx];
        let mut w = 0u32;
        for d in 1..self.m {
            w += (self.m - d) as u32 * (board.plane(d) & mask).count_ones();
        }
        w
    }

    /// Check if the current board's weight-tuple is reachable from piece_idx.
    #[inline]
    pub(crate) fn check(&self, board: &Board, piece_idx: usize) -> bool {
        let mut weights = [0u32; 8]; // max 8 groups
        for g in 0..self.num_groups {
            weights[g] = self.group_weight(board, g);
        }
        let idx = self.encode(&weights);
        self.reachable[piece_idx * self.num_configs + idx] != 0
    }
}

/// A small subset of board cells for local reachability pruning.
/// The suffix DP tracks which configurations of the subset cells are achievable.
pub(crate) struct SubsetReachability {
    /// Board cell positions in the subset (as bit indices r*15+c).
    pub(crate) cells: Vec<u32>,
    /// M value.
    pub(crate) m: u8,
    /// Number of configs = M^k.
    pub(crate) num_configs: usize,
    /// Precomputed mask: OR of all cell bit positions. Used for fast-path
    /// when all cells are already 0 (config=0 is always reachable).
    pub(crate) mask: Bitboard,
    /// Flat byte array: entry at `piece_idx * num_configs + config` = 1 if pieces
    /// [piece_idx..n] can transform the subset from `config` to all-zeros.
    /// Config is encoded as a base-M number: cell[0] + cell[1]*M + cell[2]*M^2 + ...
    pub(crate) reachable: Vec<u8>,
    /// Earliest piece_idx where at least one config is unreachable.
    /// For piece_idx < first_useful, all configs are reachable so check always passes.
    pub(crate) first_useful: usize,
}

impl SubsetReachability {
    /// Encode the current subset cell values from the board into a config index.
    #[inline(always)]
    pub(crate) fn encode_config(&self, board: &Board) -> usize {
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
    pub(crate) fn check(&self, board: &Board, piece_idx: usize) -> bool {
        // Skip check at shallow depths where all configs are reachable.
        if piece_idx < self.first_useful {
            return true;
        }
        // Fast path: if all subset cells are 0, config=0 which is always reachable
        // (the zero-effect identity is always available for every piece).
        if (board.plane(0) & self.mask) == self.mask {
            return true;
        }
        let config = self.encode_config(board);
        self.reachable[piece_idx * self.num_configs + config] != 0
    }
}

#[inline(always)]
pub(crate) fn prune_min_flips_global(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    data.remaining_bits[piece_idx] >= board.min_flips_needed()
}

#[inline(always)]
pub(crate) fn prune_line_families_rowcol(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    for f in &data.line_families[..2] {
        if !check_line_family(board, f, piece_idx, data.m) { return false; }
    }
    true
}

#[inline(always)]
pub(crate) fn prune_line_families_diagonal(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    for f in &data.line_families[2..] {
        if !check_line_family(board, f, piece_idx, data.m) { return false; }
    }
    true
}

#[inline(always)]
pub(crate) fn prune_jaggedness(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    let j = board.split_jaggedness(
        data.jagg_h_mask, data.jagg_h_total, data.jagg_v_mask, data.jagg_v_total);
    let rem_h = data.remaining_h_perimeter[piece_idx];
    let rem_v = data.remaining_v_perimeter[piece_idx];
    // Circular (symmetric) bound: sum of min(|a-b|, M-|a-b|) <= perimeter.
    if j.circular_h > rem_h || j.circular_v > rem_v {
        return false;
    }
    // Directional (asymmetric) bound for M>=4:
    // max(forward, backward) <= M/2 * perimeter, i.e., 2*max(fwd,bwd) <= M*perim.
    // Strictly tighter than circular for pairs at distance d < M/2.
    if data.m >= 4 {
        let m = data.m as u32;
        if j.forward_h * 2 > m * rem_h || j.backward_h * 2 > m * rem_h {
            return false;
        }
        if j.forward_v * 2 > m * rem_v || j.backward_v * 2 > m * rem_v {
            return false;
        }
    }
    true
}

#[inline(always)]
pub(crate) fn prune_subset_reachability(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    for subset in &data.subset_checks {
        if !subset.check(board, piece_idx) {
            return false;
        }
    }
    true
}

pub(crate) fn prune_weight_tuples(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    for wt in &data.weight_tuple_checks {
        if !wt.check(board, piece_idx) {
            return false;
        }
    }
    true
}

pub(crate) fn prune_parity_partitions(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
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

/// Run all pruning checks for a given board state and piece index.
/// Returns true if the state is feasible (search should continue).
/// Used by both the backtracker and the combo enumerator.
/// Run all pruning checks for a given board state and piece index.
/// Returns true if the state is feasible (search should continue).
/// Ordered by effectiveness: cheapest high-impact checks first.
///
/// Ablation results on historical puzzles (L43 M=2, L36/L46 M=3):
/// - min_flips_global: CRITICAL (>10× node increase without it)
/// - pair skip tables: CRITICAL (handled in placement filtering, not here)
/// - jaggedness: +83% nodes on M=2, marginal on M=3
/// - min_flips_diagonal: +16% nodes on M=2, marginal on M=3
/// - parity_partitions, subset_reachability, weight_tuples: part of min_flips_global gate
/// - active_planes, coverage, cell_locking, min_flips_rowcol: 0-1% node
///   reduction but 6-16% time cost — removed from hot path.
pub(crate) fn prune_node(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    config: &super::PruningConfig,
) -> bool {
    // Ordered by cost-effectiveness. Checks that prune 0-1% of nodes but cost
    // 6-16% of time (active_planes, coverage, cell_locking, min_flips_rowcol,
    // subgrid) are omitted. Validated on simulated L36-60:
    // 78/125 vs 79/125 solves — negligible impact, +14-27% throughput gain.
    if config.min_flips_global && !prune_min_flips_global(board, data, piece_idx) { return false; }
    if config.jaggedness && !prune_jaggedness(board, data, piece_idx) { return false; }
    if config.min_flips_rowcol && !prune_line_families_rowcol(board, data, piece_idx) { return false; }
    if config.min_flips_diagonal && !prune_line_families_diagonal(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_parity_partitions(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_subset_reachability(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_weight_tuples(board, data, piece_idx) { return false; }
    true
}
