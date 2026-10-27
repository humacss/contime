use std::num::NonZeroU64;

/// Reproducible SplitMix64 randomness for simulation, not cryptography.
///
/// The algorithm uses wrapping 64-bit integer arithmetic on every platform.
/// Equal seeds and calls produce equal values on native and WebAssembly hosts.
#[derive(Clone, Debug)]
pub struct Random {
    state: u64,
}

impl Random {
    /// Starts a deterministic sequence. Every seed, including zero, is valid.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Returns the next value from the full unsigned 64-bit range.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    /// Samples uniformly from `0..upper`, avoiding modulo bias.
    ///
    /// Rejection sampling can consume more than one random value per call.
    pub fn below(&mut self, upper: NonZeroU64) -> u64 {
        let upper = upper.get();
        let threshold = upper.wrapping_neg() % upper;
        loop {
            let value = self.next_u64();
            if value >= threshold {
                return value % upper;
            }
        }
    }
}
