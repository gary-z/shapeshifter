use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::coverage::precompute_suffix_coverage;
use crate::core::piece::Piece;
use crate::subgame::data::SubgameData;

use super::pruning::*;
use super::SolverData;

/// Build all precomputed data needed by the backtracking solver.
///
/// This includes: suffix sums, line family construction, jaggedness masks,
/// parity partitions, subset reachability, and subgame data.
pub(crate) fn build_solver_data(
    board: &Board,
    pieces: &[Piece],
    order: &[usize],
    all_placements: Vec<Vec<(usize, usize, Bitboard)>>,
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
    let total_deficit_prune = super::prune::total_deficit::TotalDeficitPrune::precompute(pieces, order);
    let jaggedness_prune = super::prune::jaggedness::JaggednessPrune::precompute(pieces, order, h, w);

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

    let line_family_prune = super::prune::line_family::LineFamilyPrune::precompute(pieces, order, h, w, m);

    let parity_prune = super::prune::parity::ParityPrune::precompute(pieces, order, h, w, m);

    // Precompute subset reachability for border regions.
    let max_subset_k: usize = match m {
        2 => 14,   // 16384 states — enables full row pairs on 7-wide boards
        3 => 8,    // 6561 states — enables full row/column subsets on 8-wide boards
        4 => 5,    // 1024 states (reduced from previous 4)
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
                        let new_digit = (digit + m_val - effect[i] as usize) % m_val;
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

            // Find earliest piece_idx where some config is unreachable.
            // Reachability is monotonically non-increasing as piece_idx grows,
            // so we scan forward and stop at the first layer with a gap.
            let mut first_useful = n; // default: never useful
            'outer: for i in 0..n {
                let base = i * num_configs;
                for config in 0..num_configs {
                    if reachable[base + config] == 0 {
                        first_useful = i;
                        break 'outer;
                    }
                }
            }

            let mut mask = Bitboard::ZERO;
            for &bit in &cells {
                mask.set_bit(bit);
            }
            SubsetReachability { cells, m, num_configs, mask, reachable, first_useful }
        };

        let mut cell_sets: Vec<Vec<u32>> = Vec::new();
        let mut seen_cell_sets: Vec<Vec<u32>> = Vec::new();

        let add_subset = |cells: Vec<u32>, cell_sets: &mut Vec<Vec<u32>>,
                              seen: &mut Vec<Vec<u32>>| {
            if cells.len() < 3 || cells.len() > max_subset_k { return; }
            let mut sorted = cells.clone();
            sorted.sort();
            if seen.contains(&sorted) { return; }
            seen.push(sorted);
            cell_sets.push(cells);
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
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
        }

        // Border edge segments: sliding windows of max_subset_k along each edge.
        let seg_len = max_subset_k;
        // Top edge: row 0, varying columns.
        for start_c in 0..=bw.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_c..start_c + seg_len.min(bw - start_c))
                .map(|c| (0 * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
        }
        // Bottom edge.
        for start_c in 0..=bw.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_c..start_c + seg_len.min(bw - start_c))
                .map(|c| ((bh - 1) * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
        }
        // Left edge: col 0, varying rows.
        for start_r in 0..=bh.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_r..start_r + seg_len.min(bh - start_r))
                .map(|r| (r * 15 + 0) as u32)
                .collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
        }
        // Right edge.
        for start_r in 0..=bh.saturating_sub(seg_len) {
            let cells: Vec<u32> = (start_r..start_r + seg_len.min(bh - start_r))
                .map(|r| (r * 15 + (bw - 1)) as u32)
                .collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
            // Bottom 2 rows.
            for start_c in 0..=bw.saturating_sub(strip_w) {
                let cells: Vec<u32> = (bh - 2..bh)
                    .flat_map(|r| (start_c..start_c + strip_w.min(bw - start_c))
                        .map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
            // Left 2 cols.
            let strip_h = max_subset_k / 2;
            for start_r in 0..=bh.saturating_sub(strip_h) {
                let cells: Vec<u32> = (start_r..start_r + strip_h.min(bh - start_r))
                    .flat_map(|r| (0..2usize).map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
            // Right 2 cols.
            for start_r in 0..=bh.saturating_sub(strip_h) {
                let cells: Vec<u32> = (start_r..start_r + strip_h.min(bh - start_r))
                    .flat_map(|r| ((bw - 2)..bw).map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);

            // Phase 1: odd-indexed perimeter cells.
            let cells: Vec<u32> = border_cells.iter().skip(1).step_by(2)
                .take(max_subset_k)
                .map(|&(r, c)| (r * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);

            // Wider spacing: every 3rd cell.
            let cells: Vec<u32> = border_cells.iter().step_by(3)
                .take(max_subset_k)
                .map(|&(r, c)| (r * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);

            // Every 3rd, offset 1.
            let cells: Vec<u32> = border_cells.iter().skip(1).step_by(3)
                .take(max_subset_k)
                .map(|&(r, c)| (r * 15 + c) as u32)
                .collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            // Bottom mid, 2 deep.
            let cells: Vec<u32> = (bh - 2..bh)
                .flat_map(|r| (mid_c..mid_c + seg.min(bw))
                    .map(move |c| (r * 15 + c) as u32))
                .take(max_subset_k).collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            // Left mid, 2 deep.
            let cells: Vec<u32> = (mid_r..mid_r + seg.min(bh))
                .flat_map(|r| (0..2usize).map(move |c| (r * 15 + c) as u32))
                .take(max_subset_k).collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            // Right mid, 2 deep.
            let cells: Vec<u32> = (mid_r..mid_r + seg.min(bh))
                .flat_map(|r| ((bw - 2)..bw).map(move |c| (r * 15 + c) as u32))
                .take(max_subset_k).collect();
            add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }

            // Center cross: center cell + 4 neighbors.
            if max_subset_k >= 5 {
                let mut cells = vec![(cr * 15 + cc) as u32];
                if cr > 0 { cells.push(((cr - 1) * 15 + cc) as u32); }
                if cr + 1 < bh { cells.push(((cr + 1) * 15 + cc) as u32); }
                if cc > 0 { cells.push((cr * 15 + cc - 1) as u32); }
                if cc + 1 < bw { cells.push((cr * 15 + cc + 1) as u32); }
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }

            // Horizontal and vertical center strips.
            for len in [3, 4, 5].iter().copied().filter(|&l| l <= max_subset_k && l <= bw) {
                let c0 = cc.saturating_sub(len / 2);
                let cells: Vec<u32> = (c0..c0 + len)
                    .map(|c| (cr * 15 + c) as u32)
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
            for len in [3, 4, 5].iter().copied().filter(|&l| l <= max_subset_k && l <= bh) {
                let r0 = cr.saturating_sub(len / 2);
                let cells: Vec<u32> = (r0..r0 + len)
                    .map(|r| (r * 15 + cc) as u32)
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
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
                    add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
                }
            }
        }

        // Full columns, rows, and diagonals as subsets.
        // For M=2 these are cheap (2^k states, fast encode) and very effective.
        // For M>=3 they produce 30-50 subsets of 7-8 cells each (3^7=2187 to
        // 3^8=6561 configs), whose per-node encode cost dominates runtime while
        // pruning <7% of nodes beyond what smaller subsets already catch.
        // The line family DP (check_line_family) already covers full-line
        // pruning for all M values.
        if m == 2 {
            for c in 0..bw {
                let cells: Vec<u32> = (0..bh).map(|r| (r * 15 + c) as u32).collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
            for r in 0..bh {
                let cells: Vec<u32> = (0..bw).map(|c| (r * 15 + c) as u32).collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }

            for d in 0..(bh + bw - 1) {
                let diag_offset = d as i32 - (bw as i32 - 1);
                let cells: Vec<u32> = (0..bh)
                    .filter_map(|r| {
                        let c = r as i32 - diag_offset;
                        if c >= 0 && (c as usize) < bw { Some((r * 15 + c as usize) as u32) } else { None }
                    })
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);

                let cells: Vec<u32> = (0..bh)
                    .filter_map(|r| {
                        let c = d as i32 - r as i32;
                        if c >= 0 && (c as usize) < bw { Some((r * 15 + c as usize) as u32) } else { None }
                    })
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
        }

        // Adjacent row pairs (M=2 only): 2 consecutive rows as one subset.
        // For width 7: 14 cells → 2^14 = 16384 states.
        if m == 2 && bh >= 2 && 2 * bw <= max_subset_k {
            for r0 in 0..bh - 1 {
                let cells: Vec<u32> = (r0..r0 + 2)
                    .flat_map(|r| (0..bw).map(move |c| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
        }

        // Adjacent column pairs (M=2 only): 2 consecutive columns.
        // For height 8: 16 cells → 2^16 = 65536 — only if within budget.
        if m == 2 && bw >= 2 && 2 * bh <= max_subset_k {
            for c0 in 0..bw - 1 {
                let cells: Vec<u32> = (c0..c0 + 2)
                    .flat_map(|c| (0..bh).map(move |r| (r * 15 + c) as u32))
                    .collect();
                add_subset(cells, &mut cell_sets, &mut seen_cell_sets);
            }
        }

        cell_sets.into_iter().map(|cells| build_subset(cells)).collect()
    };

    let weight_tuple_checks = build_weight_tuple_checks(
        bh, bw, m, n, &all_placements, order, pieces,
    );

    // Compute progress weights: fraction of naive search space per placement at each depth.
    let mut suffix_products = vec![1.0f64; n + 1];
    for d in (0..n).rev() {
        suffix_products[d] = suffix_products[d + 1] * all_placements[d].len() as f64;
    }
    let total_space = suffix_products[0];
    let progress_weights: Vec<f64> = (0..n)
        .map(|d| if total_space > 0.0 { suffix_products[d + 1] / total_space } else { 0.0 })
        .collect();

    // Build subgame data: row/col profiles, shifted lookup tables, and
    // feasibility check data (placements, max_contrib_suffix, remaining_cells).
    let subset_prune = super::prune::subset::SubsetPrune { checks: subset_checks };
    let weight_tuple_prune = super::prune::weight_tuple::WeightTuplePrune { checks: weight_tuple_checks };
    let subgame_prune = super::prune::subgame::SubgamePrune::precompute(board, pieces, order);
    let hit_count = super::prune::hit_count::precompute(board, &all_placements);
    let num_levels = hit_count.levels.len();

    SolverData {
        all_placements,
        total_deficit_prune,
        jaggedness_prune,
        line_family_prune,
        parity_prune,
        subset_prune,
        weight_tuple_prune,
        subgame_prune,
        hit_count,
        hit_count_level_idx: std::sync::atomic::AtomicUsize::new(num_levels.saturating_sub(1)),
        suffix_coverage,
        skip_tables,
        single_cell_start,
        m,
        h,
        w,
        progress_weights,
    }
}

/// Build weight-tuple reachability checks for groups of disjoint cell sets.
fn build_weight_tuple_checks(
    bh: usize, bw: usize, m: u8, n: usize,
    all_placements: &[Vec<(usize, usize, Bitboard)>],
    _order: &[usize],
    _pieces: &[crate::core::piece::Piece],
) -> Vec<WeightTupleReachability> {
    let m_val = m as u32;
    let max_total_configs = 50_000; // budget: skip if state space too large

    // Helper: build one WeightTupleReachability from a list of group masks.
    let build_wt = |groups: Vec<(Bitboard, usize)>| -> Option<WeightTupleReachability> {
        let num_groups = groups.len();
        if num_groups < 2 { return None; }

        let group_masks: Vec<Bitboard> = groups.iter().map(|&(mask, _)| mask).collect();
        let group_widths: Vec<usize> = groups.iter().map(|&(_, w)| w).collect();
        let max_weights: Vec<u32> = group_widths.iter()
            .map(|&w| w as u32 * (m_val - 1))
            .collect();

        // Compute strides and total configs.
        let mut strides = vec![0usize; num_groups];
        let mut num_configs = 1usize;
        for g in (0..num_groups).rev() {
            strides[g] = num_configs;
            num_configs = num_configs.checked_mul(max_weights[g] as usize + 1)?;
        }
        if num_configs > max_total_configs { return None; }

        // Per piece: for each placement, compute (cells_covered_in_group, cells_count_in_group)
        // per group. At runtime, the weight change depends on how many covered cells are non-zero
        // (unknown), so we enumerate all valid splits.
        //
        // For a group with current weight w and width W:
        //   piece covers C cells in this group.
        //   Let nz = number of covered cells that are non-zero.
        //   Weight change: each non-zero cell hit decreases weight by 1.
        //                  each zero cell hit increases weight by (M-1).
        //   new_w = w - nz + (M-1) * (C - nz) = w + (M-1)*C - M*nz
        //
        // Bounds on nz (non-zero cells hit by the piece in this group):
        //   nz_count (non-zero cells in group) ∈ [ceil(w/(M-1)), min(w, W)]
        //   nz ≤ nz_count ≤ w  ⟹  nz ≤ min(C, w)
        //   C - nz ≤ W - nz_count ≤ W - ceil(w/(M-1))
        //     ⟹  nz ≥ max(0, C - W + ceil(w/(M-1)))
        // For M=2: w = nz_count exactly, so these simplify to the same bounds.

        // Precompute per-piece per-group coverage counts from placements.
        struct PlacementEffect {
            group_counts: Vec<u32>, // cells covered per group
        }

        let mut piece_effects: Vec<Vec<PlacementEffect>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut effects = Vec::new();
            for &(_, _, mask) in &all_placements[i] {
                let gc: Vec<u32> = group_masks.iter()
                    .map(|&gm| (mask & gm).count_ones())
                    .collect();
                // Dedup: skip if same coverage pattern already exists.
                if !effects.iter().any(|e: &PlacementEffect| e.group_counts == gc) {
                    effects.push(PlacementEffect { group_counts: gc });
                }
            }
            piece_effects.push(effects);
        }

        // Suffix DP.
        let total = (n + 1) * num_configs;
        let mut reachable = vec![0u8; total];
        reachable[n * num_configs] = 1; // target: all weights = 0

        let mut new_weights = vec![0u32; num_groups];

        for i in (0..n).rev() {
            let next_base = (i + 1) * num_configs;
            let cur_base = i * num_configs;

            for config in 0..num_configs {
                if reachable[cur_base + config] != 0 { continue; }

                // Decode weight-tuple.
                let mut weights = [0u32; 8];
                let mut tmp = config;
                for g in 0..num_groups {
                    weights[g] = (tmp / strides[g]) as u32;
                    tmp %= strides[g];
                }

                'placement: for effect in &piece_effects[i] {
                    // For each group, compute valid new-weight range.
                    // new_w = w + (M-1)*C - M*nz, where nz ∈ [nz_min, nz_max].
                    // For M=2: nz ∈ [max(0, C-(W-w)), min(C, w)].
                    // General: nz ∈ [max(0, C-(W-w_ceil)), min(C, w_floor)]
                    //   where w_floor = w (at most w non-zero cells) — LOOSE but sound.
                    //   Actually more precise: nz ≤ count of non-zero cells in group ≤ W.
                    //   And nz ≤ C. And (C - nz) ≤ count of zero cells = W - nonzero_count.
                    //   For M=2: nonzero_count = w, so nz ∈ [max(0, C-(W-w)), min(C, w)].
                    //   For M≥3: nonzero_count ∈ [ceil(w/(M-1)), min(w, W)].
                    //     nz ∈ [max(0, C - (W - ceil(w/(M-1)))), min(C, min(w, W))]
                    //     Using the loosest sound bound: nz ∈ [max(0, C-W), min(C, W)]

                    // Use recursive enumeration over groups.
                    // For efficiency, iterate group transitions as nested loops
                    // (unrolled to avoid recursion overhead).
                    fn enumerate_transitions(
                        g: usize, num_groups: usize, m_val: u32,
                        weights: &[u32; 8], effect: &PlacementEffect,
                        group_widths: &[usize], max_weights: &[u32],
                        strides: &[usize],
                        new_weights: &mut Vec<u32>,
                        reachable: &[u8], next_base: usize, num_configs: usize,
                    ) -> bool {
                        if g == num_groups {
                            let mut idx = 0;
                            for gg in 0..num_groups {
                                idx += new_weights[gg] as usize * strides[gg];
                            }
                            return reachable[next_base + idx] != 0;
                        }

                        let w = weights[g];
                        let c = effect.group_counts[g];
                        let gw = group_widths[g] as u32;

                        if c == 0 {
                            new_weights[g] = w;
                            return enumerate_transitions(
                                g + 1, num_groups, m_val, weights, effect,
                                group_widths, max_weights, strides,
                                new_weights, reachable, next_base, num_configs,
                            );
                        }

                        // Bounds on nz (non-zero cells hit).
                        // nz_count (number of non-zero cells in group) satisfies:
                        //   ceil(w/(M-1)) <= nz_count <= min(w, W)
                        // For M=2, w = nz_count exactly, so bounds are tight.
                        // For M>=3, nz_count <= min(w, W) (each non-zero cell
                        // contributes at least 1 to weight, and at most W exist),
                        // so nz <= min(C, w, W).
                        // Zero cells = W - nz_count >= W - min(w, W), and
                        // C - nz <= zero cells, so nz >= C - (W - ceil(w/(M-1))).
                        let nz_count_min = if m_val == 2 { w } else { (w + m_val - 2) / (m_val - 1) };
                        let nz_min = c.saturating_sub(gw - nz_count_min);
                        let nz_max = c.min(w).min(gw);

                        for nz in nz_min..=nz_max {
                            let new_w_raw = w as i64 + (m_val - 1) as i64 * (c - nz) as i64 - nz as i64;
                            if new_w_raw < 0 || new_w_raw > max_weights[g] as i64 { continue; }
                            new_weights[g] = new_w_raw as u32;
                            if enumerate_transitions(
                                g + 1, num_groups, m_val, weights, effect,
                                group_widths, max_weights, strides,
                                new_weights, reachable, next_base, num_configs,
                            ) {
                                return true;
                            }
                        }
                        false
                    }

                    if enumerate_transitions(
                        0, num_groups, m_val, &weights, effect,
                        &group_widths, &max_weights, &strides,
                        &mut new_weights, &reachable, next_base, num_configs,
                    ) {
                        reachable[cur_base + config] = 1;
                        break 'placement;
                    }
                }
            }
        }

        Some(WeightTupleReachability {
            group_masks, num_groups, strides, num_configs, m,
            reachable,
        })
    };

    let mut checks = Vec::new();

    // Row triples: overlapping windows of 3 consecutive rows.
    if bh >= 3 {
        for r0 in 0..=bh - 3 {
            let groups: Vec<(Bitboard, usize)> = (r0..r0 + 3).map(|r| {
                let mut mask = Bitboard::ZERO;
                for c in 0..bw { mask.set_bit((r * 15 + c) as u32); }
                (mask, bw)
            }).collect();
            if let Some(wt) = build_wt(groups) { checks.push(wt); }
        }
    }

    // Column triples: overlapping windows of 3 consecutive columns.
    if bw >= 3 {
        for c0 in 0..=bw - 3 {
            let groups: Vec<(Bitboard, usize)> = (c0..c0 + 3).map(|c| {
                let mut mask = Bitboard::ZERO;
                for r in 0..bh { mask.set_bit((r * 15 + c) as u32); }
                (mask, bh)
            }).collect();
            if let Some(wt) = build_wt(groups) { checks.push(wt); }
        }
    }

    checks
}
