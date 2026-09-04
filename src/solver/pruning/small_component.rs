//! Lower-bounds solved cells that must be disturbed around small nonzero components.
//!
//! A connected piece that overlaps a component and extends outside it must cross its
//! cardinal boundary. Every distinct boundary cell is currently zero, so touching it
//! consumes at least one of the extra hit cycles allowed by the global deficit bound.
//! Suffix tables find the exact minimum for singleton, domino, and straight-three
//! components, plus whether a four-cell L requires any boundary cell. Runtime bit
//! shifts find the components, and only disjoint boundaries are combined. The tables
//! permit off-board translations, which only relaxes the bound.

use std::mem::MaybeUninit;

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::piece::Piece;

const MODULUS: usize = 3;
const MAX_COMPONENT_CELLS: usize = 4;
const MAX_EXACT_COMPONENT_CELLS: usize = 3;
const MAX_BOUNDARY_CELLS: usize = 9;
const GRID_CAPACITY: usize = STRIDE * STRIDE;
const UNREACHABLE: u8 = u8::MAX;
const MAX_ZERO_HIT_BUDGET: u32 = 4;

#[derive(Clone, Copy)]
struct ComponentTemplate {
    cells: [(i8, i8); MAX_COMPONENT_CELLS],
    cell_count: u8,
}

const TEMPLATES: [ComponentTemplate; 13] = [
    ComponentTemplate {
        cells: [(0, 0), (0, 0), (0, 0), (0, 0)],
        cell_count: 1,
    },
    ComponentTemplate {
        cells: [(0, 0), (0, 1), (0, 0), (0, 0)],
        cell_count: 2,
    },
    ComponentTemplate {
        cells: [(0, 0), (1, 0), (0, 0), (0, 0)],
        cell_count: 2,
    },
    ComponentTemplate {
        cells: [(0, 0), (0, 1), (0, 2), (0, 0)],
        cell_count: 3,
    },
    ComponentTemplate {
        cells: [(0, 0), (1, 0), (2, 0), (0, 0)],
        cell_count: 3,
    },
    ComponentTemplate {
        cells: [(0, 0), (1, 0), (2, 0), (2, 1)],
        cell_count: 4,
    },
    ComponentTemplate {
        cells: [(0, 1), (1, 1), (2, 0), (2, 1)],
        cell_count: 4,
    },
    ComponentTemplate {
        cells: [(0, 0), (0, 1), (1, 0), (2, 0)],
        cell_count: 4,
    },
    ComponentTemplate {
        cells: [(0, 0), (0, 1), (1, 1), (2, 1)],
        cell_count: 4,
    },
    ComponentTemplate {
        cells: [(0, 0), (0, 1), (0, 2), (1, 0)],
        cell_count: 4,
    },
    ComponentTemplate {
        cells: [(0, 0), (0, 1), (0, 2), (1, 2)],
        cell_count: 4,
    },
    ComponentTemplate {
        cells: [(0, 0), (1, 0), (1, 1), (1, 2)],
        cell_count: 4,
    },
    ComponentTemplate {
        cells: [(0, 2), (1, 0), (1, 1), (1, 2)],
        cell_count: 4,
    },
];

struct ComponentTable {
    anchor_index: usize,
    template: ComponentTemplate,
    valid_anchors: Bitboard,
    boundary_masks: Vec<Bitboard>,
    configuration_count: usize,
    minimum_boundary_cells: Vec<u8>,
}

pub(crate) struct SmallComponentBound {
    tables: Vec<ComponentTable>,
    valid_mask: Bitboard,
}

#[derive(Clone, Copy)]
struct BoundaryRequirement {
    mask: Bitboard,
    minimum: u8,
}

impl SmallComponentBound {
    pub(crate) fn precompute(
        pieces: &[Piece],
        piece_order: &[usize],
        height: u8,
        width: u8,
        modulus: u8,
    ) -> Self {
        let mut valid_mask = Bitboard::ZERO;
        for row in 0..usize::from(height) {
            for column in 0..usize::from(width) {
                valid_mask.set_bit((row * STRIDE + column) as u32);
            }
        }
        if usize::from(modulus) != MODULUS
            || !pieces.iter().all(|piece| shape_is_connected(piece.shape()))
        {
            return Self {
                tables: Vec::new(),
                valid_mask,
            };
        }

        let tables = TEMPLATES
            .into_iter()
            .enumerate()
            .map(|(anchor_index, template)| {
                ComponentTable::precompute(
                    anchor_index,
                    template,
                    pieces,
                    piece_order,
                    height,
                    width,
                )
            })
            .collect();
        Self { tables, valid_mask }
    }

    #[inline]
    pub(crate) fn allows<const BOARD_MODULUS: usize>(
        &self,
        board: &Board,
        piece_index: usize,
        zero_hit_budget: u32,
    ) -> bool {
        if BOARD_MODULUS != MODULUS
            || self.tables.is_empty()
            || zero_hit_budget > MAX_ZERO_HIT_BUDGET
        {
            return true;
        }

        let nonzero = self.valid_mask & !board.plane(0);
        let deficit_one = board.plane(1);
        let component_anchors = component_anchors(nonzero);
        let mut requirements =
            [const { MaybeUninit::<BoundaryRequirement>::uninit() }; GRID_CAPACITY];
        let mut requirement_count = 0;

        for table in &self.tables {
            let mut anchors = table.matching_anchors(&component_anchors);
            while !anchors.is_zero() {
                let anchor = anchors.lowest_set_bit();
                anchors.clear_bit(anchor);
                let configuration = table.encode(deficit_one, anchor);
                let minimum = table.minimum_boundary_cells
                    [piece_index * table.configuration_count + configuration];
                if minimum == UNREACHABLE {
                    return false;
                }
                if minimum != 0 {
                    requirements[requirement_count].write(BoundaryRequirement {
                        mask: table.boundary_masks[anchor as usize],
                        minimum,
                    });
                    requirement_count += 1;
                }
            }
        }

        // Every element in this prefix was initialized immediately before the count advanced.
        let requirements = unsafe {
            std::slice::from_raw_parts(
                requirements.as_ptr().cast::<BoundaryRequirement>(),
                requirement_count,
            )
        };
        let mut selected_boundaries = Bitboard::ZERO;
        let mut required_cycles = 0;
        for minimum in (1..=MAX_BOUNDARY_CELLS as u8).rev() {
            for requirement in requirements {
                if requirement.minimum == minimum
                    && (requirement.mask & selected_boundaries).is_zero()
                {
                    selected_boundaries |= requirement.mask;
                    required_cycles += u32::from(minimum);
                    if required_cycles > zero_hit_budget {
                        return false;
                    }
                }
            }
        }
        true
    }
}

impl ComponentTable {
    fn precompute(
        anchor_index: usize,
        template: ComponentTemplate,
        pieces: &[Piece],
        piece_order: &[usize],
        height: u8,
        width: u8,
    ) -> Self {
        let cells = template.cells();
        let boundary = component_boundary(cells);
        debug_assert!(boundary.len() <= MAX_BOUNDARY_CELLS);
        let configuration_count = MODULUS.pow(template.cell_count as u32);

        let transitions = piece_order
            .iter()
            .map(|&piece_index| piece_transitions(&pieces[piece_index], cells, &boundary))
            .collect::<Vec<_>>();
        let minimum_boundary_cells =
            if usize::from(template.cell_count) <= MAX_EXACT_COMPONENT_CELLS {
                exact_minimums(
                    &transitions,
                    template.cell_count as usize,
                    configuration_count,
                    boundary.len(),
                )
            } else {
                zero_or_one_minimums(
                    &transitions,
                    template.cell_count as usize,
                    configuration_count,
                )
            };

        let (valid_anchors, boundary_masks) = board_masks(cells, &boundary, height, width);
        Self {
            anchor_index,
            template,
            valid_anchors,
            boundary_masks,
            configuration_count,
            minimum_boundary_cells,
        }
    }

    #[inline(always)]
    fn matching_anchors(&self, component_anchors: &[Bitboard; TEMPLATES.len()]) -> Bitboard {
        component_anchors[self.anchor_index] & self.valid_anchors
    }

    #[inline(always)]
    fn encode(&self, deficit_one: Bitboard, anchor: u32) -> usize {
        let mut configuration = 0;
        let mut multiplier = 1;
        for &(row, column) in self.template.cells() {
            let index = (anchor as i32 + i32::from(row) * STRIDE as i32 + i32::from(column)) as u32;
            let deficit = if deficit_one.get_bit(index) { 1 } else { 2 };
            configuration += deficit * multiplier;
            multiplier *= MODULUS;
        }
        configuration
    }
}

fn exact_minimums(
    transitions: &[Vec<(u8, u16)>],
    cell_count: usize,
    configuration_count: usize,
    boundary_cell_count: usize,
) -> Vec<u8> {
    let boundary_configuration_count = 1 << boundary_cell_count;
    let state_count = configuration_count * boundary_configuration_count;
    let mut minimums = vec![UNREACHABLE; (transitions.len() + 1) * configuration_count];
    let mut next = vec![false; state_count];
    next[0] = true;
    record_minimums(
        &next,
        transitions.len(),
        configuration_count,
        boundary_configuration_count,
        &mut minimums,
    );

    for piece_index in (0..transitions.len()).rev() {
        let mut current = next.clone();
        for &(effect, touched_boundary) in &transitions[piece_index] {
            for configuration in 0..configuration_count {
                let next_configuration = add_effect(configuration, effect, cell_count);
                let state_base = configuration * boundary_configuration_count;
                let next_state_base = next_configuration * boundary_configuration_count;
                for boundary_mask in 0..boundary_configuration_count {
                    if next[state_base + boundary_mask] {
                        current[next_state_base + (boundary_mask | touched_boundary as usize)] =
                            true;
                    }
                }
            }
        }
        record_minimums(
            &current,
            piece_index,
            configuration_count,
            boundary_configuration_count,
            &mut minimums,
        );
        next = current;
    }
    minimums
}

fn zero_or_one_minimums(
    transitions: &[Vec<(u8, u16)>],
    cell_count: usize,
    configuration_count: usize,
) -> Vec<u8> {
    let mut minimums = vec![UNREACHABLE; (transitions.len() + 1) * configuration_count];
    let mut reachable = vec![false; configuration_count];
    let mut reachable_without_boundary = vec![false; configuration_count];
    reachable[0] = true;
    reachable_without_boundary[0] = true;
    record_zero_or_one_minimums(
        &reachable,
        &reachable_without_boundary,
        transitions.len(),
        configuration_count,
        &mut minimums,
    );

    for piece_index in (0..transitions.len()).rev() {
        let mut current = reachable.clone();
        let mut current_without_boundary = reachable_without_boundary.clone();
        for &(effect, touched_boundary) in &transitions[piece_index] {
            for configuration in 0..configuration_count {
                let next_configuration = add_effect(configuration, effect, cell_count);
                if reachable[configuration] {
                    current[next_configuration] = true;
                }
                if touched_boundary == 0 && reachable_without_boundary[configuration] {
                    current_without_boundary[next_configuration] = true;
                }
            }
        }
        record_zero_or_one_minimums(
            &current,
            &current_without_boundary,
            piece_index,
            configuration_count,
            &mut minimums,
        );
        reachable = current;
        reachable_without_boundary = current_without_boundary;
    }
    minimums
}

fn record_zero_or_one_minimums(
    reachable: &[bool],
    reachable_without_boundary: &[bool],
    piece_index: usize,
    configuration_count: usize,
    minimums: &mut [u8],
) {
    for configuration in 0..configuration_count {
        minimums[piece_index * configuration_count + configuration] =
            if reachable_without_boundary[configuration] {
                0
            } else if reachable[configuration] {
                1
            } else {
                UNREACHABLE
            };
    }
}

#[inline(always)]
fn component_anchors(nonzero: Bitboard) -> [Bitboard; TEMPLATES.len()] {
    let left = nonzero.shl_1();
    let right = nonzero.shr_1();
    let above = nonzero.shl_stride();
    let below = nonzero.shr_stride();
    let horizontal = left | right;
    let vertical = above | below;
    let any_neighbor = horizontal | vertical;
    let at_least_two = (left & right) | (above & below) | (horizontal & vertical);
    let degree_one = nonzero & any_neighbor & !at_least_two;
    let mut anchors = [Bitboard::ZERO; TEMPLATES.len()];
    anchors[0] = nonzero & !any_neighbor;
    anchors[1] = degree_one & degree_one.shr_1();
    anchors[2] = degree_one & degree_one.shr_stride();

    let at_least_three = (left & right & vertical) | (above & below & horizontal);
    let degree_two = nonzero & at_least_two & !at_least_three;
    let d1_1 = degree_one.shr_1();
    let d1_2 = d1_1.shr_1();
    let d1_15 = degree_one.shr_stride();
    let d1_16 = d1_15.shr_1();
    let d1_30 = d1_15.shr_stride();
    let d2_1 = degree_two.shr_1();
    let d2_15 = degree_two.shr_stride();
    let d2_16 = d2_15.shr_1();
    anchors[3] = degree_one & d2_1 & d1_2;
    anchors[4] = degree_one & d2_15 & d1_30;
    let d1_17 = d1_16.shr_1();
    let d1_31 = d1_30.shr_1();
    let d2_2 = d2_1.shr_1();
    let d2_17 = d2_16.shr_1();
    let d2_30 = d2_15.shr_stride();
    let d2_31 = d2_30.shr_1();
    anchors[5] = degree_one & d2_15 & d2_30 & d1_31;
    anchors[6] = d1_1 & d2_16 & d1_30 & d2_31;
    anchors[7] = degree_two & d1_1 & d2_15 & d1_30;
    anchors[8] = degree_one & d2_1 & d2_16 & d1_31;
    anchors[9] = degree_two & d2_1 & d1_2 & d1_15;
    anchors[10] = degree_one & d2_1 & d2_2 & d1_17;
    anchors[11] = degree_one & d2_15 & d2_16 & d1_17;
    anchors[12] = d1_2 & d1_15 & d2_16 & d2_17;
    anchors
}

impl ComponentTemplate {
    fn cells(&self) -> &[(i8, i8)] {
        &self.cells[..self.cell_count as usize]
    }
}

fn component_boundary(cells: &[(i8, i8)]) -> Vec<(i8, i8)> {
    let mut boundary = Vec::new();
    for &(row, column) in cells {
        for neighbor in [
            (row - 1, column),
            (row + 1, column),
            (row, column - 1),
            (row, column + 1),
        ] {
            if !cells.contains(&neighbor) && !boundary.contains(&neighbor) {
                boundary.push(neighbor);
            }
        }
    }
    boundary.sort_unstable();
    boundary
}

fn piece_transitions(piece: &Piece, cells: &[(i8, i8)], boundary: &[(i8, i8)]) -> Vec<(u8, u16)> {
    let maximum_row = cells.iter().map(|&(row, _)| row).max().unwrap();
    let maximum_column = cells.iter().map(|&(_, column)| column).max().unwrap();
    let mut transitions = Vec::new();

    for top in -(piece.height() as i8)..=maximum_row {
        for left in -(piece.width() as i8)..=maximum_column {
            let mut effect = 0;
            let mut touched_boundary = 0;
            for piece_row in 0..piece.height() as usize {
                for piece_column in 0..piece.width() as usize {
                    if !piece
                        .shape()
                        .get_bit((piece_row * STRIDE + piece_column) as u32)
                    {
                        continue;
                    }
                    let position = (top + piece_row as i8, left + piece_column as i8);
                    if let Some(cell_index) = cells.iter().position(|&cell| cell == position) {
                        effect |= 1 << cell_index;
                    }
                    if let Some(boundary_index) = boundary.iter().position(|&cell| cell == position)
                    {
                        touched_boundary |= 1 << boundary_index;
                    }
                }
            }
            if effect != 0 {
                transitions.push((effect, touched_boundary));
            }
        }
    }
    transitions.sort_unstable();
    transitions.dedup();
    let all_transitions = transitions.clone();
    transitions.retain(|&(effect, boundary_mask)| {
        !all_transitions
            .iter()
            .any(|&(other_effect, other_boundary_mask)| {
                effect == other_effect
                    && boundary_mask != other_boundary_mask
                    && other_boundary_mask & boundary_mask == other_boundary_mask
            })
    });
    transitions
}

fn record_minimums(
    reachable: &[bool],
    piece_index: usize,
    configuration_count: usize,
    boundary_configuration_count: usize,
    minimums: &mut [u8],
) {
    for configuration in 0..configuration_count {
        let minimum = (0..boundary_configuration_count)
            .filter(|&boundary| reachable[configuration * boundary_configuration_count + boundary])
            .map(|boundary| boundary.count_ones() as u8)
            .min()
            .unwrap_or(UNREACHABLE);
        minimums[piece_index * configuration_count + configuration] = minimum;
    }
}

fn add_effect(mut configuration: usize, effect: u8, cell_count: usize) -> usize {
    let mut result = configuration;
    let mut multiplier = 1;
    for cell_index in 0..cell_count {
        let digit = configuration % MODULUS;
        configuration /= MODULUS;
        if effect & (1 << cell_index) != 0 {
            if digit + 1 == MODULUS {
                result -= (MODULUS - 1) * multiplier;
            } else {
                result += multiplier;
            }
        }
        multiplier *= MODULUS;
    }
    result
}

fn board_masks(
    cells: &[(i8, i8)],
    boundary: &[(i8, i8)],
    height: u8,
    width: u8,
) -> (Bitboard, Vec<Bitboard>) {
    let mut valid_anchors = Bitboard::ZERO;
    let mut boundary_masks = vec![Bitboard::ZERO; GRID_CAPACITY];
    for anchor_row in 0..usize::from(height) {
        for anchor_column in 0..usize::from(width) {
            if !cells.iter().all(|&(row, column)| {
                coordinate_is_valid(
                    anchor_row as isize + row as isize,
                    anchor_column as isize + column as isize,
                    height,
                    width,
                )
            }) {
                continue;
            }
            let anchor = (anchor_row * STRIDE + anchor_column) as u32;
            valid_anchors.set_bit(anchor);
            for &(row, column) in boundary {
                let board_row = anchor_row as isize + row as isize;
                let board_column = anchor_column as isize + column as isize;
                if coordinate_is_valid(board_row, board_column, height, width) {
                    boundary_masks[anchor as usize]
                        .set_bit((board_row as usize * STRIDE + board_column as usize) as u32);
                }
            }
        }
    }
    (valid_anchors, boundary_masks)
}

fn coordinate_is_valid(row: isize, column: isize, height: u8, width: u8) -> bool {
    row >= 0 && row < isize::from(height) && column >= 0 && column < isize::from(width)
}

fn shape_is_connected(shape: Bitboard) -> bool {
    let mut reached = Bitboard::from_bit(shape.lowest_set_bit());
    loop {
        let expanded = (reached
            | reached.shl_1()
            | reached.shr_1()
            | reached.shl_stride()
            | reached.shr_stride())
            & shape;
        if expanded == reached {
            return reached == shape;
        }
        reached = expanded;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(pieces: &[Piece], board: &Board) -> SmallComponentBound {
        SmallComponentBound::precompute(
            pieces,
            &(0..pieces.len()).collect::<Vec<_>>(),
            board.height(),
            board.width(),
            board.m(),
        )
    }

    #[test]
    fn domino_piece_solves_matching_domino_without_boundary_cells() {
        let board = Board::from_grid(&[&[0, 0, 0], &[1, 1, 0], &[0, 0, 0]], 3);
        let pieces = [Piece::from_grid(&[&[true, true]])];
        assert!(bound(&pieces, &board).allows::<3>(&board, 0, 0));
    }

    #[test]
    fn four_cell_l_charges_one_when_no_piece_fits_inside_it() {
        let board = Board::from_grid(&[&[0, 0, 0, 0], &[0, 0, 1, 0], &[1, 1, 1, 0]], 3);
        let pieces = [Piece::from_grid(&[
            &[false, true, true],
            &[true, true, true],
        ])];

        assert!(!bound(&pieces, &board).allows::<3>(&board, 0, 0));
        assert!(bound(&pieces, &board).allows::<3>(&board, 0, 1));
    }

    #[test]
    fn disjoint_components_add_their_boundary_requirements() {
        let board = Board::from_grid(
            &[
                &[0, 0, 0, 0, 0, 0, 0, 0],
                &[0, 0, 1, 0, 0, 0, 0, 1],
                &[1, 1, 1, 0, 0, 1, 1, 1],
            ],
            3,
        );
        let piece = Piece::from_grid(&[&[false, true, true], &[true, true, true]]);
        let pieces = [piece, piece];
        assert!(!bound(&pieces, &board).allows::<3>(&board, 0, 1));
        assert!(bound(&pieces, &board).allows::<3>(&board, 0, 2));
    }

    #[test]
    fn bit_shift_detection_matches_component_boundaries() {
        const HEIGHT: u8 = 3;
        const WIDTH: u8 = 3;

        for pattern in 0u16..(1 << (HEIGHT * WIDTH)) {
            let mut nonzero = Bitboard::ZERO;
            for row in 0..usize::from(HEIGHT) {
                for column in 0..usize::from(WIDTH) {
                    if pattern & (1 << (row * usize::from(WIDTH) + column)) != 0 {
                        nonzero.set_bit((row * STRIDE + column) as u32);
                    }
                }
            }
            let detected = component_anchors(nonzero);

            for (template_index, template) in TEMPLATES.iter().enumerate() {
                let cells = template.cells();
                let boundary = component_boundary(cells);
                let (valid_anchors, _) = board_masks(cells, &boundary, HEIGHT, WIDTH);
                let mut expected = Bitboard::ZERO;
                for anchor_row in 0..usize::from(HEIGHT) {
                    for anchor_column in 0..usize::from(WIDTH) {
                        let anchor = (anchor_row * STRIDE + anchor_column) as u32;
                        if !valid_anchors.get_bit(anchor) {
                            continue;
                        }
                        let contains = |(row, column): (i8, i8)| {
                            let row = anchor_row as isize + row as isize;
                            let column = anchor_column as isize + column as isize;
                            coordinate_is_valid(row, column, HEIGHT, WIDTH)
                                && nonzero.get_bit((row as usize * STRIDE + column as usize) as u32)
                        };
                        if cells.iter().copied().all(&contains)
                            && boundary.iter().copied().all(|cell| !contains(cell))
                        {
                            expected.set_bit(anchor);
                        }
                    }
                }

                assert_eq!(
                    detected[template_index] & valid_anchors,
                    expected,
                    "template {template_index}, pattern {pattern:09b}"
                );
            }
        }
    }

    fn assert_generated_states_are_allowed(pieces: &[Piece], height: u8, width: u8) {
        fn visit(
            bound: &SmallComponentBound,
            remaining_cells: &[u32],
            placements: &[Vec<(usize, usize, Bitboard)>],
            board: Board,
            next_piece: usize,
            piece_index: usize,
        ) {
            if next_piece == placements.len() {
                let deficit = board.total_deficit();
                let excess = remaining_cells[piece_index] - deficit;
                assert_eq!(excess % MODULUS as u32, 0);
                assert!(
                    bound.allows::<MODULUS>(&board, piece_index, excess / MODULUS as u32),
                    "rejected generated solvable state: {board:?}, suffix={piece_index}"
                );
                return;
            }
            for &(_, _, mask) in &placements[next_piece] {
                let mut predecessor = board;
                predecessor.undo_piece(mask);
                visit(
                    bound,
                    remaining_cells,
                    placements,
                    predecessor,
                    next_piece + 1,
                    piece_index,
                );
            }
        }

        let placements = pieces
            .iter()
            .map(|piece| piece.placements(height, width))
            .collect::<Vec<_>>();
        let mut remaining_cells = vec![0; pieces.len() + 1];
        for piece_index in (0..pieces.len()).rev() {
            remaining_cells[piece_index] =
                remaining_cells[piece_index + 1] + pieces[piece_index].cell_count();
        }
        let bound = bound(pieces, &Board::new_solved(height, width, MODULUS as u8));
        for piece_index in 0..pieces.len() {
            visit(
                &bound,
                &remaining_cells,
                &placements,
                Board::new_solved(height, width, MODULUS as u8),
                piece_index,
                piece_index,
            );
        }
    }

    #[test]
    fn allows_exhaustively_generated_solvable_states() {
        let pieces = [
            Piece::from_grid(&[&[true, true], &[true, false]]),
            Piece::from_grid(&[&[true, true]]),
            Piece::from_grid(&[&[true], &[true]]),
            Piece::from_grid(&[&[true]]),
        ];

        assert_generated_states_are_allowed(&pieces, 3, 3);
    }
}
