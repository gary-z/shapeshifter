use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

use super::pruning::*;
use super::{PruningConfig, SolverData};

/// Inline xorshift64 step — very fast, good enough for tie-breaking shuffles.
#[inline(always)]
pub(crate) fn xorshift64(state: &Cell<u64>) -> u64 {
    let mut s = state.get();
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    state.set(s);
    s
}

/// Try to solve remaining pieces when they're all 1x1.
/// Each cell at value d needs (M-d)%M hits. Total hits must equal number of pieces.
/// Returns true and fills solution if solvable.
pub(crate) fn solve_single_cells(
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

/// Generate backtrack functions with and without abort support.
/// This macro avoids code duplication while keeping the serial path
/// free of any abort-related overhead (no extra parameter, no branch).
macro_rules! define_backtrack {
    ($name:ident $(, abort: $abort_param:ident : $abort_ty:ty)?) => {
        pub(crate) fn $name(
            board: &Board,
            data: &SolverData,
            piece_idx: usize,
            min_placement: usize,
            prev_dup_placement: usize,
            solution: &mut Vec<(usize, usize)>,
            nodes: &Cell<u64>,
            config: &PruningConfig,
            rng: &Cell<u64>,
            $($abort_param: $abort_ty,)?
        ) -> bool {
            nodes.set(nodes.get() + 1);

            $(
                // Check abort every 1024 nodes.
                if nodes.get() & 1023 == 0 && $abort_param.load(Ordering::Relaxed) {
                    return false;
                }
            )?

            if piece_idx == data.all_placements.len() {
                return board.is_solved();
            }

            // If all remaining pieces are 1x1, solve directly.
            if config.single_cell_endgame && piece_idx >= data.single_cell_start {
                let num_remaining = data.all_placements.len() - piece_idx;
                return solve_single_cells(board, data.m, data.h, data.w, num_remaining, solution);
            }

            if !prune_node(board, data, piece_idx, config) { return false; }

            // Compute locked mask: cells at 0 where remaining coverage < M.
            let locked_mask = if config.cell_locking {
                board.plane(0) & !data.suffix_coverage[piece_idx].coverage_ge(data.m)
            } else {
                Bitboard::ZERO
            };

            // Prune: per-component checks (jaggedness, min_flips).
            // Run when branching factor justifies flood-fill cost.
            // Component checks (flood-fill + per-component jaggedness/min_flips) disabled:
            // profiling shows 12% of instructions but <0.1% node reduction on real puzzles.
            // The per-component bounds are too loose with large pieces on medium boards.
            // Keeping the code for potential future use on larger boards.
            if false && config.component_checks {
                if !check_components(board, locked_mask, data, piece_idx) {
                    return false;
                }
            }

            let placements = &data.all_placements[piece_idx];

            // Order placements by zeros hit ascending using counting sort.
            // Keys are small (0..=max_piece_area), so O(n) bucket sort beats O(n^2) insertion sort.
            let zero_plane = board.plane(0);
            let pl_len = placements.len();
            let mut order = [0u8; 196];
            let mut keys = [0u8; 196];
            for i in 0..pl_len {
                keys[i] = (placements[i].2 & zero_plane).count_ones() as u8;
            }
            // Counting sort: count occurrences, then build order from buckets.
            let mut counts = [0u8; 26]; // max piece area is 25 (5x5)
            for i in 0..pl_len { counts[keys[i] as usize] += 1; }
            let mut offsets = [0u8; 26];
            for i in 1..26 { offsets[i] = offsets[i - 1] + counts[i - 1]; }
            // Save bucket boundaries for tie-shuffling.
            let bucket_starts = offsets;
            for i in 0..pl_len {
                let k = keys[i] as usize;
                order[offsets[k] as usize] = i as u8;
                offsets[k] += 1;
            }

            // Shuffle within each bucket (Fisher-Yates) to diversify tie-breaking.
            // rng state of 0 means no shuffling (deterministic baseline).
            if rng.get() != 0 {
                for bucket in 0..26 {
                    let start = bucket_starts[bucket] as usize;
                    let end = offsets[bucket] as usize;
                    let len = end - start;
                    if len > 1 {
                        for i in (1..len).rev() {
                            let j = xorshift64(rng) as usize % (i + 1);
                            order.swap(start + i, start + j);
                        }
                    }
                }
            }

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

                // Min-flips lookahead: predict min_flips after this placement
                // without the cost of apply_piece + recursive call + undo_piece.
                // min_flips_after = current + M * zeros_hit - piece_area.
                if piece_idx + 1 < data.all_placements.len() {
                    let min_flips_after = board.min_flips_needed()
                        + data.m as u32 * keys[pl_idx] as u32
                        - data.cell_counts[piece_idx];
                    if min_flips_after > data.remaining_bits[piece_idx + 1] {
                        continue;
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

                if $name(
                    &board,
                    data,
                    piece_idx + 1,
                    next_min,
                    next_prev_dup,
                    solution,
                    nodes,
                    config,
                    rng,
                    $($abort_param,)?
                ) {
                    return true;
                }

                solution.pop();
                board.undo_piece(mask);
            }

            false
        }
    };
}

// Serial backtrack: no abort parameter, no overhead.
define_backtrack!(backtrack);

// Abortable backtrack: checks abort flag every 1024 nodes.
define_backtrack!(backtrack_abortable, abort: abort: &AtomicBool);

// ---------------------------------------------------------------------------
// Iterative backtrack with budget-based work stealing
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::sync::atomic::AtomicUsize;

/// A frame on the explicit search stack for the iterative backtracker.
struct SearchFrame {
    /// Board state BEFORE any placement at this level.
    board: Board,
    /// Which piece this frame places.
    piece_idx: usize,
    /// Pre-filtered, ordered placements: (original_pl_idx, row, col, mask).
    placements: Vec<(usize, usize, usize, Bitboard)>,
    /// Next index into `placements` to try.
    cursor: usize,
}

/// A stealable task: a search starting point at any depth.
pub(crate) struct StealableTask {
    pub board: Board,
    pub prefix: Vec<(usize, usize)>,
    pub depth: usize,
    pub min_placement: usize,
    pub prev_dup: usize,
}

/// Shared work queue with condvar for idle thread notification.
pub(crate) struct WorkQueue {
    queue: Mutex<VecDeque<StealableTask>>,
    condvar: Condvar,
}

impl WorkQueue {
    pub fn new() -> Self {
        Self { queue: Mutex::new(VecDeque::new()), condvar: Condvar::new() }
    }

    pub fn push(&self, task: StealableTask) {
        self.queue.lock().unwrap().push_back(task);
        self.condvar.notify_one();
    }

    pub fn push_many(&self, tasks: Vec<StealableTask>) {
        if tasks.is_empty() { return; }
        let mut q = self.queue.lock().unwrap();
        for t in tasks {
            q.push_back(t);
        }
        self.condvar.notify_all();
    }

    pub fn pop(&self) -> Option<StealableTask> {
        self.queue.lock().unwrap().pop_front()
    }

    /// Block until a task is available, abort is set, or termination is detected
    /// (active_count == 0 means no thread can produce new work).
    pub fn wait_for_task(
        &self,
        abort: &AtomicBool,
        active_count: &AtomicUsize,
    ) -> Option<StealableTask> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if abort.load(Ordering::Relaxed) { return None; }
            if let Some(t) = q.pop_front() { return Some(t); }
            if active_count.load(Ordering::SeqCst) == 0 { return None; }
            // Wait for notify (from push/push_many) with a timeout
            // to recheck active_count periodically.
            let (new_q, _) = self.condvar.wait_timeout(
                q, std::time::Duration::from_millis(1)
            ).unwrap();
            q = new_q;
        }
    }
}

/// Build a search frame: compute placement ordering, filter, and collect valid moves.
fn build_search_frame(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    min_placement: usize,
    prev_dup_placement: usize,
    config: &PruningConfig,
    rng: &Cell<u64>,
) -> SearchFrame {
    let placements = &data.all_placements[piece_idx];
    let pl_len = placements.len();

    // Locked mask.
    let locked_mask = if config.cell_locking {
        board.plane(0) & !data.suffix_coverage[piece_idx].coverage_ge(data.m)
    } else {
        Bitboard::ZERO
    };

    // Counting sort by zeros_hit ascending.
    let zero_plane = board.plane(0);
    let mut order = [0u8; 196];
    let mut keys = [0u8; 196];
    for i in 0..pl_len {
        keys[i] = (placements[i].2 & zero_plane).count_ones() as u8;
    }
    let mut counts = [0u8; 26];
    for i in 0..pl_len { counts[keys[i] as usize] += 1; }
    let mut offsets = [0u8; 26];
    for i in 1..26 { offsets[i] = offsets[i - 1] + counts[i - 1]; }
    let bucket_starts = offsets;
    for i in 0..pl_len {
        let k = keys[i] as usize;
        order[offsets[k] as usize] = i as u8;
        offsets[k] += 1;
    }

    // Shuffle within buckets for tie diversity.
    if rng.get() != 0 {
        for bucket in 0..26 {
            let start = bucket_starts[bucket] as usize;
            let end = offsets[bucket] as usize;
            let len = end - start;
            if len > 1 {
                for i in (1..len).rev() {
                    let j = xorshift64(rng) as usize % (i + 1);
                    order.swap(start + i, start + j);
                }
            }
        }
    }

    // Filter and collect valid placements.
    let mut filtered = Vec::with_capacity(pl_len);
    for oi in 0..pl_len {
        let pl_idx = order[oi] as usize;
        let (row, col, mask) = placements[pl_idx];

        if config.duplicate_pruning && pl_idx < min_placement {
            continue;
        }
        if !(mask & locked_mask).is_zero() {
            continue;
        }
        if prev_dup_placement < usize::MAX {
            if let Some(ref table) = data.skip_tables[piece_idx] {
                let num_curr = placements.len();
                if table[prev_dup_placement * num_curr + pl_idx] {
                    continue;
                }
            }
        }
        // Min-flips lookahead: skip placements that will immediately fail the global budget.
        if piece_idx + 1 < data.all_placements.len() {
            let min_flips_after = board.min_flips_needed()
                + data.m as u32 * keys[pl_idx] as u32
                - data.cell_counts[piece_idx];
            if min_flips_after > data.remaining_bits[piece_idx + 1] {
                continue;
            }
        }
        filtered.push((pl_idx, row, col, mask));
    }

    SearchFrame {
        board: board.clone(),
        piece_idx,
        placements: filtered,
        cursor: 0,
    }
}

/// Compute (next_min_placement, next_prev_dup) for the piece after `piece_idx`,
/// given that we chose `pl_idx` at `piece_idx`.
#[inline]
fn next_dup_state(
    data: &SolverData,
    piece_idx: usize,
    pl_idx: usize,
    config: &PruningConfig,
) -> (usize, usize) {
    let next = piece_idx + 1;
    let is_next_dup = config.duplicate_pruning
        && next < data.all_placements.len()
        && data.is_dup_of_prev[next];
    let next_min = if is_next_dup { pl_idx } else { 0 };
    let next_prev = if next < data.all_placements.len()
        && data.skip_tables[next].is_some()
    {
        pl_idx
    } else {
        usize::MAX
    };
    (next_min, next_prev)
}

/// Split work from the explicit stack and push to the shared steal queue.
/// Finds the shallowest frame with remaining placements and donates them.
fn split_work(
    stack: &mut [SearchFrame],
    solution_prefix: &[(usize, usize)],
    base_solution_len: usize,
    data: &SolverData,
    config: &PruningConfig,
    wq: &WorkQueue,
) {
    // Find shallowest frame with remaining placements (largest subtrees).
    for (si, frame) in stack.iter_mut().enumerate() {
        if frame.cursor >= frame.placements.len() {
            continue;
        }
        // Donate remaining placements from this frame.
        let mut tasks = Vec::new();
        for ci in frame.cursor..frame.placements.len() {
            let (pl_idx, row, col, mask) = frame.placements[ci];
            let mut board = frame.board.clone();
            board.apply_piece(mask);
            let depth = frame.piece_idx + 1;

            let prefix_len = base_solution_len + si;
            let mut prefix = solution_prefix[..prefix_len].to_vec();
            prefix.push((row, col));

            let (next_min, next_prev) = next_dup_state(data, frame.piece_idx, pl_idx, config);

            tasks.push(StealableTask {
                board, prefix, depth,
                min_placement: next_min, prev_dup: next_prev,
            });
        }
        frame.cursor = frame.placements.len();
        wq.push_many(tasks);
        return;
    }
}

/// Node budget before checking whether to split.
const SPLIT_BUDGET: u64 = 4096;

/// Iterative backtracker with budget-based work stealing.
/// Runs DFS with an explicit stack. Every SPLIT_BUDGET nodes, if idle threads
/// exist, donates remaining work at the shallowest stack level.
pub(crate) fn backtrack_stealing(
    initial_board: &Board,
    data: &SolverData,
    start_depth: usize,
    initial_min: usize,
    initial_prev_dup: usize,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    config: &PruningConfig,
    rng: &Cell<u64>,
    abort: &AtomicBool,
    wq: &WorkQueue,
    idle_count: &AtomicUsize,
    exhaustive: bool,
) -> bool {
    let n = data.all_placements.len();
    let base_solution_len = solution.len();

    // Check terminal / single-cell endgame before building first frame.
    if start_depth == n {
        return initial_board.is_solved();
    }
    if config.single_cell_endgame && start_depth >= data.single_cell_start {
        let num_remaining = n - start_depth;
        return solve_single_cells(initial_board, data.m, data.h, data.w, num_remaining, solution);
    }

    // Pruning at root.
    if !prune_node(initial_board, data, start_depth, config) {
        return false;
    }

    let mut stack: Vec<SearchFrame> = Vec::with_capacity(n - start_depth);
    stack.push(build_search_frame(
        initial_board, data, start_depth, initial_min, initial_prev_dup, config, rng,
    ));

    let mut budget = SPLIT_BUDGET;
    let mut found = false;

    loop {
        // Abort check.
        if abort.load(Ordering::Relaxed) {
            break;
        }

        // Stack empty → this subtree is exhausted.
        if stack.is_empty() {
            break;
        }

        let frame = stack.last_mut().unwrap();

        // All placements at this level tried → backtrack.
        if frame.cursor >= frame.placements.len() {
            stack.pop();
            continue;
        }

        // Take next placement.
        let (pl_idx, row, col, mask) = frame.placements[frame.cursor];
        frame.cursor += 1;
        let piece_idx = frame.piece_idx;

        // Apply placement.
        let mut board = frame.board.clone();
        board.apply_piece(mask);

        // Update solution: truncate to this frame's depth, then push.
        let sol_depth = base_solution_len + stack.len() - 1;
        solution.truncate(sol_depth);
        solution.push((row, col));

        nodes.set(nodes.get() + 1);

        let next_piece = piece_idx + 1;

        // Terminal: placed all pieces.
        if next_piece == n {
            if board.is_solved() {
                found = true;
                if !exhaustive {
                    return true;
                }
            }
            continue;
        }

        // Single-cell endgame.
        if config.single_cell_endgame && next_piece >= data.single_cell_start {
            let num_remaining = n - next_piece;
            let saved_len = solution.len();
            if solve_single_cells(&board, data.m, data.h, data.w, num_remaining, solution) {
                found = true;
                if !exhaustive {
                    return true;
                }
                solution.truncate(saved_len);
            }
            continue;
        }

        // Pruning.
        if !prune_node(&board, data, next_piece, config) {
            continue;
        }

        // Budget check: should we split?
        budget = budget.saturating_sub(1);
        if budget == 0 {
            budget = SPLIT_BUDGET;
            if idle_count.load(Ordering::Relaxed) > 0 {
                split_work(&mut stack, solution, base_solution_len, data, config, wq);
            }
        }

        // Push new frame for next depth.
        let (next_min, next_prev) = next_dup_state(data, piece_idx, pl_idx, config);
        stack.push(build_search_frame(
            &board, data, next_piece, next_min, next_prev, config, rng,
        ));
    }

    found
}
