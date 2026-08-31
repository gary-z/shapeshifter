//! Parallel backtracking with budget-based work stealing.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use crate::core::board::Board;

use super::backtrack::{
    MAX_PLACEMENTS, next_previous_placement, rank_placements, solve_single_cell_suffix,
};
use super::pruning::HitCounter;
use super::pruning::{is_canonical_placement_pair, max_zero_cells_allowed, state_is_feasible};
use super::{SolveResult, SolverData, format_count, restore_piece_order};

struct SearchFrame {
    board: Board,
    hits: HitCounter,
    piece_index: usize,
    ranked_indices: [u8; MAX_PLACEMENTS],
    candidate_count: u8,
    cursor: u8,
    skipped_count: usize,
}

struct SearchTask {
    board: Board,
    hits: HitCounter,
    solution_prefix: Vec<(usize, usize)>,
    piece_index: usize,
    previous_placement: usize,
}

struct WorkQueue {
    queue: Mutex<VecDeque<SearchTask>>,
    condvar: Condvar,
}

impl WorkQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            condvar: Condvar::new(),
        }
    }

    fn push(&self, task: SearchTask) {
        self.queue.lock().unwrap().push_back(task);
        self.condvar.notify_one();
    }

    fn push_many(&self, tasks: Vec<SearchTask>) {
        if tasks.is_empty() {
            return;
        }
        let mut queue = self.queue.lock().unwrap();
        for task in tasks {
            queue.push_back(task);
        }
        self.condvar.notify_all();
    }

    fn pop(&self) -> Option<SearchTask> {
        self.queue.lock().unwrap().pop_front()
    }

    fn wait_for_task(&self, abort: &AtomicBool, active_count: &AtomicUsize) -> Option<SearchTask> {
        let mut queue = self.queue.lock().unwrap();
        loop {
            if abort.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(task) = queue.pop_front() {
                return Some(task);
            }
            if active_count.load(Ordering::SeqCst) == 0 {
                return None;
            }
            let (resumed_queue, _) = self
                .condvar
                .wait_timeout(queue, std::time::Duration::from_millis(1))
                .unwrap();
            queue = resumed_queue;
        }
    }
}

fn build_search_frame<const MODULUS: usize>(
    board: &Board,
    hits: HitCounter,
    data: &SolverData,
    piece_index: usize,
    previous_placement: usize,
) -> SearchFrame {
    let placements = &data.placements[piece_index];
    let placement_count = placements.len();

    let mut ranked_indices = [0u8; MAX_PLACEMENTS];
    let max_zero_cells = max_zero_cells_allowed::<MODULUS>(board, data, piece_index);
    let ranked_count = rank_placements(
        board,
        data.modulus,
        placements,
        max_zero_cells,
        &mut ranked_indices,
    );

    let mut candidate_count = 0u8;
    for ranked_index in 0..ranked_count {
        let placement_index = ranked_indices[ranked_index] as usize;
        if is_canonical_placement_pair(data, piece_index, placement_index, previous_placement) {
            ranked_indices[candidate_count as usize] = placement_index as u8;
            candidate_count += 1;
        }
    }

    SearchFrame {
        board: *board,
        hits,
        piece_index,
        ranked_indices,
        candidate_count,
        cursor: 0,
        skipped_count: placement_count - candidate_count as usize,
    }
}

fn split_work(
    stack: &mut [SearchFrame],
    solution_prefix: &[(usize, usize)],
    base_solution_length: usize,
    data: &SolverData,
    work_queue: &WorkQueue,
) {
    for (stack_index, frame) in stack.iter_mut().enumerate() {
        if frame.cursor >= frame.candidate_count {
            continue;
        }
        let mut tasks = Vec::new();
        for candidate_index in frame.cursor..frame.candidate_count {
            let placement_index = frame.ranked_indices[candidate_index as usize] as usize;
            let mask = data.placements[frame.piece_index][placement_index].2;
            let mut board = frame.board;
            board.apply_piece(mask);
            let mut hits = frame.hits;
            hits.apply_piece(mask);
            let next_piece_index = frame.piece_index + 1;
            let prefix_length = base_solution_length + stack_index;
            let mut task_solution = solution_prefix[..prefix_length].to_vec();
            let (row, column, _) = data.placements[frame.piece_index][placement_index];
            task_solution.push((row, column));
            let previous_placement =
                next_previous_placement(data, frame.piece_index, placement_index);
            tasks.push(SearchTask {
                board,
                hits,
                solution_prefix: task_solution,
                piece_index: next_piece_index,
                previous_placement,
            });
        }
        frame.cursor = frame.candidate_count;
        work_queue.push_many(tasks);
        return;
    }
}

const NODES_BETWEEN_SPLITS: u64 = 4096;

fn add_progress(progress: &std::sync::atomic::AtomicU64, value: f64) {
    let mut old = progress.load(Ordering::Relaxed);
    loop {
        let new_value = f64::from_bits(old) + value;
        match progress.compare_exchange_weak(
            old,
            new_value.to_bits(),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(current) => old = current,
        }
    }
}

fn backtrack_with_stealing<const MODULUS: usize>(
    initial_board: &Board,
    initial_hits: HitCounter,
    data: &SolverData,
    start_piece_index: usize,
    initial_previous_placement: usize,
    solution: &mut Vec<(usize, usize)>,
    nodes: &Cell<u64>,
    abort: &AtomicBool,
    work_queue: &WorkQueue,
    idle_count: &AtomicUsize,
    exhaustive: bool,
    progress: &std::sync::atomic::AtomicU64,
) -> bool {
    let piece_count = data.placements.len();
    let base_solution_length = solution.len();

    let task_weight = if start_piece_index < piece_count {
        data.placements[start_piece_index].len() as f64 * data.progress_weights[start_piece_index]
    } else {
        0.0
    };

    if start_piece_index == piece_count {
        return initial_board.is_solved();
    }
    if start_piece_index >= data.single_cell_suffix_start {
        let remaining_pieces = piece_count - start_piece_index;
        let result = solve_single_cell_suffix(
            initial_board,
            data.modulus,
            data.height,
            data.width,
            remaining_pieces,
            solution,
        );
        add_progress(progress, task_weight);
        return result;
    }

    if !state_is_feasible::<MODULUS>(initial_board, data, start_piece_index) {
        add_progress(progress, task_weight);
        return false;
    }

    let mut stack: Vec<SearchFrame> = Vec::with_capacity(piece_count - start_piece_index);
    let first_frame = build_search_frame::<MODULUS>(
        initial_board,
        initial_hits,
        data,
        start_piece_index,
        initial_previous_placement,
    );
    let mut accumulated_progress = 0.0;
    accumulated_progress +=
        first_frame.skipped_count as f64 * data.progress_weights[start_piece_index];
    nodes.set(nodes.get() + first_frame.skipped_count as u64);
    stack.push(first_frame);

    let mut nodes_until_split = NODES_BETWEEN_SPLITS;
    let mut found = false;
    let mut first_solution: Option<Vec<(usize, usize)>> = None;

    loop {
        if abort.load(Ordering::Relaxed) {
            break;
        }
        if stack.is_empty() {
            break;
        }

        let frame = stack.last_mut().unwrap();
        if frame.cursor >= frame.candidate_count {
            stack.pop();
            continue;
        }

        let placement_index = frame.ranked_indices[frame.cursor as usize] as usize;
        frame.cursor += 1;
        let piece_index = frame.piece_index;
        let mask = data.placements[piece_index][placement_index].2;

        let mut board = frame.board;
        board.apply_piece(mask);

        let mut hits_after_placement = frame.hits;
        hits_after_placement.apply_piece(mask);
        if data
            .monte_carlo
            .exceeds_hit_threshold(&hits_after_placement, piece_index + 1)
        {
            accumulated_progress += data.progress_weights[piece_index];
            nodes.set(nodes.get() + 1);
            continue;
        }

        let solution_depth = base_solution_length + stack.len() - 1;
        solution.truncate(solution_depth);
        let (row, column, _) = data.placements[piece_index][placement_index];
        solution.push((row, column));

        nodes.set(nodes.get() + 1);

        let next_piece_index = piece_index + 1;

        if next_piece_index == piece_count {
            accumulated_progress += data.progress_weights[piece_index];
            if board.is_solved() {
                found = true;
                if !exhaustive {
                    add_progress(progress, accumulated_progress);
                    return true;
                }
                if first_solution.is_none() {
                    first_solution = Some(solution.clone());
                }
            }
            continue;
        }

        if next_piece_index >= data.single_cell_suffix_start {
            accumulated_progress += data.progress_weights[piece_index];
            let remaining_pieces = piece_count - next_piece_index;
            let solution_length = solution.len();
            if solve_single_cell_suffix(
                &board,
                data.modulus,
                data.height,
                data.width,
                remaining_pieces,
                solution,
            ) {
                found = true;
                if !exhaustive {
                    add_progress(progress, accumulated_progress);
                    return true;
                }
                if first_solution.is_none() {
                    first_solution = Some(solution.clone());
                }
                solution.truncate(solution_length);
            }
            continue;
        }

        if !state_is_feasible::<MODULUS>(&board, data, next_piece_index) {
            accumulated_progress += data.progress_weights[piece_index];
            continue;
        }

        nodes_until_split = nodes_until_split.saturating_sub(1);
        if nodes_until_split == 0 {
            nodes_until_split = NODES_BETWEEN_SPLITS;
            if accumulated_progress > 0.0 {
                add_progress(progress, accumulated_progress);
                accumulated_progress = 0.0;
            }
            if idle_count.load(Ordering::Relaxed) > 0 {
                split_work(&mut stack, solution, base_solution_length, data, work_queue);
            }
        }

        let previous_placement = next_previous_placement(data, piece_index, placement_index);
        let next_frame = build_search_frame::<MODULUS>(
            &board,
            hits_after_placement,
            data,
            next_piece_index,
            previous_placement,
        );
        accumulated_progress +=
            next_frame.skipped_count as f64 * data.progress_weights[next_piece_index];
        nodes.set(nodes.get() + next_frame.skipped_count as u64);
        stack.push(next_frame);
    }

    if accumulated_progress > 0.0 {
        add_progress(progress, accumulated_progress);
    }

    if let Some(first_solution) = first_solution {
        solution.clear();
        solution.extend_from_slice(&first_solution);
    }

    found
}

pub(super) fn solve_parallel<const MODULUS: usize>(
    board: &Board,
    piece_order: &[usize],
    data: &SolverData,
    exhaustive: bool,
) -> SolveResult {
    let piece_count = data.placements.len();

    let work_queue = WorkQueue::new();
    work_queue.push(SearchTask {
        board: *board,
        hits: HitCounter::new(),
        solution_prefix: Vec::new(),
        piece_index: 0,
        previous_placement: usize::MAX,
    });

    let thread_count = std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(4);

    let abort = AtomicBool::new(false);
    let first_solution: Mutex<Option<Vec<(usize, usize)>>> = Mutex::new(None);
    let total_nodes = std::sync::atomic::AtomicU64::new(0);
    let active_count = AtomicUsize::new(0);
    let idle_count = AtomicUsize::new(0);
    let progress = std::sync::atomic::AtomicU64::new(0f64.to_bits());
    let workers_alive = AtomicUsize::new(thread_count);

    let total_space: f64 = data
        .placements
        .iter()
        .map(|placements| placements.len() as f64)
        .product();
    eprintln!("search space: {:.3e}", total_space);

    let solve_start = std::time::Instant::now();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let bar_width = 30;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if abort.load(Ordering::Relaxed) || workers_alive.load(Ordering::Relaxed) == 0 {
                    break;
                }

                let progress_fraction = f64::from_bits(progress.load(Ordering::Relaxed)).min(1.0);
                let percentage = progress_fraction * 100.0;
                let nodes_so_far = total_nodes.load(Ordering::Relaxed);
                let elapsed = solve_start.elapsed().as_secs_f64();

                let filled = (progress_fraction * bar_width as f64) as usize;
                let bar: String = (0..bar_width)
                    .map(|position| if position < filled { '#' } else { ' ' })
                    .collect();

                let nodes_str = format_count(nodes_so_far);
                eprint!(
                    "\r\x1b[K[{}] {:.1}%  {}  {:.1}s",
                    bar, percentage, nodes_str, elapsed
                );
            }
            eprint!("\r\x1b[K");
        });

        for _ in 0..thread_count {
            scope.spawn(|| {
                let nodes = Cell::new(0u64);
                let mut solution = Vec::with_capacity(piece_count);

                loop {
                    if abort.load(Ordering::Relaxed) {
                        break;
                    }

                    let task = work_queue.pop().or_else(|| {
                        idle_count.fetch_add(1, Ordering::Relaxed);
                        let task = work_queue.wait_for_task(&abort, &active_count);
                        idle_count.fetch_sub(1, Ordering::Relaxed);
                        task
                    });
                    let task = match task {
                        Some(task) => task,
                        None => break,
                    };
                    active_count.fetch_add(1, Ordering::SeqCst);

                    solution.clear();
                    solution.extend_from_slice(&task.solution_prefix);
                    nodes.set(0);

                    let found = backtrack_with_stealing::<MODULUS>(
                        &task.board,
                        task.hits,
                        data,
                        task.piece_index,
                        task.previous_placement,
                        &mut solution,
                        &nodes,
                        &abort,
                        &work_queue,
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
                        let mut guard = first_solution.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some(solution.clone());
                        }
                    }
                }
                workers_alive.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });

    let first_solution = first_solution.into_inner().unwrap();
    let nodes_visited = total_nodes.load(Ordering::Relaxed);

    let solution =
        first_solution.map(|sorted_solution| restore_piece_order(&sorted_solution, piece_order));

    let final_progress = f64::from_bits(progress.load(Ordering::Relaxed));

    SolveResult {
        solution,
        nodes_visited,
        progress: final_progress,
    }
}
