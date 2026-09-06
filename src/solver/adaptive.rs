//! Backtracking with dynamic placement domains and modular count propagation.
//!
//! Bounds use the placements still available to each piece, including mandatory
//! and possible cell coverage. Single-cell marginals guide variable and value
//! ordering; they never remove a placement. Workers vary that ordering and use
//! growing restart budgets. Returned solutions are checked independently.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rand::{RngExt, SeedableRng};

use crate::core::{STRIDE, bitboard::Bitboard};
use crate::game::Game;

use super::Solution;

struct Placement {
    position: (usize, usize),
    cells: Vec<usize>,
    mask: Bitboard,
}

type Domains = Vec<Vec<usize>>;

pub(super) struct AdaptiveSearch {
    options: Vec<Vec<Placement>>,
    target: Vec<u8>,
    modulus: usize,
}

struct SharedSearch {
    deadline: Instant,
    stop: AtomicBool,
    solution: Mutex<Option<Solution>>,
    nodes: AtomicU64,
}

struct Worker<'a> {
    data: &'a AdaptiveSearch,
    shared: &'a SharedSearch,
    rng: rand_mt::Mt64,
    exploration_rng: rand::rngs::SmallRng,
    id: usize,
    restart: usize,
    nodes: u64,
    limit: u64,
}

/// Whether an integer in the inclusive interval has the required residue.
fn residue_is_possible(lower: usize, upper: usize, target: usize, modulus: usize) -> bool {
    lower + (target + modulus - lower % modulus) % modulus <= upper
}

impl AdaptiveSearch {
    pub(super) fn precompute(game: &Game) -> Self {
        let board = game.board();
        let height = usize::from(board.height());
        let width = usize::from(board.width());
        let target = (0..height * width)
            .map(|cell| board.get(cell / width, cell % width))
            .collect();
        let options = game
            .pieces()
            .iter()
            .map(|piece| {
                piece
                    .placements(board.height(), board.width())
                    .into_iter()
                    .map(|(row, column, placed)| {
                        let cells = (0..height * width)
                            .filter(|&cell| {
                                placed.get_bit((cell / width * STRIDE + cell % width) as u32)
                            })
                            .collect::<Vec<_>>();
                        let mut mask = Bitboard::ZERO;
                        for &cell in &cells {
                            mask.set_bit(cell as u32);
                        }
                        Placement {
                            position: (row, column),
                            cells,
                            mask,
                        }
                    })
                    .collect()
            })
            .collect();
        Self {
            options,
            target,
            modulus: usize::from(board.m()),
        }
    }

    pub(super) fn solve(&self, budget: Duration, workers: usize) -> (Option<Solution>, u64) {
        if self.options.iter().any(Vec::is_empty) {
            return (None, 0);
        }
        let shared = SharedSearch {
            deadline: Instant::now() + budget,
            stop: AtomicBool::new(false),
            solution: Mutex::new(None),
            nodes: AtomicU64::new(0),
        };
        std::thread::scope(|scope| {
            for id in 0..workers {
                let shared = &shared;
                scope.spawn(move || {
                    Worker {
                        data: self,
                        shared,
                        rng: rand_mt::Mt64::new(1_234_567 + id as u64 * 99_991),
                        exploration_rng: rand::rngs::SmallRng::seed_from_u64(
                            1_234_567 + id as u64 * 99_991,
                        ),
                        id,
                        restart: 0,
                        nodes: 0,
                        limit: 0,
                    }
                    .run();
                });
            }
        });
        (
            shared.solution.into_inner().unwrap(),
            shared.nodes.into_inner(),
        )
    }

    fn domains(&self) -> Domains {
        self.options
            .iter()
            .map(|options| (0..options.len()).collect())
            .collect()
    }

    /// Reach a fixed point of sound domain filtering. A singleton domain is a
    /// fixed placement. Bounds are recomputed whenever a domain changes.
    fn propagate(&self, domains: &mut Domains) -> Option<Vec<u8>> {
        let pieces = self.options.len();
        let cells = self.target.len();
        loop {
            let mut residual = self.target.clone();
            let mut area = 0;
            for (piece, domain) in domains.iter().enumerate() {
                if domain.is_empty() {
                    return None;
                }
                if domain.len() == 1 {
                    for &cell in &self.options[piece][domain[0]].cells {
                        residual[cell] =
                            (residual[cell] + self.modulus as u8 - 1) % self.modulus as u8;
                    }
                } else {
                    area += self.options[piece][0].cells.len();
                }
            }
            let deficit = residual
                .iter()
                .map(|&value| usize::from(value))
                .sum::<usize>();
            if area < deficit || !(area - deficit).is_multiple_of(self.modulus) {
                return None;
            }
            let wraps = (area - deficit) / self.modulus;
            let mut zeros = Bitboard::ZERO;
            for (cell, &value) in residual.iter().enumerate() {
                if value == 0 {
                    zeros.set_bit(cell as u32);
                }
            }
            let mut changed = false;
            let mut possible = vec![Bitboard::ZERO; pieces];
            let mut forced = vec![Bitboard::ZERO; pieces];
            let mut lower = vec![0; cells];
            let mut upper = vec![0; cells];
            for (piece, domain) in domains.iter_mut().enumerate() {
                if domain.len() <= 1 {
                    continue;
                }
                let previous = domain.len();
                domain.retain(|&placement| {
                    (self.options[piece][placement].mask & zeros).count_ones() as usize <= wraps
                });
                changed |= domain.len() != previous;
                let &first = domain.first()?;
                forced[piece] = self.options[piece][first].mask;
                for &placement in domain.iter() {
                    possible[piece] |= self.options[piece][placement].mask;
                    forced[piece] &= self.options[piece][placement].mask;
                }
                for cell in 0..cells {
                    lower[cell] += usize::from(forced[piece].get_bit(cell as u32));
                    upper[cell] += usize::from(possible[piece].get_bit(cell as u32));
                }
            }
            if changed {
                continue;
            }
            for cell in 0..cells {
                if !residue_is_possible(
                    lower[cell],
                    upper[cell],
                    usize::from(residual[cell]),
                    self.modulus,
                ) {
                    return None;
                }
            }
            for (piece, domain) in domains.iter_mut().enumerate() {
                if domain.len() <= 1 {
                    continue;
                }
                let mut required = Bitboard::ZERO;
                let mut forbidden = Bitboard::ZERO;
                for cell in 0..cells {
                    if !possible[piece].get_bit(cell as u32) {
                        continue;
                    }
                    let lower = lower[cell] - usize::from(forced[piece].get_bit(cell as u32));
                    let upper = upper[cell] - 1;
                    let target = usize::from(residual[cell]);
                    if !residue_is_possible(lower, upper, target, self.modulus) {
                        required.set_bit(cell as u32);
                    }
                    if !residue_is_possible(lower + 1, upper + 1, target, self.modulus) {
                        forbidden.set_bit(cell as u32);
                    }
                }
                let previous = domain.len();
                domain.retain(|&placement| {
                    let mask = self.options[piece][placement].mask;
                    mask & required == required && (mask & forbidden).is_zero()
                });
                changed |= domain.len() != previous;
                if domain.is_empty() {
                    return None;
                }
            }
            if !changed {
                return Some(residual);
            }
        }
    }

    /// Rank placements by the modular coverage distribution of the other
    /// unfixed pieces, treating their current domains as independent. These
    /// probabilities guide search only; domain filtering above is exact.
    fn scores(&self, domains: &Domains, residual: &[u8]) -> Vec<Vec<f64>> {
        let pieces = self.options.len();
        let cells = self.target.len();
        let modulus = self.modulus;
        let mut probability = vec![vec![0.0; cells]; pieces];
        let mut result = vec![Vec::new(); pieces];
        for (piece, domain) in domains.iter().enumerate() {
            if domain.len() > 1 {
                for &placement in domain {
                    for &cell in &self.options[piece][placement].cells {
                        probability[piece][cell] += 1.0 / domain.len() as f64;
                    }
                }
                result[piece] = vec![0.0; domain.len()];
            }
        }
        let mut delta = vec![vec![0.0; cells]; pieces];
        for (cell, &target) in residual.iter().enumerate() {
            let target = usize::from(target);
            let mut prefix = vec![[0.0; 5]; pieces + 1];
            let mut suffix = vec![[0.0; 5]; pieces + 1];
            prefix[0][0] = 1.0;
            suffix[pieces][0] = 1.0;
            for piece in 0..pieces {
                let hit = probability[piece][cell];
                for residue in 0..modulus {
                    prefix[piece + 1][residue] = prefix[piece][residue] * (1.0 - hit)
                        + prefix[piece][(residue + modulus - 1) % modulus] * hit;
                }
            }
            for piece in (0..pieces).rev() {
                let hit = probability[piece][cell];
                for residue in 0..modulus {
                    suffix[piece][residue] = suffix[piece + 1][residue] * (1.0 - hit)
                        + suffix[piece + 1][(residue + modulus - 1) % modulus] * hit;
                }
            }
            for (piece, domain) in domains.iter().enumerate() {
                if domain.len() <= 1 {
                    continue;
                }
                let mut uncovered = 0.0;
                let mut covered = 0.0;
                for residue in 0..modulus {
                    uncovered += prefix[piece][residue]
                        * suffix[piece + 1][(target + modulus - residue) % modulus];
                    covered += prefix[piece][residue]
                        * suffix[piece + 1][(target + 2 * modulus - 1 - residue) % modulus];
                }
                delta[piece][cell] = covered.max(1e-12).ln() - uncovered.max(1e-12).ln();
            }
        }
        for (piece, domain) in domains.iter().enumerate() {
            if domain.len() > 1 {
                for (candidate, &placement) in domain.iter().enumerate() {
                    result[piece][candidate] = self.options[piece][placement]
                        .cells
                        .iter()
                        .map(|&cell| delta[piece][cell])
                        .sum();
                }
            }
        }
        result
    }

    fn verified_solution(&self, domains: &Domains) -> Option<Solution> {
        let mut hits = vec![0; self.target.len()];
        for (piece, domain) in domains.iter().enumerate() {
            if domain.len() != 1 {
                return None;
            }
            for &cell in &self.options[piece][domain[0]].cells {
                hits[cell] += 1;
            }
        }
        hits.iter()
            .zip(&self.target)
            .all(|(&hits, &target)| hits % self.modulus == usize::from(target))
            .then(|| {
                domains
                    .iter()
                    .enumerate()
                    .map(|(piece, domain)| self.options[piece][domain[0]].position)
                    .collect()
            })
    }
}

impl Worker<'_> {
    fn random_unit(&mut self) -> f64 {
        // Independent streams preserve different search paths through ties and
        // randomized restarts. Each stream keeps its own state and budgets.
        if self.restart.is_multiple_of(2) {
            (self.rng.next_u64() as f64 / (u64::MAX as f64 + 1.0)).min(1.0 - f64::EPSILON / 2.0)
        } else {
            self.exploration_rng.random()
        }
    }

    fn interrupted(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed) || Instant::now() >= self.shared.deadline
    }

    fn search(&mut self, mut domains: Domains) -> bool {
        if self.interrupted() {
            return false;
        }
        self.nodes += 1;
        if self.nodes > self.limit {
            return false;
        }
        let Some(residual) = self.data.propagate(&mut domains) else {
            return false;
        };
        if domains.iter().all(|domain| domain.len() == 1) {
            if let Some(solution) = self.data.verified_solution(&domains) {
                let mut saved = self.shared.solution.lock().unwrap();
                if saved.is_none() {
                    *saved = Some(solution);
                }
                self.shared.stop.store(true, Ordering::Relaxed);
                return true;
            }
            return false;
        }
        let scores = self.data.scores(&domains, &residual);
        let mut selected = 0;
        let mut priority = f64::NEG_INFINITY;
        for (piece, domain) in domains.iter().enumerate() {
            if domain.len() <= 1 {
                continue;
            }
            let value = match self.id % 3 {
                0 => {
                    let maximum = scores[piece]
                        .iter()
                        .copied()
                        .fold(f64::NEG_INFINITY, f64::max);
                    1.0 / scores[piece]
                        .iter()
                        .map(|&value| (value - maximum).exp())
                        .sum::<f64>()
                }
                1 => -(domain.len() as f64),
                _ => {
                    -(domain.len() as f64) / (self.data.options[piece][0].cells.len() as f64).sqrt()
                }
            } + self.random_unit() * if self.restart < 2 { 0.0001 } else { 0.2 };
            if value > priority {
                priority = value;
                selected = piece;
            }
        }
        let mut choices = domains[selected]
            .iter()
            .zip(&scores[selected])
            .map(|(&placement, &score)| {
                (
                    score + self.random_unit() * if self.restart < 2 { 0.01 } else { 2.0 },
                    placement,
                )
            })
            .collect::<Vec<_>>();
        choices.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        for (_, placement) in choices {
            domains[selected] = vec![placement];
            if self.search(domains.clone()) {
                return true;
            }
            if self.nodes >= self.limit || self.interrupted() {
                break;
            }
        }
        false
    }

    fn run(&mut self) {
        while !self.interrupted() {
            self.nodes = 0;
            self.limit = (256 << (self.restart / 6).min(6)) * (1 + self.id % 4) as u64;
            self.search(self.data.domains());
            self.shared.nodes.fetch_add(self.nodes, Ordering::Relaxed);
            self.restart += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{board::Board, piece::Piece};

    #[test]
    fn residue_bounds_match_integer_enumeration() {
        for modulus in 2..=5 {
            for lower in 0..=36 {
                for upper in lower..=36 {
                    for target in 0..modulus {
                        assert_eq!(
                            residue_is_possible(lower, upper, target, modulus),
                            (lower..=upper).any(|value| value % modulus == target),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn propagation_forces_a_modular_wrap() {
        let piece = Piece::from_grid(&[&[true]]);
        for modulus in 2..=5 {
            let board = Board::from_grid(&[&[0, 0, 0], &[0, 0, 0], &[0, 0, 0]], modulus);
            let game = Game::new(board, vec![piece; usize::from(modulus)]);
            let data = AdaptiveSearch::precompute(&game);
            let mut domains = data.domains();
            domains[0] = vec![4];
            assert!(data.propagate(&mut domains).is_some());
            assert!(domains.iter().all(|domain| domain == &[4]));
            let solution = data.verified_solution(&domains).unwrap();
            let mut replay = board;
            for (row, column) in solution {
                replay.apply_piece(piece.placed_at(row, column));
            }
            assert!(replay.is_solved());
        }
    }

    #[test]
    fn propagation_preserves_every_bruteforce_solution() {
        let pieces = vec![
            Piece::from_grid(&[&[true, true]]),
            Piece::from_grid(&[&[true], &[true]]),
            Piece::from_grid(&[&[true, false], &[true, true]]),
        ];
        for modulus in 2..=5 {
            for positions in [[0, 0, 0], [2, 3, 1], [5, 5, 3]] {
                let blank = Board::from_grid(&[&[0, 0, 0], &[0, 0, 0], &[0, 0, 0]], modulus);
                let placements = pieces
                    .iter()
                    .map(|piece| piece.placements(3, 3))
                    .collect::<Vec<_>>();
                let mut grid = [[0; 3]; 3];
                for (piece, &placement) in positions.iter().enumerate() {
                    for (row, values) in grid.iter_mut().enumerate() {
                        for (column, value) in values.iter_mut().enumerate() {
                            *value = (*value
                                + u8::from(
                                    placements[piece][placement]
                                        .2
                                        .get_bit((row * STRIDE + column) as u32),
                                ))
                                % modulus;
                        }
                    }
                }
                let board = Board::from_grid(&[&grid[0], &grid[1], &grid[2]], modulus);
                let game = Game::new(board, pieces.clone());
                let data = AdaptiveSearch::precompute(&game);
                let original = data.domains();
                let mut solutions = Vec::new();
                for &a in &original[0] {
                    for &b in &original[1] {
                        for &c in &original[2] {
                            let selection = [a, b, c];
                            let mut replay = board;
                            for (piece, &placement) in selection.iter().enumerate() {
                                replay.apply_piece(placements[piece][placement].2);
                            }
                            if replay == blank {
                                solutions.push(selection);
                            }
                        }
                    }
                }
                assert!(!solutions.is_empty());
                let mut cases = vec![original.clone()];
                for (piece, domain) in original.iter().enumerate() {
                    for &placement in domain {
                        let mut restricted = original.clone();
                        restricted[piece] = vec![placement];
                        cases.push(restricted);
                    }
                }
                for mut domains in cases {
                    let expected = solutions
                        .iter()
                        .filter(|selection| {
                            selection
                                .iter()
                                .enumerate()
                                .all(|(piece, placement)| domains[piece].contains(placement))
                        })
                        .collect::<Vec<_>>();
                    let feasible = data.propagate(&mut domains).is_some();
                    for selection in expected {
                        assert!(feasible);
                        for (piece, placement) in selection.iter().enumerate() {
                            assert!(domains[piece].contains(placement));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn search_returns_placements_in_original_piece_order() {
        let pieces = vec![
            Piece::from_grid(&[&[true, false], &[true, true]]),
            Piece::from_grid(&[&[true, true]]),
        ];
        for modulus in 2..=5 {
            let board = Board::from_grid(&[&[1, 1, 0], &[0, 1, 0], &[0, 1, 1]], modulus);
            let game = Game::new(board, pieces.clone());
            let data = AdaptiveSearch::precompute(&game);
            let (solution, _) = data.solve(Duration::from_secs(2), 1);
            let solution = solution.expect("the small puzzle has a solution");
            assert_eq!(solution.len(), pieces.len());
            let mut replay = board;
            for (piece, (row, column)) in pieces.iter().zip(solution) {
                replay.apply_piece(piece.placed_at(row, column));
            }
            assert!(replay.is_solved());
        }
    }
}
