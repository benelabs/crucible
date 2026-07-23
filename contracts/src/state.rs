// Gas Optimization via Bit-Packing
// Packing multiple booleans and small integers into a single 64-bit storage key.
// [bool flag1] [bool flag2] [u8 small_int] [u32 large_int] [16 bits reserved]
// Total = 1 + 1 + 8 + 32 = 42 bits used, easily fits in u64.

#[derive(Clone, Copy)]
pub struct PackedState {
    pub packed: u64,
}

impl PackedState {
    pub fn new(flag1: bool, flag2: bool, small_int: u8, large_int: u32) -> Self {
        let mut packed: u64 = 0;
        if flag1 { packed |= 1 << 0; }
        if flag2 { packed |= 1 << 1; }
        packed |= (small_int as u64) << 2;
        packed |= (large_int as u64) << 10;
        Self { packed }
    }

    pub fn flag1(&self) -> bool {
        (self.packed & (1 << 0)) != 0
    }

    pub fn flag2(&self) -> bool {
        (self.packed & (1 << 1)) != 0
    }

    pub fn small_int(&self) -> u8 {
        ((self.packed >> 2) & 0xFF) as u8
    }

    pub fn large_int(&self) -> u32 {
        ((self.packed >> 10) & 0xFFFFFFFF) as u32
    }
}
