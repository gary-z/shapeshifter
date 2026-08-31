//! Reachability bounds for checkerboard, row, column, and diagonal partitions.
//!
//! The deficit in each partition must be achievable by the remaining pieces.
//! A suffix dynamic program records the reachable totals.

use crate::core::STRIDE;
use crate::core::bitboard::Bitboard;
use crate::core::board::Board;
use crate::core::piece::Piece;

struct Partition {
    mask: Bitboard,
    suffix_max: Vec<u32>,
    suffix_min: Vec<u32>,
    suffix_reachable: Vec<Vec<bool>>,
}

pub(crate) struct PartitionReachability {
    partitions: Vec<Partition>,
}

impl PartitionReachability {
    pub(crate) fn precompute(
        pieces: &[Piece],
        piece_order: &[usize],
        height: u8,
        width: u8,
    ) -> Self {
        let board_height = height as usize;
        let board_width = width as usize;
        let piece_count = pieces.len();

        let build_partition = |contains_board_cell: &dyn Fn(usize, usize) -> bool,
                               offset_count: usize,
                               contains_piece_cell: &dyn Fn(usize, usize, usize) -> bool|
         -> Partition {
            let mut mask = Bitboard::ZERO;
            for row in 0..board_height {
                for column in 0..board_width {
                    if contains_board_cell(row, column) {
                        mask.set_bit((row * STRIDE + column) as u32);
                    }
                }
            }

            let mut partition_cell_counts = Vec::with_capacity(piece_count);
            for &piece_index in piece_order {
                let piece = &pieces[piece_index];
                let mut counts = vec![0u32; offset_count];
                for offset in 0..offset_count {
                    for piece_row in 0..piece.height() as usize {
                        for piece_column in 0..piece.width() as usize {
                            if piece
                                .shape()
                                .get_bit((piece_row * STRIDE + piece_column) as u32)
                                && contains_piece_cell(piece_row, piece_column, offset)
                            {
                                counts[offset] += 1;
                            }
                        }
                    }
                }
                partition_cell_counts.push(counts);
            }

            let mut suffix_max = vec![0u32; piece_count + 1];
            let mut suffix_min = vec![0u32; piece_count + 1];
            for piece_index in (0..piece_count).rev() {
                suffix_max[piece_index] = suffix_max[piece_index + 1]
                    + *partition_cell_counts[piece_index].iter().max().unwrap();
                suffix_min[piece_index] = suffix_min[piece_index + 1]
                    + *partition_cell_counts[piece_index].iter().min().unwrap();
            }

            let reachable_count = suffix_max[0] as usize + 1;
            let mut suffix_reachable = vec![vec![false; reachable_count]; piece_count + 1];
            suffix_reachable[piece_count][0] = true;
            for piece_index in (0..piece_count).rev() {
                for reachable_total in 0..reachable_count {
                    if suffix_reachable[piece_index + 1][reachable_total] {
                        for &cell_count in &partition_cell_counts[piece_index] {
                            let new_total = reachable_total + cell_count as usize;
                            if new_total < reachable_count {
                                suffix_reachable[piece_index][new_total] = true;
                            }
                        }
                    }
                }
            }

            Partition {
                mask,
                suffix_max,
                suffix_min,
                suffix_reachable,
            }
        };

        let mut partitions = Vec::new();

        // Modulo-two partitions.
        partitions.push(build_partition(
            &|row, column| (row + column) % 2 == 0,
            2,
            &|piece_row, piece_column, offset| (piece_row + piece_column + offset) % 2 == 0,
        ));
        partitions.push(build_partition(
            &|row, _| row % 2 == 0,
            2,
            &|piece_row, _, offset| (piece_row + offset) % 2 == 0,
        ));
        partitions.push(build_partition(
            &|_, column| column % 2 == 0,
            2,
            &|_, piece_column, offset| (piece_column + offset) % 2 == 0,
        ));

        // Modulo-three row and column partitions.
        if board_height >= 6 {
            for residue in 0..3usize {
                partitions.push(build_partition(
                    &|row, _| row % 3 == residue,
                    3,
                    &|piece_row, _, offset| (piece_row + offset) % 3 == residue,
                ));
            }
        }
        if board_width >= 6 {
            for residue in 0..3usize {
                partitions.push(build_partition(
                    &|_, column| column % 3 == residue,
                    3,
                    &|_, piece_column, offset| (piece_column + offset) % 3 == residue,
                ));
            }
        }

        // Modulo-three diagonal partitions.
        if board_height >= 4 && board_width >= 4 {
            for residue in 0..3usize {
                partitions.push(build_partition(
                    &|row, column| (row + column) % 3 == residue,
                    3,
                    &|piece_row, piece_column, offset| {
                        (piece_row + piece_column + offset) % 3 == residue
                    },
                ));
            }
        }
        if board_height >= 4 && board_width >= 4 {
            for residue in 0..3usize {
                partitions.push(build_partition(
                    &|row, column| (row + 2 * column) % 3 == residue,
                    3,
                    &|piece_row, piece_column, offset| {
                        (piece_row + 2 * piece_column + offset) % 3 == residue
                    },
                ));
            }
        }

        Self { partitions }
    }

    #[inline(always)]
    pub(crate) fn allows(
        &self,
        board: &Board,
        piece_index: usize,
        modulus: u8,
        remaining_cells: u32,
    ) -> bool {
        let modulus_u32 = modulus as u32;
        let total_deficit = board.total_deficit();

        for partition in &self.partitions {
            let mut partition_deficit = 0u32;
            for deficit in 1..modulus {
                partition_deficit +=
                    deficit as u32 * (board.plane(deficit) & partition.mask).count_ones();
            }

            if partition.suffix_max[piece_index] < partition_deficit {
                return false;
            }
            let complement_deficit = total_deficit - partition_deficit;
            let max_complement_coverage = remaining_cells - partition.suffix_min[piece_index];
            if max_complement_coverage < complement_deficit {
                return false;
            }

            let reachable = &partition.suffix_reachable[piece_index];
            let mut target = partition_deficit;
            let mut found = false;
            while (target as usize) < reachable.len() {
                if reachable[target as usize] {
                    found = true;
                    break;
                }
                target += modulus_u32;
            }
            if !found {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Board;
    use crate::core::piece::Piece;

    #[test]
    fn creates_small_board_partitions() {
        let piece = Piece::from_grid(&[&[true]]);
        let pieces = vec![piece];
        let piece_order = vec![0];
        let reachability = PartitionReachability::precompute(&pieces, &piece_order, 3, 3);
        assert_eq!(reachability.partitions.len(), 3);
    }

    #[test]
    fn creates_modulo_three_partitions_for_large_boards() {
        let piece = Piece::from_grid(&[&[true]]);
        let pieces = vec![piece];
        let piece_order = vec![0];
        let reachability = PartitionReachability::precompute(&pieces, &piece_order, 6, 6);
        assert_eq!(reachability.partitions.len(), 15);
    }

    #[test]
    fn allows_solved_board() {
        let board = Board::new_solved(3, 3, 2);
        let piece = Piece::from_grid(&[&[true]]);
        let pieces = vec![piece];
        let piece_order = vec![0];
        let reachability = PartitionReachability::precompute(&pieces, &piece_order, 3, 3);
        assert!(reachability.allows(&board, 0, 2, 1));
    }
}
