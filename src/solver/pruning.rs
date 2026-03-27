use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::coverage::has_sufficient_coverage;

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
    /// Width (number of cells) per group, for transition bounds.
    pub(crate) group_widths: Vec<usize>,
    /// Number of groups.
    pub(crate) num_groups: usize,
    /// Max weight per group: group_width * (M-1).
    pub(crate) max_weights: Vec<u32>,
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
        // Fast path: if all subset cells are 0, config=0 which is always reachable
        // (the zero-effect identity is always available for every piece).
        if (board.plane(0) & self.mask) == self.mask {
            return true;
        }
        let config = self.encode_config(board);
        self.reachable[piece_idx * self.num_configs + config] != 0
    }
}

/// Flood-fill one connected component from a seed bit within `region`.
/// Returns the component mask. Uses bitboard-parallel expansion.
pub(crate) fn flood_fill(seed_bit: u32, region: Bitboard) -> Bitboard {
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
/// For each component, verify:
/// - Reachable pieces have enough cell_counts to cover min_flips
/// - Reachable pieces have enough perimeter to cover jaggedness
/// Also computes sum of active_planes across components (returned for caller to check).
pub(crate) fn check_components(
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

        // Component jaggedness -- split into h/v.
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

#[inline(always)]
pub(crate) fn prune_active_planes(board: &Board, remaining: usize) -> bool {
    board.active_planes() as usize <= remaining
}

#[inline(always)]
pub(crate) fn prune_min_flips_global(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    data.remaining_bits[piece_idx] >= board.min_flips_needed()
}

#[inline(always)]
pub(crate) fn prune_line_families_rowcol(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    check_line_family(board, &data.line_families[0], piece_idx, data.m)
        && check_line_family(board, &data.line_families[1], piece_idx, data.m)
}

#[inline(always)]
pub(crate) fn prune_line_families_diagonal(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    for f in &data.line_families[2..] {
        if !check_line_family(board, f, piece_idx, data.m) { return false; }
    }
    true
}

#[inline(always)]
pub(crate) fn prune_subgrid(board: &Board, data: &SolverData, piece_idx: usize, remaining: usize) -> bool {
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
pub(crate) fn prune_coverage(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    has_sufficient_coverage(board, &data.suffix_coverage[piece_idx], data.m)
}

#[inline(always)]
pub(crate) fn prune_jaggedness(board: &Board, data: &SolverData, piece_idx: usize) -> bool {
    let (h_jagg, v_jagg) = board.split_jaggedness(
        data.jagg_h_mask, data.jagg_h_total, data.jagg_v_mask, data.jagg_v_total);
    h_jagg <= data.remaining_h_perimeter[piece_idx]
        && v_jagg <= data.remaining_v_perimeter[piece_idx]
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

/// Expand a bitboard horizontally by w_steps and vertically by h_steps,
/// staying within the valid board mask. This produces a rectangular expansion.
#[inline(always)]
fn expand_rect(bb: Bitboard, h_steps: usize, w_steps: usize, board_mask: Bitboard) -> Bitboard {
    let mut result = bb;
    for _ in 0..w_steps {
        result = (result | (result << 1) | (result >> 1)) & board_mask;
    }
    for _ in 0..h_steps {
        result = (result | (result << 15) | (result >> 15)) & board_mask;
    }
    result
}

/// Flood-fill with rectangular expansion matching piece bounding box.
/// Two non-zero cells are in the same cluster if connected by a chain of
/// non-zero cells where consecutive cells are within (max_h-1, max_w-1) L∞ distance.
fn flood_fill_rect(seed_bit: u32, nz: Bitboard, max_h: usize, max_w: usize, board_mask: Bitboard) -> Bitboard {
    let mut cluster = Bitboard::from_bit(seed_bit) & nz;
    loop {
        let expanded = expand_rect(cluster, max_h.saturating_sub(1), max_w.saturating_sub(1), board_mask) & nz;
        if expanded == cluster { break; }
        cluster = expanded;
    }
    cluster
}

/// Distance-based partition pruning with Hall's condition checks.
/// Partitions non-zero cells into clusters where no piece can span between clusters.
/// Uses per-cluster reachability checks, then checks Hall's condition on cluster
/// subsets to detect cross-cluster conflicts where clusters compete for the same pieces.
pub(crate) fn prune_distance_partition(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
) -> bool {
    let max_h = data.line_families[0].suffix_max_span[piece_idx] as usize;
    let max_w = data.line_families[1].suffix_max_span[piece_idx] as usize;
    if max_h == 0 || max_w == 0 { return true; }

    // Find all non-zero cells.
    let mut nz = Bitboard::ZERO;
    for d in 1..data.m {
        nz |= board.plane(d);
    }
    if nz.is_zero() { return true; }

    // Collect clusters.
    let mut clusters = [Bitboard::ZERO; 16];
    let mut cluster_demands = [0u32; 16];
    let mut num_clusters = 0usize;
    let mut remaining_nz = nz;

    while !remaining_nz.is_zero() && num_clusters < 16 {
        let seed = remaining_nz.lowest_set_bit();
        let cluster = flood_fill_rect(seed, remaining_nz, max_h, max_w, data.board_mask);
        remaining_nz = remaining_nz & !cluster;

        let mut demand = 0u32;
        for d in 1..data.m {
            demand += (data.m - d) as u32 * (board.plane(d) & cluster).count_ones();
        }

        clusters[num_clusters] = cluster;
        cluster_demands[num_clusters] = demand;
        num_clusters += 1;
    }

    // Single cluster: reduces to global checks already done.
    if num_clusters <= 1 { return true; }

    let num_remaining = data.reaches.len() - piece_idx;

    // Build per-piece reach masks and capacities.
    let mut piece_caps = [0u32; 36];
    let mut reach_mask = [0u16; 36];

    for pi in piece_idx..data.reaches.len() {
        let i = pi - piece_idx;
        piece_caps[i] = data.cell_counts[pi];
        for j in 0..num_clusters {
            if !(data.reaches[pi] & clusters[j]).is_zero() {
                reach_mask[i] |= 1 << j;
            }
        }
    }

    // Per-cluster independent check (Hall's condition on singletons).
    for j in 0..num_clusters {
        let mut supply = 0u32;
        for i in 0..num_remaining {
            if reach_mask[i] & (1 << j) != 0 {
                supply += piece_caps[i];
            }
        }
        if supply < cluster_demands[j] {
            return false;
        }
    }

    // Hall's condition on pairs: for every pair of clusters {j1, j2},
    // the total supply of pieces reaching j1 OR j2 must cover both demands.
    for j1 in 0..num_clusters {
        for j2 in (j1 + 1)..num_clusters {
            let pair_demand = cluster_demands[j1] + cluster_demands[j2];
            let pair_mask = (1u16 << j1) | (1u16 << j2);
            let mut supply = 0u32;
            for i in 0..num_remaining {
                if reach_mask[i] & pair_mask != 0 {
                    supply += piece_caps[i];
                }
                if supply >= pair_demand { break; }
            }
            if supply < pair_demand {
                return false;
            }
        }
    }

    // Hall's condition on triples (only when cluster count is moderate).
    if num_clusters >= 3 && num_clusters <= 10 {
        for j1 in 0..num_clusters {
            for j2 in (j1 + 1)..num_clusters {
                for j3 in (j2 + 1)..num_clusters {
                    let triple_demand = cluster_demands[j1] + cluster_demands[j2] + cluster_demands[j3];
                    let triple_mask = (1u16 << j1) | (1u16 << j2) | (1u16 << j3);
                    let mut supply = 0u32;
                    for i in 0..num_remaining {
                        if reach_mask[i] & triple_mask != 0 {
                            supply += piece_caps[i];
                        }
                        if supply >= triple_demand { break; }
                    }
                    if supply < triple_demand {
                        return false;
                    }
                }
            }
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
/// - duplicate_pruning: CRITICAL (handled in placement filtering, not here)
/// - jaggedness: +83% nodes on M=2, marginal on M=3
/// - min_flips_diagonal: +16% nodes on M=2, marginal on M=3
/// - parity_partitions, subset_reachability, weight_tuples: part of min_flips_global gate
/// - active_planes, coverage, cell_locking, min_flips_rowcol, component_checks: 0-1% node
///   reduction but 6-16% time cost — removed from hot path.
pub(crate) fn prune_node(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    config: &super::PruningConfig,
) -> bool {
    // Ordered by cost-effectiveness. Checks that prune 0-1% of nodes but cost
    // 6-16% of time (active_planes, coverage, cell_locking, min_flips_rowcol,
    // subgrid, component_checks) are omitted. Validated on simulated L36-60:
    // 78/125 vs 79/125 solves — negligible impact, +14-27% throughput gain.
    if config.min_flips_global && !prune_min_flips_global(board, data, piece_idx) { return false; }
    if config.jaggedness && !prune_jaggedness(board, data, piece_idx) { return false; }
    if config.min_flips_diagonal && !prune_line_families_diagonal(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_parity_partitions(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_subset_reachability(board, data, piece_idx) { return false; }
    if config.min_flips_global && !prune_weight_tuples(board, data, piece_idx) { return false; }
    true
}
