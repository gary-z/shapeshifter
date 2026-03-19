/// A 256-bit bitboard stored as four u64 limbs in little-endian order.
/// Bit index `i` lives in limb `i / 64`, bit position `i % 64`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Bitboard {
    pub limbs: [u64; 4],
}

impl Bitboard {
    pub const ZERO: Bitboard = Bitboard { limbs: [0; 4] };

    pub const fn new(limbs: [u64; 4]) -> Self {
        Self { limbs }
    }

    /// Create a bitboard with a single bit set.
    pub const fn from_bit(index: u32) -> Self {
        let mut limbs = [0u64; 4];
        limbs[index as usize / 64] = 1u64 << (index % 64);
        Self { limbs }
    }

    /// Check if a specific bit is set.
    pub const fn get_bit(&self, index: u32) -> bool {
        let limb = self.limbs[index as usize / 64];
        (limb >> (index % 64)) & 1 != 0
    }

    /// Set a specific bit.
    pub fn set_bit(&mut self, index: u32) {
        self.limbs[index as usize / 64] |= 1u64 << (index % 64);
    }

    /// Clear a specific bit.
    pub fn clear_bit(&mut self, index: u32) {
        self.limbs[index as usize / 64] &= !(1u64 << (index % 64));
    }

    /// Returns true if all bits are zero.
    pub const fn is_zero(&self) -> bool {
        self.limbs[0] == 0 && self.limbs[1] == 0 && self.limbs[2] == 0 && self.limbs[3] == 0
    }

    /// Return the index of the lowest set bit, or 256 if none.
    pub const fn lowest_set_bit(&self) -> u32 {
        if self.limbs[0] != 0 {
            return self.limbs[0].trailing_zeros();
        }
        if self.limbs[1] != 0 {
            return 64 + self.limbs[1].trailing_zeros();
        }
        if self.limbs[2] != 0 {
            return 128 + self.limbs[2].trailing_zeros();
        }
        if self.limbs[3] != 0 {
            return 192 + self.limbs[3].trailing_zeros();
        }
        256
    }

    /// Count the number of set bits.
    pub const fn count_ones(&self) -> u32 {
        self.limbs[0].count_ones()
            + self.limbs[1].count_ones()
            + self.limbs[2].count_ones()
            + self.limbs[3].count_ones()
    }

    /// Shift left by `n` bits. Bits shifted beyond 256 are lost.
    pub const fn shl(&self, n: u32) -> Self {
        if n >= 256 {
            return Self::ZERO;
        }
        let limb_shift = (n / 64) as usize;
        let bit_shift = n % 64;

        let mut result = [0u64; 4];
        let mut i = 3;
        // Manual loop because for loops aren't allowed in const fn
        while i < 4 {
            // i is usize, so check i >= limb_shift
            if i >= limb_shift {
                let src = i - limb_shift;
                result[i] = self.limbs[src] << bit_shift;
                if bit_shift > 0 && src > 0 {
                    result[i] |= self.limbs[src - 1] >> (64 - bit_shift);
                }
            }
            if i == 0 {
                break;
            }
            i = i.wrapping_sub(1);
        }
        // The while loop above starts at 3 and decrements. Let me redo this properly.
        Self { limbs: result }
    }

    /// Shift right by `n` bits. Bits shifted below 0 are lost.
    pub const fn shr(&self, n: u32) -> Self {
        if n >= 256 {
            return Self::ZERO;
        }
        let limb_shift = (n / 64) as usize;
        let bit_shift = n % 64;

        let mut result = [0u64; 4];
        let mut i = 0;
        while i < 4 {
            let src = i + limb_shift;
            if src < 4 {
                result[i] = self.limbs[src] >> bit_shift;
                if bit_shift > 0 && src + 1 < 4 {
                    result[i] |= self.limbs[src + 1] << (64 - bit_shift);
                }
            }
            i += 1;
        }
        Self { limbs: result }
    }

    /// Bitwise AND.
    pub const fn and(&self, other: &Bitboard) -> Self {
        Self {
            limbs: [
                self.limbs[0] & other.limbs[0],
                self.limbs[1] & other.limbs[1],
                self.limbs[2] & other.limbs[2],
                self.limbs[3] & other.limbs[3],
            ],
        }
    }

    /// Bitwise OR.
    pub const fn or(&self, other: &Bitboard) -> Self {
        Self {
            limbs: [
                self.limbs[0] | other.limbs[0],
                self.limbs[1] | other.limbs[1],
                self.limbs[2] | other.limbs[2],
                self.limbs[3] | other.limbs[3],
            ],
        }
    }

    /// Bitwise XOR.
    pub const fn xor(&self, other: &Bitboard) -> Self {
        Self {
            limbs: [
                self.limbs[0] ^ other.limbs[0],
                self.limbs[1] ^ other.limbs[1],
                self.limbs[2] ^ other.limbs[2],
                self.limbs[3] ^ other.limbs[3],
            ],
        }
    }

    /// Bitwise NOT (inverts all 256 bits).
    pub const fn not(&self) -> Self {
        Self {
            limbs: [
                !self.limbs[0],
                !self.limbs[1],
                !self.limbs[2],
                !self.limbs[3],
            ],
        }
    }
}

impl std::ops::BitAnd for Bitboard {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        self.and(&rhs)
    }
}

impl std::ops::BitOr for Bitboard {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.or(&rhs)
    }
}

impl std::ops::BitXor for Bitboard {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self {
        self.xor(&rhs)
    }
}

impl std::ops::Not for Bitboard {
    type Output = Self;
    fn not(self) -> Self {
        Bitboard::not(&self)
    }
}

impl std::ops::Shl<u32> for Bitboard {
    type Output = Self;
    fn shl(self, n: u32) -> Self {
        Bitboard::shl(&self, n)
    }
}

impl std::ops::Shr<u32> for Bitboard {
    type Output = Self;
    fn shr(self, n: u32) -> Self {
        Bitboard::shr(&self, n)
    }
}

impl std::ops::BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = *self & rhs;
    }
}

impl std::ops::BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

impl std::ops::BitXorAssign for Bitboard {
    fn bitxor_assign(&mut self, rhs: Self) {
        *self = *self ^ rhs;
    }
}

impl std::fmt::Debug for Bitboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Bitboard([{:#018x}, {:#018x}, {:#018x}, {:#018x}])",
            self.limbs[0], self.limbs[1], self.limbs[2], self.limbs[3]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero() {
        let b = Bitboard::ZERO;
        assert!(b.is_zero());
        assert_eq!(b.count_ones(), 0);
    }

    #[test]
    fn test_from_bit() {
        let b = Bitboard::from_bit(0);
        assert!(b.get_bit(0));
        assert!(!b.get_bit(1));
        assert_eq!(b.count_ones(), 1);

        let b = Bitboard::from_bit(63);
        assert!(b.get_bit(63));
        assert_eq!(b.limbs[0], 1u64 << 63);

        let b = Bitboard::from_bit(64);
        assert!(b.get_bit(64));
        assert_eq!(b.limbs[1], 1);

        let b = Bitboard::from_bit(255);
        assert!(b.get_bit(255));
        assert_eq!(b.limbs[3], 1u64 << 63);
    }

    #[test]
    fn test_set_clear_bit() {
        let mut b = Bitboard::ZERO;
        b.set_bit(100);
        assert!(b.get_bit(100));
        assert_eq!(b.count_ones(), 1);

        b.clear_bit(100);
        assert!(!b.get_bit(100));
        assert!(b.is_zero());
    }

    #[test]
    fn test_count_ones() {
        let mut b = Bitboard::ZERO;
        b.set_bit(0);
        b.set_bit(64);
        b.set_bit(128);
        b.set_bit(192);
        assert_eq!(b.count_ones(), 4);
    }

    #[test]
    fn test_bitwise_ops() {
        let a = Bitboard::from_bit(10);
        let b = Bitboard::from_bit(20);
        let c = a | b;
        assert!(c.get_bit(10));
        assert!(c.get_bit(20));
        assert_eq!(c.count_ones(), 2);

        let d = c & a;
        assert!(d.get_bit(10));
        assert!(!d.get_bit(20));

        let e = a ^ a;
        assert!(e.is_zero());
    }

    #[test]
    fn test_not() {
        let a = Bitboard::ZERO;
        let b = !a;
        assert_eq!(b.limbs, [u64::MAX; 4]);
        let c = !b;
        assert!(c.is_zero());
    }

    #[test]
    fn test_shl_small() {
        let a = Bitboard::from_bit(0);
        let b = a << 10;
        assert!(b.get_bit(10));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shl_cross_limb() {
        let a = Bitboard::from_bit(60);
        let b = a << 10;
        assert!(b.get_bit(70));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shl_full_limb() {
        let a = Bitboard::from_bit(0);
        let b = a << 64;
        assert!(b.get_bit(64));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shl_overflow() {
        let a = Bitboard::from_bit(200);
        let b = a << 100;
        // 200 + 100 = 300 >= 256, should be lost
        assert!(b.is_zero());
    }

    #[test]
    fn test_shl_256() {
        let a = Bitboard::from_bit(0);
        let b = a << 256;
        assert!(b.is_zero());
    }

    #[test]
    fn test_shr_small() {
        let a = Bitboard::from_bit(10);
        let b = a >> 5;
        assert!(b.get_bit(5));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shr_cross_limb() {
        let a = Bitboard::from_bit(70);
        let b = a >> 10;
        assert!(b.get_bit(60));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shr_full_limb() {
        let a = Bitboard::from_bit(64);
        let b = a >> 64;
        assert!(b.get_bit(0));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shr_underflow() {
        let a = Bitboard::from_bit(10);
        let b = a >> 20;
        assert!(b.is_zero());
    }

    #[test]
    fn test_shl_multiple_bits() {
        let mut a = Bitboard::ZERO;
        a.set_bit(0);
        a.set_bit(1);
        a.set_bit(15); // row stride
        let b = a << 30; // shift by 2 rows
        assert!(b.get_bit(30));
        assert!(b.get_bit(31));
        assert!(b.get_bit(45));
        assert_eq!(b.count_ones(), 3);
    }

    #[test]
    fn test_assign_ops() {
        let mut a = Bitboard::from_bit(5);
        a |= Bitboard::from_bit(10);
        assert_eq!(a.count_ones(), 2);

        a &= Bitboard::from_bit(5);
        assert_eq!(a.count_ones(), 1);
        assert!(a.get_bit(5));

        a ^= Bitboard::from_bit(5);
        assert!(a.is_zero());
    }

    #[test]
    fn test_equality() {
        let a = Bitboard::from_bit(42);
        let b = Bitboard::from_bit(42);
        let c = Bitboard::from_bit(43);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_board_index_convention() {
        // Verify the 15-column stride: (row, col) -> row * 15 + col
        let row = 3;
        let col = 7;
        let index = row * 15 + col;
        let b = Bitboard::from_bit(index);
        assert!(b.get_bit(index));
        assert_eq!(index, 52); // 3*15 + 7 = 52
    }

    #[test]
    fn test_max_board_index() {
        // 14x14 board: max cell is (13, 13) = 13*15 + 13 = 208
        let index = 13 * 15 + 13;
        assert_eq!(index, 208);
        let b = Bitboard::from_bit(index);
        assert!(b.get_bit(index));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shl_preserves_pattern() {
        // Place a 2x2 block at (0,0) and shift to (2,3)
        let mut piece = Bitboard::ZERO;
        piece.set_bit(0);      // (0,0)
        piece.set_bit(1);      // (0,1)
        piece.set_bit(15);     // (1,0)
        piece.set_bit(16);     // (1,1)

        let offset = 2 * 15 + 3; // shift to row 2, col 3
        let placed = piece << offset;

        assert!(placed.get_bit(2 * 15 + 3)); // (2,3)
        assert!(placed.get_bit(2 * 15 + 4)); // (2,4)
        assert!(placed.get_bit(3 * 15 + 3)); // (3,3)
        assert!(placed.get_bit(3 * 15 + 4)); // (3,4)
        assert_eq!(placed.count_ones(), 4);
    }
}
