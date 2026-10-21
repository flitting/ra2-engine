//! The scenario random stream in the installed RA2 1.006 and YR 1.001.
//!
//! Constructor / next / inclusive range:
//! RA2 0x638790 / 0x638840 / 0x6388a0;
//! YR  0x65c6d0 / 0x65c780 / 0x65c7e0.
//! The original instructions and all 250 state words were checked after every
//! operation in a two-edition, 8,597-operation native CPU oracle. This establishes
//! the generator, not the callers' original random-consumption order or seed.

use serde::{Deserialize, Serialize};

const WORDS: usize = 250;
const SEPARATION: usize = 103;
const C1: [u32; 4] = [0xbaa9_6887, 0x1e17_d32c, 0x03bc_dc3c, 0x0f33_d1b2];
const C2: [u32; 4] = [0x4b0f_3b58, 0xe874_f0c3, 0x6955_c5a6, 0x55a7_ca46];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "StoredRandom", try_from = "StoredRandom")]
pub struct NativeRandom {
    frozen: bool,
    next1: usize,
    next2: usize,
    table: [u32; WORDS],
}

/// The native object has three padding bytes after its flag. They are not RNG
/// state and are intentionally absent from this portable serialization.
#[derive(Serialize, Deserialize)]
struct StoredRandom {
    frozen: bool,
    next1: u16,
    next2: u16,
    table: Vec<u32>,
}

impl From<NativeRandom> for StoredRandom {
    fn from(random: NativeRandom) -> Self {
        Self { frozen: random.frozen, next1: random.next1 as u16, next2: random.next2 as u16, table: random.table.to_vec() }
    }
}

impl TryFrom<StoredRandom> for NativeRandom {
    type Error = &'static str;

    fn try_from(stored: StoredRandom) -> Result<Self, Self::Error> {
        let random = Self {
            frozen: stored.frozen,
            next1: usize::from(stored.next1),
            next2: usize::from(stored.next2),
            table: stored.table.try_into().map_err(|_| "native RNG must contain exactly 250 words")?,
        };
        if !random.valid() { return Err("native RNG indices are invalid or have lost their separation") }
        Ok(random)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeTooWide;

impl std::fmt::Display for RangeTooWide {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("native RNG range spans more than i32::MAX and may not terminate in the original")
    }
}

impl std::error::Error for RangeTooWide {}

impl NativeRandom {
    /// Seed mixing is integer-only. In particular, the original uses SAR on
    /// both signed intermediates below; replacing it with a rotate is wrong.
    pub fn seed(seed: u32) -> Self {
        let mut table = [0; WORDS];
        for (counter, word) in table.iter_mut().enumerate() {
            let (mut left, mut right) = (seed, counter as u32);
            for (&c1, &c2) in C1.iter().zip(C2.iter()) {
                let original = right;
                let work = right ^ c1;
                let low = (work & 0xffff) as i32;
                let high = (work as i32) >> 16;
                let mixed = (!high.wrapping_mul(high)).wrapping_add(low.wrapping_mul(low));
                let swapped = ((mixed >> 16) | mixed.wrapping_shl(16)) as u32;
                right = (swapped ^ c2).wrapping_add(high.wrapping_mul(low) as u32) ^ left;
                left = original;
            }
            *word = right;
        }
        Self { frozen: false, next1: 0, next2: SEPARATION, table }
    }

    pub fn valid(&self) -> bool {
        self.next1 < WORDS && self.next2 < WORDS
            && (self.next2 + WORDS - self.next1) % WORDS == SEPARATION
    }

    pub fn set_frozen(&mut self, frozen: bool) { self.frozen = frozen; }
    pub fn is_frozen(&self) -> bool { self.frozen }
    pub fn indices(&self) -> (usize, usize) { (self.next1, self.next2) }
    pub fn table_words(&self) -> &[u32; WORDS] { &self.table }

    /// Full original 32 bits, including the sign bit. A frozen stream returns
    /// zero without writing the table or advancing either index.
    pub fn next(&mut self) -> u32 {
        if self.frozen { return 0 }
        let value = self.table[self.next1] ^ self.table[self.next2];
        self.table[self.next1] = value;
        self.next1 = (self.next1 + 1) % WORDS;
        self.next2 = (self.next2 + 1) % WORDS;
        value
    }

    /// Includes both endpoints and accepts them in either order. Equal
    /// endpoints consume no state. Non-power-of-two widths can consume several
    /// words because the original masks and rejects, rather than using modulo.
    pub fn ranged(&mut self, minimum: i32, maximum: i32) -> i32 {
        self.try_ranged(minimum, maximum).expect("unsupported nonterminating native RNG range")
    }

    pub fn try_ranged(&mut self, minimum: i32, maximum: i32) -> Result<i32, RangeTooWide> {
        if minimum == maximum { return Ok(minimum) }
        let (minimum, maximum) = if minimum > maximum { (maximum, minimum) } else { (minimum, maximum) };
        let span = (maximum as u32).wrapping_sub(minimum as u32);
        if span > i32::MAX as u32 { return Err(RangeTooWide) }
        let mut value = span.wrapping_add(1);
        // Preserve the original signed-overflow branch for span=INT_MAX:
        // it returns this wrapped intermediate without consuming RNG state.
        if (value as i32) > span as i32 {
            let mask = u32::MAX >> span.leading_zeros();
            loop {
                value = self.next() & mask;
                if value <= span { break }
            }
        }
        Ok(minimum.wrapping_add(value as i32))
    }
}

impl Default for NativeRandom {
    /// The original Scenario constructor uses zero before the game setup
    /// reseeds it. This does not imply that campaigns have a fixed map seed.
    fn default() -> Self { Self::seed(0) }
}
