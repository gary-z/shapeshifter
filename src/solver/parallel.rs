//! Parallel backtracking with budget-based work stealing.
//!
//! This module is only compiled on non-wasm targets.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use crate::core::bitboard::Bitboard;
use crate::core::board::Board;

use super::backtrack::{sort_placements, solve_single_cells, backtrack};
use super::prune::mc::HitCounter;
use super::pruning::*;
use super::{PruningConfig, SolverData, SolveResult, format_count};

struct SearchFrame {
    board: Board,
    hits: HitCounter,
    piece_idx: usize,
    placements: Vec<(usize, usize, usize, Bitboard)>,
    cursor: usize,
    filtered_out: usize,
}

pub(crate) struct StealableTask {
    pub board: Board,
    pub hits: HitCounter,
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
    hits: HitCounter,
    data: &SolverData,
    piece_idx: usize,
    prev_placement: usize,
    config: &PruningConfig,
) -> SearchFrame {
    let placements = &data.all_placements[piece_idx];
    let pl_len = placements.len();

    let mut order = [0u8; 196];
    sort_placements(board, data.m, placements, &mut order);

    let fs = filter_state(board, data, piece_idx);

    let mut filtered = Vec::with_capacity(pl_len);
    for oi in 0..pl_len {
        let pl_idx = order[oi] as usize;
        let (row, col, mask) = placements[pl_idx];
        if !filter_placement(data, piece_idx, pl_idx, mask, prev_placement, &fs) {
            continue;
        }
        filtered.push((pl_idx, row, col, mask));
    }

    let filtered_out = pl_len - filtered.len();
    SearchFrame { board: board.clone(), hits, piece_idx, placements: filtered, cursor: 0, filtered_out }
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
            let mut hits = frame.hits;
            hits.apply_piece(mask);
            let depth = frame.piece_idx + 1;
            let prefix_len = base_solution_len + si;
            let mut prefix = solution_prefix[..prefix_len].to_vec();
            prefix.push((row, col));
            let next_prev = next_prev_placement(data, frame.piece_idx, pl_idx);
            tasks.push(StealableTask { board, hits, prefix, depth, prev_placement: next_prev });
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

fn backtrack_stealing(
    initial_board: &Board,
    initial_hits: HitCounter,
    data: &SolverData,
    start_depth: usize,
    initial_prev_placement: usize,
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

    if !prune_node(initial_board, data, start_depth, config) {
        atomic_add_f64(progress, task_weight);
        return false;
    }

    let mut stack: Vec<SearchFrame> = Vec::with_capacity(n - start_depth);
    let first_frame = build_search_frame(
        initial_board, initial_hits, data, start_depth, initial_prev_placement, config,
    );
    let mut progress_local: f64 = 0.0;
    progress_local += first_frame.filtered_out as f64 * data.progress_weights[start_depth];
    nodes.set(nodes.get() + first_frame.filtered_out as u64);
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

        // Inline hit-count update + check (cross-module fn call not reliably inlined).
        let mut new_hits = frame.hits;
        new_hits.apply_piece(mask);
        if data.mc_prune.exceeds_hit_threshold(&new_hits, piece_idx + 1) {
            progress_local += data.progress_weights[piece_idx];
            nodes.set(nodes.get() + 1);
            continue;
        }

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

        if !prune_node(&board, data, next_piece, config) {
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
            &board, new_hits, data, next_piece, next_prev, config,
        );
        progress_local += new_frame.filtered_out as f64 * data.progress_weights[next_piece];
        nodes.set(nodes.get() + new_frame.filtered_out as u64);
        stack.push(new_frame);
    }

    if progress_local > 0.0 {
        atomic_add_f64(progress, progress_local);
    }

    found
}

/// Parallel backtrack with pre-built data.
pub(crate) fn run_parallel(
    board: &Board,
    order: &[usize],
    data: &SolverData,
    config: &PruningConfig,
    exhaustive: bool,
) -> SolveResult {
    let n = data.all_placements.len();

    let wq = WorkQueue::new();
    wq.push(StealableTask {
        board: board.clone(),
        hits: HitCounter::new(),
        prefix: Vec::new(),
        depth: 0,
        prev_placement: usize::MAX,
    });

    let num_threads = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);

    let abort = AtomicBool::new(false);
    let result: Mutex<Option<Vec<(usize, usize)>>> = Mutex::new(None);
    let total_nodes = std::sync::atomic::AtomicU64::new(0);
    let active_count = AtomicUsize::new(0);
    let idle_count = AtomicUsize::new(0);
    let progress = std::sync::atomic::AtomicU64::new(0f64.to_bits());
    let workers_alive = AtomicUsize::new(num_threads);

    let total_space: f64 = data.all_placements.iter()
        .map(|p| p.len() as f64)
        .product();
    eprintln!("search space: {:.3e}", total_space);

    let solve_start = std::time::Instant::now();

    std::thread::scope(|s| {
        s.spawn(|| {
            let bar_width = 30;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if abort.load(Ordering::Relaxed)
                    || workers_alive.load(Ordering::Relaxed) == 0 { break; }

                let p = f64::from_bits(progress.load(Ordering::Relaxed)).min(1.0);
                let pct = p * 100.0;
                let nodes_so_far = total_nodes.load(Ordering::Relaxed);
                let elapsed = solve_start.elapsed().as_secs_f64();

                let filled = (p * bar_width as f64) as usize;
                let bar: String = (0..bar_width).map(|i| if i < filled { '#' } else { ' ' }).collect();

                let nodes_str = format_count(nodes_so_far);
                eprint!("\r\x1b[K[{}] {:.1}%  {}  {:.1}s", bar, pct, nodes_str, elapsed);
            }
            eprint!("\r\x1b[K");
        });

        for _ in 0..num_threads {
            s.spawn(|| {
                let nodes = Cell::new(0u64);
                let mut solution = Vec::with_capacity(n);

                loop {
                    if abort.load(Ordering::Relaxed) { break; }

                    let task = wq.pop().or_else(|| {
                        idle_count.fetch_add(1, Ordering::Relaxed);
                        let t = wq.wait_for_task(&abort, &active_count);
                        idle_count.fetch_sub(1, Ordering::Relaxed);
                        t
                    });
                    let task = match task {
                        Some(t) => t,
                        None => break,
                    };
                    active_count.fetch_add(1, Ordering::SeqCst);

                    solution.clear();
                    solution.extend_from_slice(&task.prefix);
                    nodes.set(0);

                    let found = backtrack_stealing(
                        &task.board,
                        task.hits,
                        data,
                        task.depth,
                        task.prev_placement,
                        &mut solution,
                        &nodes,
                        config,
                        &abort,
                        &wq,
                        &idle_count,
                        exhaustive,
                        &progress,
                    );

                    active_count.fetch_sub(1, Ordering::SeqCst);
                    total_nodes.fetch_add(nodes.get(), Ordering::Relaxed);

                    if found {
                        if !exhaustive {
                            abort.store(true, Ordering::Relaxed);
                        }
                        let mut guard = result.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some(solution.clone());
                        }
                    }
                }
                workers_alive.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });

    let result = result.into_inner().unwrap();
    let nodes_visited = total_nodes.load(Ordering::Relaxed);

    let solution = result.map(|sorted_solution| {
        let mut solution = vec![(0, 0); n];
        for (sorted_idx, &(row, col)) in sorted_solution.iter().enumerate() {
            solution[order[sorted_idx]] = (row, col);
        }
        solution
    });

    let final_progress = f64::from_bits(progress.load(Ordering::Relaxed));

    SolveResult {
        solution,
        nodes_visited,
        progress: final_progress,
    }
}
