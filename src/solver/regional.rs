//! Search by conditioning joint distributions on small, disjoint board regions.
//!
//! A region's factor sums whole-placement effects modulo M. Messages from these
//! factors guide decimation of the most confident piece. Restarts vary both the
//! region partition and the choices, so a mistaken decimation is reversible.
//! This is a heuristic search; only replayed, exact solutions are returned.

use std::ops::{Add, Mul};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rand::{RngExt, SeedableRng};

use crate::core::STRIDE;
use crate::game::Game;

use super::Solution;

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    const ONE: Self = Self { re: 1.0, im: 0.0 };

    fn conjugate(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }

    fn scale(self, scale: f64) -> Self {
        Self {
            re: self.re * scale,
            im: self.im * scale,
        }
    }
}

impl Add for Complex {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }
}

impl Mul for Complex {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }
}

struct Fourier {
    modulus: usize,
    roots: [[Complex; 5]; 5],
}

impl Fourier {
    fn new(modulus: usize) -> Self {
        let mut roots = [[Complex::default(); 5]; 5];
        for (i, row) in roots.iter_mut().enumerate().take(modulus) {
            for (j, value) in row.iter_mut().enumerate().take(modulus) {
                let angle = -std::f64::consts::TAU * (i * j) as f64 / modulus as f64;
                *value = Complex {
                    re: angle.cos(),
                    im: angle.sin(),
                };
            }
        }
        Self { modulus, roots }
    }

    /// Tensor-product transform on (Z/MZ)^k, not a length-M^k cyclic FFT.
    fn transform(&self, values: &mut [Complex], inverse: bool) {
        let mut stride = 1;
        while stride < values.len() {
            for block in (0..values.len()).step_by(stride * self.modulus) {
                for offset in 0..stride {
                    let mut input = [Complex::default(); 5];
                    for (digit, value) in input.iter_mut().enumerate().take(self.modulus) {
                        *value = values[block + offset + digit * stride];
                    }
                    for digit in 0..self.modulus {
                        let mut sum = Complex::default();
                        for (other, &value) in input.iter().enumerate().take(self.modulus) {
                            let root = self.roots[digit][other];
                            sum = sum + value * if inverse { root.conjugate() } else { root };
                        }
                        values[block + offset + digit * stride] = sum;
                    }
                }
            }
            stride *= self.modulus;
        }
        if inverse {
            let scale = 1.0 / values.len() as f64;
            for value in values {
                *value = value.scale(scale);
            }
        }
    }
}

struct OptionData {
    position: (usize, usize),
    cells: Vec<usize>,
}

struct Region {
    states: usize,
    effects: Vec<Vec<usize>>,
    residual: Vec<usize>,
}

struct Partition {
    regions: Vec<Region>,
}

pub(super) struct RegionalSearch {
    options: Vec<Vec<OptionData>>,
    target: Vec<u8>,
    modulus: usize,
    fourier: Fourier,
    partitions: Vec<Vec<Partition>>,
}

type Messages = Vec<Vec<Vec<f64>>>;

struct SharedSearch {
    deadline: Instant,
    stop: AtomicBool,
    solution: Mutex<Option<Solution>>,
    nodes: AtomicU64,
    next_attempt: AtomicUsize,
}

fn partition_cells(
    height: usize,
    width: usize,
    rows: usize,
    columns: usize,
    shift: usize,
) -> Vec<Vec<usize>> {
    let mut regions = Vec::new();
    let first_row = -((shift % rows) as isize);
    let first_column = -((shift / rows) as isize);
    for top in (first_row..height as isize).step_by(rows) {
        for left in (first_column..width as isize).step_by(columns) {
            let mut cells = Vec::new();
            for row in top.max(0)..(top + rows as isize).min(height as isize) {
                for column in left.max(0)..(left + columns as isize).min(width as isize) {
                    cells.push(row as usize * width + column as usize);
                }
            }
            if !cells.is_empty() {
                regions.push(cells);
            }
        }
    }
    regions
}

impl RegionalSearch {
    pub(super) fn precompute(game: &Game) -> Self {
        let board = game.board();
        let height = usize::from(board.height());
        let width = usize::from(board.width());
        let modulus = usize::from(board.m());
        let target = (0..height * width)
            .map(|cell| board.get(cell / width, cell % width))
            .collect::<Vec<_>>();
        let placements = game
            .pieces()
            .iter()
            .map(|piece| piece.placements(board.height(), board.width()))
            .collect::<Vec<_>>();
        let options = placements
            .iter()
            .map(|piece| {
                piece
                    .iter()
                    .map(|&(row, column, mask)| OptionData {
                        position: (row, column),
                        cells: (0..height * width)
                            .filter(|&cell| {
                                mask.get_bit((cell / width * STRIDE + cell % width) as u32)
                            })
                            .collect(),
                    })
                    .collect()
            })
            .collect();
        // Choose region sizes by the cost of their joint distributions, using
        // the same state budget for every board and modulus.
        const MAX_REGION_STATES: usize = 1 << 10;
        let region_shapes = if modulus.pow(6) <= MAX_REGION_STATES {
            [(2, 2), (2, 3), (3, 2)]
        } else {
            [(2, 2), (1, 4), (4, 1)]
        };
        let partitions = region_shapes
            .into_iter()
            .map(|(rows, columns)| {
                (0..rows * columns)
                    .map(|shift| {
                        let regions = partition_cells(height, width, rows, columns, shift)
                            .into_iter()
                            .map(|cells| {
                                let states = modulus.pow(cells.len() as u32);
                                let effects = placements
                                    .iter()
                                    .map(|piece| {
                                        piece
                                            .iter()
                                            .map(|&(_, _, mask)| {
                                                let mut state = 0;
                                                let mut power = 1;
                                                for &cell in &cells {
                                                    if mask.get_bit(
                                                        (cell / width * STRIDE + cell % width)
                                                            as u32,
                                                    ) {
                                                        state += power;
                                                    }
                                                    power *= modulus;
                                                }
                                                state
                                            })
                                            .collect()
                                    })
                                    .collect();
                                let residual = (0..states)
                                    .map(|mut state| {
                                        let mut result = 0;
                                        let mut power = 1;
                                        for &cell in &cells {
                                            result += (usize::from(target[cell]) + modulus
                                                - state % modulus)
                                                % modulus
                                                * power;
                                            power *= modulus;
                                            state /= modulus;
                                        }
                                        result
                                    })
                                    .collect();
                                Region {
                                    states,
                                    effects,
                                    residual,
                                }
                            })
                            .collect();
                        Partition { regions }
                    })
                    .collect()
            })
            .collect();
        Self {
            options,
            target,
            modulus,
            fourier: Fourier::new(modulus),
            partitions,
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
            next_attempt: AtomicUsize::new(0),
        };
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let shared = &shared;
                scope.spawn(move || self.worker(shared));
            }
        });
        (
            shared.solution.into_inner().unwrap(),
            shared.nodes.into_inner(),
        )
    }

    fn scores(
        &self,
        partition: &Partition,
        messages: &Messages,
        prior: &[Vec<f64>],
    ) -> Vec<Vec<f64>> {
        let mut scores = prior.to_vec();
        for (piece, options) in scores.iter_mut().enumerate() {
            for (region, message) in partition.regions.iter().zip(messages) {
                for (placement, score) in options.iter_mut().enumerate() {
                    *score += message[piece][region.effects[piece][placement]];
                }
            }
        }
        scores
    }

    fn update_messages(
        &self,
        partition: &Partition,
        messages: &mut Messages,
        scores: &[Vec<f64>],
        fixed: &[Option<usize>],
        damping: f64,
    ) {
        let pieces = self.options.len();
        for (region, messages) in partition.regions.iter().zip(messages) {
            let states = region.states;
            let mut spectra = vec![vec![Complex::default(); states]; pieces];
            let mut prefix = vec![vec![Complex::ONE; states]; pieces + 1];
            for piece in 0..pieces {
                if let Some(placement) = fixed[piece] {
                    spectra[piece][region.effects[piece][placement]] = Complex::ONE;
                } else {
                    let cavity = scores[piece]
                        .iter()
                        .enumerate()
                        .map(|(placement, &score)| {
                            score - messages[piece][region.effects[piece][placement]]
                        })
                        .collect::<Vec<_>>();
                    let maximum = cavity.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let mut sum = 0.0;
                    for (placement, value) in cavity.into_iter().enumerate() {
                        let weight = (value - maximum).max(-80.0).exp();
                        spectra[piece][region.effects[piece][placement]].re += weight;
                        sum += weight;
                    }
                    for value in &mut spectra[piece] {
                        *value = value.scale(1.0 / sum);
                    }
                }
                self.fourier.transform(&mut spectra[piece], false);
                let (before, after) = prefix.split_at_mut(piece + 1);
                for ((next, &previous), &value) in
                    after[0].iter_mut().zip(&before[piece]).zip(&spectra[piece])
                {
                    *next = previous * value;
                }
            }
            // Prefix/suffix products avoid division by a possibly zero Fourier
            // coefficient when removing a piece from the joint distribution.
            let mut suffix = vec![Complex::ONE; states];
            let mut without = vec![Complex::default(); states];
            for piece in (0..pieces).rev() {
                for (state, suffix) in suffix.iter_mut().enumerate() {
                    without[state] = prefix[piece][state] * *suffix;
                    *suffix = *suffix * spectra[piece][state];
                }
                if fixed[piece].is_some() {
                    continue;
                }
                self.fourier.transform(&mut without, true);
                let next = region
                    .residual
                    .iter()
                    .map(|&state| without[state].re.max(1e-16).ln())
                    .collect::<Vec<_>>();
                let maximum = next.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                for (message, next) in messages[piece].iter_mut().zip(next) {
                    *message = (1.0 - damping) * *message + damping * (next - maximum).max(-30.0);
                }
            }
        }
    }

    fn verified_solution(&self, selection: &[usize]) -> Option<Solution> {
        let mut hits = vec![0usize; self.target.len()];
        for (piece, &placement) in selection.iter().enumerate() {
            for &cell in &self.options[piece][placement].cells {
                hits[cell] += 1;
            }
        }
        hits.iter()
            .zip(&self.target)
            .all(|(&hits, &target)| hits % self.modulus == usize::from(target))
            .then(|| {
                selection
                    .iter()
                    .enumerate()
                    .map(|(piece, &placement)| self.options[piece][placement].position)
                    .collect()
            })
    }

    fn fixed_choices_are_feasible(&self, fixed: &[Option<usize>]) -> bool {
        let mut residual = self.target.clone();
        let mut area = 0;
        for (piece, &placement) in fixed.iter().enumerate() {
            if let Some(placement) = placement {
                for &cell in &self.options[piece][placement].cells {
                    residual[cell] = (residual[cell] + self.modulus as u8 - 1) % self.modulus as u8;
                }
            } else {
                area += self.options[piece][0].cells.len();
            }
        }
        residual
            .iter()
            .map(|&value| usize::from(value))
            .sum::<usize>()
            <= area
    }

    fn worker(&self, shared: &SharedSearch) {
        let partition_count = self.partitions.iter().map(Vec::len).sum::<usize>();
        let configuration_count = partition_count * 5;
        let mut nodes = 0;
        while Instant::now() < shared.deadline && !shared.stop.load(Ordering::Relaxed) {
            // Cover every partition/damping pair before revisiting one. Fast
            // attempts take more work from this counter, so a slow region
            // partition cannot strand a search family on one worker.
            let attempt = shared.next_attempt.fetch_add(1, Ordering::Relaxed);
            // 37 is coprime to both configuration counts (80 and 60), spreading
            // early workers over all three region shapes.
            let configuration = (attempt % configuration_count) * 37 % configuration_count;
            let partition = self
                .partitions
                .iter()
                .flatten()
                .nth(configuration % partition_count)
                .unwrap();
            let damping = 0.15 + 0.1 * (configuration / partition_count) as f64;
            let explore = attempt >= configuration_count;
            let mut rng = rand::rngs::SmallRng::seed_from_u64(98_765 + attempt as u64 * 701);
            let mut messages = partition
                .regions
                .iter()
                .map(|region| vec![vec![0.0; region.states]; self.options.len()])
                .collect::<Messages>();
            let prior = self
                .options
                .iter()
                .map(|options| {
                    options
                        .iter()
                        .map(|_| (rng.random::<f64>() - 0.5) * if explore { 0.5 } else { 0.01 })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut fixed = vec![None; self.options.len()];
            for iteration in 0..600usize {
                if Instant::now() >= shared.deadline || shared.stop.load(Ordering::Relaxed) {
                    break;
                }
                nodes += 1;
                let scores = self.scores(partition, &messages, &prior);
                let mut selection = Vec::with_capacity(self.options.len());
                let mut most_confident = None;
                let mut confidence = 0.0;
                for (piece, options) in scores.iter().enumerate() {
                    let mut best = 0;
                    for placement in 1..options.len() {
                        if options[placement] > options[best] {
                            best = placement;
                        }
                    }
                    selection.push(fixed[piece].unwrap_or(best));
                    if fixed[piece].is_none() {
                        let sum = options
                            .iter()
                            .map(|&score| (score - options[best]).max(-80.0).exp())
                            .sum::<f64>();
                        if 1.0 / sum > confidence {
                            confidence = 1.0 / sum;
                            most_confident = Some((piece, best));
                        }
                    }
                }
                if let Some(solution) = self.verified_solution(&selection) {
                    let mut saved = shared.solution.lock().unwrap();
                    if saved.is_none() {
                        *saved = Some(solution);
                    }
                    shared.stop.store(true, Ordering::Relaxed);
                    break;
                }
                if iteration >= 30 && iteration.is_multiple_of(15) {
                    let Some((piece, mut placement)) = most_confident else {
                        break;
                    };
                    if explore && !attempt.is_multiple_of(3) {
                        let maximum = scores[piece][placement];
                        let weights = scores[piece]
                            .iter()
                            .map(|&score| ((score - maximum) * (1 + attempt % 4) as f64).exp())
                            .collect::<Vec<_>>();
                        let mut draw = rng.random::<f64>() * weights.iter().sum::<f64>();
                        for (candidate, weight) in weights.into_iter().enumerate() {
                            draw -= weight;
                            if draw <= 0.0 {
                                placement = candidate;
                                break;
                            }
                        }
                    }
                    fixed[piece] = Some(placement);
                    if !self.fixed_choices_are_feasible(&fixed) {
                        break;
                    }
                }
                self.update_messages(partition, &mut messages, &scores, &fixed, damping);
            }
        }
        shared.nodes.fetch_add(nodes, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{board::Board, piece::Piece};

    #[test]
    fn region_partitions_cover_every_cell_once() {
        for height in 3..=14 {
            for width in 3..=14 {
                for (rows, columns) in [(2, 2), (2, 3), (3, 2), (1, 4), (4, 1)] {
                    for shift in 0..rows * columns {
                        let mut counts = vec![0; height * width];
                        for region in partition_cells(height, width, rows, columns, shift) {
                            for cell in region {
                                counts[cell] += 1;
                            }
                        }
                        assert!(counts.into_iter().all(|count| count == 1));
                    }
                }
            }
        }
    }

    #[test]
    fn fourier_product_matches_direct_modular_convolution() {
        for modulus in 2..=5 {
            let plan = Fourier::new(modulus);
            let states = modulus * modulus;
            let left = (0..states)
                .map(|i| (i * 7 % 11) as f64 / 11.0)
                .collect::<Vec<_>>();
            let right = (0..states)
                .map(|i| (i * 3 % 13) as f64 / 13.0)
                .collect::<Vec<_>>();
            let mut expected = vec![0.0; states];
            for (a, &pa) in left.iter().enumerate() {
                for (b, &pb) in right.iter().enumerate() {
                    let sum = (a % modulus + b % modulus) % modulus
                        + ((a / modulus + b / modulus) % modulus) * modulus;
                    expected[sum] += pa * pb;
                }
            }
            let mut transformed_left = left
                .into_iter()
                .map(|re| Complex { re, im: 0.0 })
                .collect::<Vec<_>>();
            let mut transformed_right = right
                .into_iter()
                .map(|re| Complex { re, im: 0.0 })
                .collect::<Vec<_>>();
            plan.transform(&mut transformed_left, false);
            plan.transform(&mut transformed_right, false);
            for (left, right) in transformed_left.iter_mut().zip(transformed_right) {
                *left = *left * right;
            }
            plan.transform(&mut transformed_left, true);
            for (actual, expected) in transformed_left.into_iter().zip(expected) {
                assert!((actual.re - expected).abs() < 1e-10);
                assert!(actual.im.abs() < 1e-10);
            }
        }
    }

    #[test]
    fn returns_only_replayed_legal_placements() {
        let piece = Piece::from_grid(&[&[true, false], &[true, true]]);
        for modulus in 2..=5 {
            let board = Board::from_grid(&[&[0, 0, 0], &[0, 1, 0], &[0, 1, 1]], modulus);
            let game = Game::new(board, vec![piece]);
            let search = RegionalSearch::precompute(&game);
            let (solution, _) = search.solve(Duration::from_secs(2), 1);
            let solution =
                solution.expect("one piece must be identified by the regional constraints");
            assert_eq!(solution, vec![(1, 1)]);
            let mut replay = board;
            replay.apply_piece(piece.placed_at(solution[0].0, solution[0].1));
            assert!(replay.is_solved());
            assert!(search.verified_solution(&[0]).is_none());
        }
    }
}
