use std::simd::{u64x4, num::SimdUint, cmp::SimdPartialEq};

/// A 256-bit bitboard stored as a SIMD u64x4 vector.
/// Bit index `i` lives in lane `i / 64`, bit position `i % 64`.
///
/// Uses `std::simd` portable SIMD for branchless 256-bit operations.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Bitboard {
    v: u64x4,
}

impl Default for Bitboard {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Bitboard {
    pub const ZERO: Bitboard = Bitboard { v: u64x4::from_array([0; 4]) };

    #[inline(always)]
    pub const fn new(limbs: [u64; 4]) -> Self {
        Self { v: u64x4::from_array(limbs) }
    }

    /// Access the underlying limbs (for backwards compat).
    #[inline(always)]
    pub fn limbs(&self) -> [u64; 4] {
        self.v.to_array()
    }

    /// Create a bitboard with a single bit set.
    #[inline(always)]
    pub fn from_bit(index: u32) -> Self {
        let mut arr = [0u64; 4];
        arr[index as usize / 64] = 1u64 << (index % 64);
        Self { v: u64x4::from_array(arr) }
    }

    /// Check if a specific bit is set.
    #[inline(always)]
    pub fn get_bit(&self, index: u32) -> bool {
        let arr = self.v.to_array();
        (arr[index as usize / 64] >> (index % 64)) & 1 != 0
    }

    /// Set a specific bit.
    #[inline(always)]
    pub fn set_bit(&mut self, index: u32) {
        let mut arr = self.v.to_array();
        arr[index as usize / 64] |= 1u64 << (index % 64);
        self.v = u64x4::from_array(arr);
    }

    /// Clear a specific bit.
    #[inline(always)]
    pub fn clear_bit(&mut self, index: u32) {
        let mut arr = self.v.to_array();
        arr[index as usize / 64] &= !(1u64 << (index % 64));
        self.v = u64x4::from_array(arr);
    }

    /// Returns true if all bits are zero.
    #[inline(always)]
    pub fn is_zero(&self) -> bool {
        self.v.simd_eq(u64x4::splat(0)).all()
    }

    /// Return the index of the lowest set bit, or 256 if none.
    #[inline(always)]
    pub fn lowest_set_bit(&self) -> u32 {
        let arr = self.v.to_array();
        if arr[0] != 0 { return arr[0].trailing_zeros(); }
        if arr[1] != 0 { return 64 + arr[1].trailing_zeros(); }
        if arr[2] != 0 { return 128 + arr[2].trailing_zeros(); }
        if arr[3] != 0 { return 192 + arr[3].trailing_zeros(); }
        256
    }

    /// Count the number of set bits across all 256 bits.
    /// Uses SIMD lane-wise popcount and horizontal reduction.
    #[inline(always)]
    pub fn count_ones(&self) -> u32 {
        self.v.count_ones().reduce_sum() as u32
    }

    /// Const-compatible scalar count_ones.
    #[inline(always)]
    pub const fn count_ones_const(&self) -> u32 {
        let arr = self.v.to_array();
        arr[0].count_ones()
            + arr[1].count_ones()
            + arr[2].count_ones()
            + arr[3].count_ones()
    }

    /// Shift right by 1 bit (one cell horizontally). Specialized for performance.
    #[inline(always)]
    pub fn shr_1(&self) -> Self {
        let arr = self.v.to_array();
        Self { v: u64x4::from_array([
            (arr[0] >> 1) | (arr[1] << 63),
            (arr[1] >> 1) | (arr[2] << 63),
            (arr[2] >> 1) | (arr[3] << 63),
            arr[3] >> 1,
        ])}
    }

    /// Shift left by 1 bit (one cell horizontally). Specialized for performance.
    #[inline(always)]
    pub fn shl_1(&self) -> Self {
        let arr = self.v.to_array();
        Self { v: u64x4::from_array([
            arr[0] << 1,
            (arr[1] << 1) | (arr[0] >> 63),
            (arr[2] << 1) | (arr[1] >> 63),
            (arr[3] << 1) | (arr[2] >> 63),
        ])}
    }

    /// Shift right by STRIDE bits (one cell vertically). Specialized for performance.
    #[inline(always)]
    pub fn shr_stride(&self) -> Self {
        let arr = self.v.to_array();
        Self { v: u64x4::from_array([
            (arr[0] >> crate::core::STRIDE) | (arr[1] << (64 - crate::core::STRIDE)),
            (arr[1] >> crate::core::STRIDE) | (arr[2] << (64 - crate::core::STRIDE)),
            (arr[2] >> crate::core::STRIDE) | (arr[3] << (64 - crate::core::STRIDE)),
            arr[3] >> crate::core::STRIDE,
        ])}
    }

    /// Shift left by STRIDE bits (one cell vertically). Specialized for performance.
    #[inline(always)]
    pub fn shl_stride(&self) -> Self {
        let arr = self.v.to_array();
        Self { v: u64x4::from_array([
            arr[0] << crate::core::STRIDE,
            (arr[1] << crate::core::STRIDE) | (arr[0] >> (64 - crate::core::STRIDE)),
            (arr[2] << crate::core::STRIDE) | (arr[1] >> (64 - crate::core::STRIDE)),
            (arr[3] << crate::core::STRIDE) | (arr[2] >> (64 - crate::core::STRIDE)),
        ])}
    }

    /// Shift left by `n` bits. Bits shifted beyond 256 are lost.
    pub fn shl(&self, n: u32) -> Self {
        if n >= 256 {
            return Self::ZERO;
        }
        let limb_shift = (n / 64) as usize;
        let bit_shift = n % 64;
        let arr = self.v.to_array();

        let mut result = [0u64; 4];
        for i in limb_shift..4 {
            let src = i - limb_shift;
            result[i] = arr[src] << bit_shift;
            if bit_shift > 0 && src > 0 {
                result[i] |= arr[src - 1] >> (64 - bit_shift);
            }
        }
        Self { v: u64x4::from_array(result) }
    }

    /// Shift right by `n` bits. Bits shifted below 0 are lost.
    pub fn shr(&self, n: u32) -> Self {
        if n >= 256 {
            return Self::ZERO;
        }
        let limb_shift = (n / 64) as usize;
        let bit_shift = n % 64;
        let arr = self.v.to_array();

        let mut result = [0u64; 4];
        for i in 0..4 {
            let src = i + limb_shift;
            if src < 4 {
                result[i] = arr[src] >> bit_shift;
                if bit_shift > 0 && src + 1 < 4 {
                    result[i] |= arr[src + 1] << (64 - bit_shift);
                }
            }
        }
        Self { v: u64x4::from_array(result) }
    }

    /// Bitwise AND.
    #[inline(always)]
    pub fn and(&self, other: &Bitboard) -> Self {
        Self { v: self.v & other.v }
    }

    /// Bitwise OR.
    #[inline(always)]
    pub fn or(&self, other: &Bitboard) -> Self {
        Self { v: self.v | other.v }
    }

    /// Bitwise XOR.
    #[inline(always)]
    pub fn xor(&self, other: &Bitboard) -> Self {
        Self { v: self.v ^ other.v }
    }

    /// Bitwise NOT (inverts all 256 bits).
    #[inline(always)]
    pub fn not(&self) -> Self {
        Self { v: !self.v }
    }
}

// ---------------------------------------------------------------------------
// Operator trait impls — all delegate to SIMD vector ops
// ---------------------------------------------------------------------------

impl std::ops::BitAnd for Bitboard {
    type Output = Self;
    #[inline(always)]
    fn bitand(self, rhs: Self) -> Self {
        Self { v: self.v & rhs.v }
    }
}

impl std::ops::BitOr for Bitboard {
    type Output = Self;
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self {
        Self { v: self.v | rhs.v }
    }
}

impl std::ops::BitXor for Bitboard {
    type Output = Self;
    #[inline(always)]
    fn bitxor(self, rhs: Self) -> Self {
        Self { v: self.v ^ rhs.v }
    }
}

impl std::ops::Not for Bitboard {
    type Output = Self;
    #[inline(always)]
    fn not(self) -> Self {
        Self { v: !self.v }
    }
}

impl std::ops::Shl<u32> for Bitboard {
    type Output = Self;
    #[inline(always)]
    fn shl(self, n: u32) -> Self {
        Bitboard::shl(&self, n)
    }
}

impl std::ops::Shr<u32> for Bitboard {
    type Output = Self;
    #[inline(always)]
    fn shr(self, n: u32) -> Self {
        Bitboard::shr(&self, n)
    }
}

impl std::ops::BitAndAssign for Bitboard {
    #[inline(always)]
    fn bitand_assign(&mut self, rhs: Self) {
        self.v &= rhs.v;
    }
}

impl std::ops::BitOrAssign for Bitboard {
    #[inline(always)]
    fn bitor_assign(&mut self, rhs: Self) {
        self.v |= rhs.v;
    }
}

impl std::ops::BitXorAssign for Bitboard {
    #[inline(always)]
    fn bitxor_assign(&mut self, rhs: Self) {
        self.v ^= rhs.v;
    }
}

impl std::fmt::Debug for Bitboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let arr = self.v.to_array();
        write!(
            f,
            "Bitboard([{:#018x}, {:#018x}, {:#018x}, {:#018x}])",
            arr[0], arr[1], arr[2], arr[3]
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
        assert_eq!(b.limbs()[0], 1u64 << 63);

        let b = Bitboard::from_bit(64);
        assert!(b.get_bit(64));
        assert_eq!(b.limbs()[1], 1);

        let b = Bitboard::from_bit(255);
        assert!(b.get_bit(255));
        assert_eq!(b.limbs()[3], 1u64 << 63);
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
        assert_eq!(b.limbs(), [u64::MAX; 4]);
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
        a.set_bit(15);
        let b = a << 30;
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
        let row = 3;
        let col = 7;
        let index = row * 15 + col;
        let b = Bitboard::from_bit(index);
        assert!(b.get_bit(index));
        assert_eq!(index, 52);
    }

    #[test]
    fn test_max_board_index() {
        let index = 13 * 15 + 13;
        assert_eq!(index, 208);
        let b = Bitboard::from_bit(index);
        assert!(b.get_bit(index));
        assert_eq!(b.count_ones(), 1);
    }

    #[test]
    fn test_shl_preserves_pattern() {
        let mut piece = Bitboard::ZERO;
        piece.set_bit(0);
        piece.set_bit(1);
        piece.set_bit(15);
        piece.set_bit(16);

        let offset = 2 * 15 + 3;
        let placed = piece << offset;

        assert!(placed.get_bit(2 * 15 + 3));
        assert!(placed.get_bit(2 * 15 + 4));
        assert!(placed.get_bit(3 * 15 + 3));
        assert!(placed.get_bit(3 * 15 + 4));
        assert_eq!(placed.count_ones(), 4);
    }
}
