use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::subgame::state::SubgameState;

use super::pruning::*;
use super::{PruningConfig, SolverData};

/// Try to solve remaining pieces when they're all 1x1.
/// Each cell at deficit d needs d more hits to reach 0. Total hits must equal number of pieces.
/// Returns true and fills solution if solvable.
pub(crate) fn solve_single_cells(
    board: &Board,
    m: u8,
    h: u8,
    w: u8,
    num_pieces: usize,
    solution: &mut Vec<(usize, usize)>,
) -> bool {
    // Count total deficit remaining and verify it matches available pieces.
    let mut needed = 0u32;
    for d in 1..m {
        needed += d as u32 * board.plane(d).count_ones();
    }
    if needed as usize != num_pieces {
        return false;
    }

    // Assign pieces to cells: for each non-zero cell, emit (deficit) placements.
    let base_len = solution.len();
    for r in 0..h as usize {
        for c in 0..w as usize {
            let deficit = board.get(r, c) as usize;
            if deficit != 0 {
                for _ in 0..deficit {
                    solution.push((r, c));
                }
            }
        }
    }

    debug_assert_eq!(solution.len() - base_len, num_pieces);
    true
}

/// Generate backtrack functions with and without abort support.
macro_rules! define_backtrack {
    ($name:ident $(, abort: $abort_param:ident : $abort_ty:ty)?) => {
        pub(crate) fn $name(
            board: &Board,
            data: &SolverData,
            piece_idx: usize,
            prev_placement: usize,
            sg_state: SubgameState,
            solution: &mut Vec<(usize, usize)>,
            nodes: &Cell<u64>,
            config: &PruningConfig,
            $($abort_param: $abort_ty,)?
        ) -> bool {
            nodes.set(nodes.get() + 1);

            $(
                if nodes.get() & 1023 == 0 && $abort_param.load(Ordering::Relaxed) {
                    return false;
                }
            )?

            if piece_idx == data.all_placements.len() {
                return board.is_solved();
            }

            if config.single_cell_endgame && piece_idx >= data.single_cell_start {
                let num_remaining = data.all_placements.len() - piece_idx;
                return solve_single_cells(board, data.m, data.h, data.w, num_remaining, solution);
            }

            if !prune_node(board, data, piece_idx, config, &sg_state) { return false; }

            // Subgame placement filter.
            let (sg_valid_rows, sg_valid_cols) = if config.subgame {
                let v = data.subgame_prune.valid_positions(&sg_state, piece_idx);
                if v.0 == 0 || v.1 == 0 { return false; } // no valid placements
                v
            } else {
                (u16::MAX, u16::MAX)
            };

            let locked_mask = if config.cell_locking {
                board.plane(0) & !data.suffix_coverage[piece_idx].coverage_ge(data.m)
            } else {
                Bitboard::ZERO
            };

            let placements = &data.all_placements[piece_idx];

            let zero_plane = board.plane(0);
            let pl_len = placements.len();
            let mut order = [0u8; 196];
            let mut keys = [0u8; 196];
            for i in 0..pl_len {
                keys[i] = (placements[i].2 & zero_plane).count_ones() as u8;
            }
            let mut counts = [0u8; 26];
            for i in 0..pl_len { counts[keys[i] as usize] += 1; }
            let mut offsets = [0u8; 26];
            for i in 1..26 { offsets[i] = offsets[i - 1] + counts[i - 1]; }
            for i in 0..pl_len {
                let k = keys[i] as usize;
                order[offsets[k] as usize] = i as u8;
                offsets[k] += 1;
            }

            let board_snapshot = *board;
            for oi in 0..pl_len {
                let pl_idx = order[oi] as usize;
                let (row, col, mask) = placements[pl_idx];

                if !(mask & locked_mask).is_zero() {
                    continue;
                }

                if sg_valid_rows & (1 << row) == 0 || sg_valid_cols & (1 << col) == 0 {
                    continue;
                }

                if prev_placement < usize::MAX {
                    if let Some(ref table) = data.skip_tables[piece_idx] {
                        let num_curr = placements.len();
                        if table[prev_placement * num_curr + pl_idx] {
                            continue;
                        }
                    }
                }

                let mut board = board_snapshot;
                board.apply_piece(mask);
                let next_sg = if config.subgame {
                    let mut sg = sg_state;
                    sg.apply_piece(&data.subgame_prune.data, piece_idx, row, col);
                    sg
                } else {
                    sg_state
                };
                solution.push((row, col));

                let next_prev = if piece_idx + 1 < data.all_placements.len()
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
                    next_prev,
                    next_sg,
                    solution,
                    nodes,
                    config,
                    $($abort_param,)?
                ) {
                    return true;
                }

                solution.pop();
            }

            false
        }
    };
}

// Serial backtrack: no abort parameter, no overhead.
define_backtrack!(backtrack);

// ---------------------------------------------------------------------------
// Iterative backtrack with budget-based work stealing
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::sync::atomic::AtomicUsize;

struct SearchFrame {
    board: Board,
    sg_state: SubgameState,
    piece_idx: usize,
    placements: Vec<(usize, usize, usize, Bitboard)>,
    cursor: usize,
}

pub(crate) struct StealableTask {
    pub board: Board,
    pub sg_state: SubgameState,
    pub prefix: Vec<(usize, usize)>,
    pub depth: usize,
    pub prev_placement: usize,
}

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
        for t in tasks { q.push_back(t); }
        self.condvar.notify_all();
    }

    pub fn pop(&self) -> Option<StealableTask> {
        self.queue.lock().unwrap().pop_front()
    }

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
            let (new_q, _) = self.condvar.wait_timeout(
                q, std::time::Duration::from_millis(1)
            ).unwrap();
            q = new_q;
        }
    }
}

fn build_search_frame(
    board: &Board,
    data: &SolverData,
    piece_idx: usize,
    prev_placement: usize,
    config: &PruningConfig,
    sg_state: SubgameState,
) -> SearchFrame {
    let placements = &data.all_placements[piece_idx];
    let pl_len = placements.len();

    let (sg_valid_rows, sg_valid_cols) = if config.subgame {
        let v = data.subgame_prune.valid_positions(&sg_state, piece_idx);
        if v.0 == 0 || v.1 == 0 {
            return SearchFrame { board: board.clone(), sg_state, piece_idx, placements: vec![], cursor: 0 };
        }
        v
    } else {
        (u16::MAX, u16::MAX)
    };

    let locked_mask = if config.cell_locking {
        board.plane(0) & !data.suffix_coverage[piece_idx].coverage_ge(data.m)
    } else {
        Bitboard::ZERO
    };

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
    for i in 0..pl_len {
        let k = keys[i] as usize;
        order[offsets[k] as usize] = i as u8;
        offsets[k] += 1;
    }

    let mut filtered = Vec::with_capacity(pl_len);
    for oi in 0..pl_len {
        let pl_idx = order[oi] as usize;
        let (row, col, mask) = placements[pl_idx];
        if !(mask & locked_mask).is_zero() { continue; }
        if sg_valid_rows & (1 << row) == 0 || sg_valid_cols & (1 << col) == 0 { continue; }
        if prev_placement < usize::MAX {
            if let Some(ref table) = data.skip_tables[piece_idx] {
                let num_curr = placements.len();
                if table[prev_placement * num_curr + pl_idx] { continue; }
            }
        }
        filtered.push((pl_idx, row, col, mask));
    }

    SearchFrame { board: board.clone(), sg_state, piece_idx, placements: filtered, cursor: 0 }
}

#[inline]
fn next_prev_placement(data: &SolverData, piece_idx: usize, pl_idx: usize) -> usize {
    let next = piece_idx + 1;
    if next < data.all_placements.len() && data.skip_tables[next].is_some() { pl_idx } else { usize::MAX }
}

fn split_work(
    stack: &mut [SearchFrame],
    solution_prefix: &[(usize, usize)],
    base_solution_len: usize,
    data: &SolverData,
    wq: &WorkQueue,
) {
    for (si, frame) in stack.iter_mut().enumerate() {
        if frame.cursor >= frame.placements.len() { continue; }
        let mut tasks = Vec::new();
        for ci in frame.cursor..frame.placements.len() {
            let (pl_idx, row, col, mask) = frame.placements[ci];
            let mut board = frame.board.clone();
            board.apply_piece(mask);
            let mut sg = frame.sg_state;
            sg.apply_piece(&data.subgame_prune.data, frame.piece_idx, row, col);
            let depth = frame.piece_idx + 1;
            let prefix_len = base_solution_len + si;
            let mut prefix = solution_prefix[..prefix_len].to_vec();
            prefix.push((row, col));
            let next_prev = next_prev_placement(data, frame.piece_idx, pl_idx);
            tasks.push(StealableTask { board, sg_state: sg, prefix, depth, prev_placement: next_prev });
        }
        frame.cursor = frame.placements.len();
        wq.push_many(tasks);
        return;
    }
}

const SPLIT_BUDGET: u64 = 4096;

fn atomic_add_f64(atomic: &std::sync::atomic::AtomicU64, val: f64) {
    let mut old = atomic.load(Ordering::Relaxed);
    loop {
        let new_val = f64::from_bits(old) + val;
        match atomic.compare_exchange_weak(old, new_val.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(x) => old = x,
        }
    }
}

pub(crate) fn backtrack_stealing(
    initial_board: &Board,
    data: &SolverData,
    start_depth: usize,
    initial_prev_placement: usize,
    initial_sg_state: SubgameState,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    config: &PruningConfig,
    abort: &AtomicBool,
    wq: &WorkQueue,
    idle_count: &AtomicUsize,
    exhaustive: bool,
    progress: &std::sync::atomic::AtomicU64,
) -> bool {
    let n = data.all_placements.len();
    let base_solution_len = solution.len();

    let task_weight = if start_depth < n {
        data.all_placements[start_depth].len() as f64 * data.progress_weights[start_depth]
    } else {
        0.0
    };

    if start_depth == n {
        return initial_board.is_solved();
    }
    if config.single_cell_endgame && start_depth >= data.single_cell_start {
        let num_remaining = n - start_depth;
        let result = solve_single_cells(initial_board, data.m, data.h, data.w, num_remaining, solution);
        atomic_add_f64(progress, task_weight);
        return result;
    }

    if !prune_node(initial_board, data, start_depth, config, &initial_sg_state) {
        atomic_add_f64(progress, task_weight);
        return false;
    }

    let mut stack: Vec<SearchFrame> = Vec::with_capacity(n - start_depth);
    let first_frame = build_search_frame(
        initial_board, data, start_depth, initial_prev_placement, config, initial_sg_state,
    );
    let mut progress_local: f64 = 0.0;
    let filtered_out = data.all_placements[start_depth].len() - first_frame.placements.len();
    progress_local += filtered_out as f64 * data.progress_weights[start_depth];
    stack.push(first_frame);

    let mut budget = SPLIT_BUDGET;
    let mut found = false;

    loop {
        if abort.load(Ordering::Relaxed) { break; }
        if stack.is_empty() { break; }

        let frame = stack.last_mut().unwrap();
        if frame.cursor >= frame.placements.len() {
            stack.pop();
            continue;
        }

        let (pl_idx, row, col, mask) = frame.placements[frame.cursor];
        frame.cursor += 1;
        let piece_idx = frame.piece_idx;

        let mut board = frame.board.clone();
        board.apply_piece(mask);
        let sg = if config.subgame {
            let mut s = frame.sg_state;
            s.apply_piece(&data.subgame_prune.data, piece_idx, row, col);
            s
        } else {
            frame.sg_state
        };

        let sol_depth = base_solution_len + stack.len() - 1;
        solution.truncate(sol_depth);
        solution.push((row, col));

        nodes.set(nodes.get() + 1);

        let next_piece = piece_idx + 1;

        if next_piece == n {
            progress_local += data.progress_weights[piece_idx];
            if board.is_solved() {
                found = true;
                if !exhaustive {
                    atomic_add_f64(progress, progress_local);
                    return true;
                }
            }
            continue;
        }

        if config.single_cell_endgame && next_piece >= data.single_cell_start {
            progress_local += data.progress_weights[piece_idx];
            let num_remaining = n - next_piece;
            let saved_len = solution.len();
            if solve_single_cells(&board, data.m, data.h, data.w, num_remaining, solution) {
                found = true;
                if !exhaustive {
                    atomic_add_f64(progress, progress_local);
                    return true;
                }
                solution.truncate(saved_len);
            }
            continue;
        }

        if !prune_node(&board, data, next_piece, config, &sg) {
            progress_local += data.progress_weights[piece_idx];
            continue;
        }

        budget = budget.saturating_sub(1);
        if budget == 0 {
            budget = SPLIT_BUDGET;
            if progress_local > 0.0 {
                atomic_add_f64(progress, progress_local);
                progress_local = 0.0;
            }
            if idle_count.load(Ordering::Relaxed) > 0 {
                split_work(&mut stack, solution, base_solution_len, data, wq);
            }
        }

        let next_prev = next_prev_placement(data, piece_idx, pl_idx);
        let new_frame = build_search_frame(
            &board, data, next_piece, next_prev, config, sg,
        );
        let filtered_out = data.all_placements[next_piece].len() - new_frame.placements.len();
        progress_local += filtered_out as f64 * data.progress_weights[next_piece];
        stack.push(new_frame);
    }

    if progress_local > 0.0 {
        atomic_add_f64(progress, progress_local);
    }

    found
}
