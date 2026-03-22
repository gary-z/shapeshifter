use crate::bitboard::Bitboard;
use crate::coverage::{precompute_suffix_coverage, CoverageCounter};
use crate::piece::Piece;

use super::pruning::*;
use super::SolverData;

/// Build all precomputed data needed by the backtracking solver.
///
/// This includes: suffix sums, line family construction, jaggedness masks,
/// parity partitions, subset reachability.
pub(crate) fn build_solver_data(
    pieces: &[Piece],
    order: &[usize],
    all_placements: Vec<Vec<(usize, usize, Bitboard)>>,
    is_dup_of_prev: Vec<bool>,
    skip_tables: Vec<Option<Vec<bool>>>,
    single_cell_start: usize,
    h: u8,
    w: u8,
    m: u8,
) -> SolverData {
    let n = pieces.len();
    let bh = h as usize;
    let bw = w as usize;

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

            // Per piece: enumerate unique effects on this subset from actual placements.
            let mut piece_effects: Vec<Vec<Vec<u8>>> = Vec::with_capacity(n);
            for i in 0..n {
                let mut effects_set: Vec<Vec<u8>> = Vec::new();
                for &(_, _, mask) in &all_placements[i] {
                    let mut effect = vec![0u8; k];
                    for (ci, &bit) in cells.iter().enumerate() {
                        if mask.get_bit(bit) {
                            effect[ci] = 1;
                        }
                    }
                    if !effects_set.contains(&effect) {
                        effects_set.push(effect);
                    }
                }
                piece_effects.push(effects_set);
            }

            // Suffix DP into a flat Vec<u8>: (n+1) layers x num_configs entries.
            let total = (n + 1) * num_configs;
            let mut reachable = vec![0u8; total];
            // Base case: piece n, config 0 (all zeros) is reachable.
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

            let mut mask = Bitboard::ZERO;
            for &bit in &cells {
                mask.set_bit(bit);
            }
            SubsetReachability { cells, m, num_configs, mask, reachable }
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
        if bh >= 4 && bw >= 4 {
            let strip_w = max_subset_k / 2; // columns per window (2 rows x strip_w cols)
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

        // Center subsets: small rectangles and cross patterns near the board center.
        if bh >= 5 && bw >= 5 {
            let cr = bh / 2;
            let cc = bw / 2;

            // Center rectangles of various sizes.
            for &(sh, sw) in &[(2, 2), (2, 3), (3, 2), (3, 3), (1, 4), (4, 1), (1, 5), (5, 1)] {
                if sh > bh || sw > bw { continue; }
                let k = sh * sw;
                if k < 3 || k > max_subset_k { continue; }
                let r0 = cr.saturating_sub(sh / 2);
                let c0 = cc.saturating_sub(sw / 2);
                let cells: Vec<u32> = (r0..r0 + sh)
                    .flat_map(|r| (c0..c0 + sw).map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }

            // Center cross: center cell + 4 neighbors.
            if max_subset_k >= 5 {
                let mut cells = vec![(cr * 15 + cc) as u32];
                if cr > 0 { cells.push(((cr - 1) * 15 + cc) as u32); }
                if cr + 1 < bh { cells.push(((cr + 1) * 15 + cc) as u32); }
                if cc > 0 { cells.push((cr * 15 + cc - 1) as u32); }
                if cc + 1 < bw { cells.push((cr * 15 + cc + 1) as u32); }
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }

            // Horizontal and vertical center strips.
            for len in [3, 4, 5].iter().copied().filter(|&l| l <= max_subset_k && l <= bw) {
                let c0 = cc.saturating_sub(len / 2);
                let cells: Vec<u32> = (c0..c0 + len)
                    .map(|c| (cr * 15 + c) as u32)
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }
            for len in [3, 4, 5].iter().copied().filter(|&l| l <= max_subset_k && l <= bh) {
                let r0 = cr.saturating_sub(len / 2);
                let cells: Vec<u32> = (r0..r0 + len)
                    .map(|r| (r * 15 + cc) as u32)
                    .collect();
                add_subset(cells, &mut subsets, &mut seen_cell_sets);
            }

            // Offset center rectangles: shifted by 1 in each direction.
            for &dr in &[-1i32, 0, 1] {
                for &dc in &[-1i32, 0, 1] {
                    if dr == 0 && dc == 0 { continue; }
                    let r0 = (cr as i32 + dr).max(0) as usize;
                    let c0 = (cc as i32 + dc).max(0) as usize;
                    if r0 + 2 > bh || c0 + 2 > bw { continue; }
                    let cells: Vec<u32> = (r0..r0 + 2)
                        .flat_map(|r| (c0..c0 + 2).map(move |c| (r * 15 + c) as u32))
                        .collect();
                    add_subset(cells, &mut subsets, &mut seen_cell_sets);
                }
            }
        }

        subsets
    };

    SolverData {
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
    }
}
