//! Parallel backtracking with budget-based work stealing.
//!
//! This module is only compiled on non-wasm targets.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use crate::core::board::Board;

use super::backtrack::{sort_placements_with_flavor, solve_single_cells};
use super::prune::mc::HitCounter;
use super::pruning::*;
use super::{PruningConfig, SolverData, SolveResult, format_count};

struct SearchFrame {
    board: Board,
    hits: HitCounter,
    piece_idx: usize,
    /// Sorted, filtered placement indices into data.all_placements[piece_idx].
    order: [u8; 196],
    len: u8,
    cursor: u8,
    filtered_out: usize,
}

/// Default flavor count (keep small so overhead stays modest); one top-level
/// task is seeded per flavor, so each flavor gets ≈ num_threads / FLAVORS
/// dedicated worker cycles on average.
const NUM_FLAVORS: u8 = 2;

pub(crate) struct StealableTask {
    pub board: Board,
    pub hits: HitCounter,
    pub prefix: Vec<(usize, usize)>,
    pub depth: usize,
    pub prev_placement: usize,
    /// Placement-sort flavor (0 = canonical, 1 = reversed). Controls the order
    /// this task and its descendants explore placements. Seeded at top-level with
    /// one task per flavor so worker threads pull work from both explorations.
    pub flavor: u8,
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

fn build_search_frame<const M: usize>(
    board: &Board,
    hits: HitCounter,
    data: &SolverData,
    piece_idx: usize,
    prev_placement: usize,
    _config: &PruningConfig,
    flavor: u8,
) -> SearchFrame {
    let placements = &data.all_placements[piece_idx];
    let pl_len = placements.len();

    let mut order = [0u8; 196];
    sort_placements_with_flavor(board, data.m, placements, &mut order, flavor);

    let fs = filter_state::<M>(board, data, piece_idx);

    // Filter in-place: pack surviving indices into the front of order.
    let mut len = 0u8;
    for oi in 0..pl_len {
        let pl_idx = order[oi] as usize;
        let mask = placements[pl_idx].2;
        if filter_placement(data, piece_idx, pl_idx, mask, prev_placement, &fs) {
            order[len as usize] = pl_idx as u8;
            len += 1;
        }
    }

    let filtered_out = pl_len - len as usize;
    SearchFrame { board: board.clone(), hits, piece_idx, order, len, cursor: 0, filtered_out }
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
    flavor: u8,
) {
    for (si, frame) in stack.iter_mut().enumerate() {
        if frame.cursor >= frame.len { continue; }
        let mut tasks = Vec::new();
        let mut sub_idx = 0usize;
        for ci in frame.cursor..frame.len {
            let pl_idx = frame.order[ci as usize] as usize;
            let mask = data.all_placements[frame.piece_idx][pl_idx].2;
            let mut board = frame.board.clone();
            board.apply_piece(mask);
            let mut hits = frame.hits;
            hits.apply_piece(mask);
            let depth = frame.piece_idx + 1;
            let prefix_len = base_solution_len + si;
            let mut prefix = solution_prefix[..prefix_len].to_vec();
            let (row, col, _) = data.all_placements[frame.piece_idx][pl_idx];
            prefix.push((row, col));
            let next_prev = next_prev_placement(data, frame.piece_idx, pl_idx);
            // Alternate flavor across split children to diversify exploration.
            // Parent flavor is the "natural" order; we flip every other child
            // to explore the reversed placement sort in parallel.
            let child_flavor = if sub_idx % 2 == 0 { flavor } else { flavor ^ 1 };
            sub_idx += 1;
            tasks.push(StealableTask { board, hits, prefix, depth, prev_placement: next_prev, flavor: child_flavor });
        }
        frame.cursor = frame.len;
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

fn backtrack_stealing<const M: usize>(
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
    flavor: u8,
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

    if !prune_node::<M>(initial_board, data, start_depth, config) {
        atomic_add_f64(progress, task_weight);
        return false;
    }

    let mut stack: Vec<SearchFrame> = Vec::with_capacity(n - start_depth);
    let first_frame = build_search_frame::<M>(
        initial_board, initial_hits, data, start_depth, initial_prev_placement, config, flavor,
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
        if frame.cursor >= frame.len {
            stack.pop();
            continue;
        }

        let pl_idx = frame.order[frame.cursor as usize] as usize;
        frame.cursor += 1;
        let piece_idx = frame.piece_idx;
        let mask = data.all_placements[piece_idx][pl_idx].2;

        let mut board = frame.board.clone();
        board.apply_piece(mask);

        let mut new_hits = frame.hits;
        new_hits.apply_piece(mask);
        if data.mc_prune.exceeds_hit_threshold(&new_hits, piece_idx + 1) {
            progress_local += data.progress_weights[piece_idx];
            nodes.set(nodes.get() + 1);
            continue;
        }

        let sol_depth = base_solution_len + stack.len() - 1;
        solution.truncate(sol_depth);
        let (row, col, _) = data.all_placements[piece_idx][pl_idx];
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

        if !prune_node::<M>(&board, data, next_piece, config) {
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
                split_work(&mut stack, solution, base_solution_len, data, wq, flavor);
            }
        }

        let next_prev = next_prev_placement(data, piece_idx, pl_idx);
        let new_frame = build_search_frame::<M>(
            &board, new_hits, data, next_piece, next_prev, config, flavor,
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
pub(crate) fn run_parallel<const M: usize>(
    board: &Board,
    order: &[usize],
    data: &SolverData,
    config: &PruningConfig,
    exhaustive: bool,
) -> SolveResult {
    let n = data.all_placements.len();

    let wq = WorkQueue::new();
    // Seed a single canonical (flavor=0) top-level task. When the first task
    // splits work on idle thread detection, half of the split children inherit
    // flavor=1 (reversed sort) — this gives diversity without duplicating the
    // starting exploration. See `split_work` for the alternation logic.
    wq.push(StealableTask {
        board: board.clone(),
        hits: HitCounter::new(),
        prefix: Vec::new(),
        depth: 0,
        prev_placement: usize::MAX,
        flavor: 0,
    });
    let _ = NUM_FLAVORS; // retained for future multi-flavor expansion

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

                    let found = backtrack_stealing::<M>(
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
                        task.flavor,
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
