//! Seeded randomness. The OS TRNG is touched only by `trng_seed`, which callers use to
//! pick a seed; everything inside a run flows from `SimRng` sub-streams.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Draw a fresh seed from OS entropy, masked to 53 bits. Callers only: never used inside a run.
pub fn trng_seed() -> u64 {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).unwrap_or_else(|e| panic!("OS entropy unavailable: {e}"));
    u64::from_le_bytes(b) & mm_core::SEED_MASK
}

/// FNV-1a 64 over bytes. Also used for the per-stream constants and config hashes.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Root of all in-run randomness. Each subsystem asks for a named stream, so adding a
/// sensor never shifts the noise another subsystem sees.
#[derive(Debug, Clone, Copy)]
pub struct SimRng {
    seed: u64,
}

impl SimRng {
    pub fn new(seed: u64) -> Self {
        SimRng { seed }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Independent stream: seed plus a fixed constant derived from `name`.
    pub fn stream(&self, name: &str) -> Stream {
        let k = fnv1a(name.as_bytes()).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        Stream(ChaCha8Rng::seed_from_u64(self.seed.wrapping_add(k)))
    }
}

/// One ChaCha8 stream with the helpers the sim needs.
#[derive(Debug, Clone)]
pub struct Stream(ChaCha8Rng);

impl Stream {
    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        self.0.gen::<f64>()
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    /// Standard normal by Box-Muller (always consumes two uniforms).
    pub fn gauss(&mut self) -> f64 {
        let u1 = 1.0 - self.uniform();
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.uniform() < p
    }

    /// +1.0 or -1.0.
    pub fn sign(&mut self) -> f64 {
        if self.uniform() < 0.5 {
            -1.0
        } else {
            1.0
        }
    }

    /// Uniform index in 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        ((self.uniform() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream_and_streams_independent() {
        let a = SimRng::new(42);
        let x: Vec<f64> = {
            let mut s = a.stream("tof/a");
            (0..5).map(|_| s.gauss()).collect()
        };
        let y: Vec<f64> = {
            let mut s = SimRng::new(42).stream("tof/a");
            (0..5).map(|_| s.gauss()).collect()
        };
        assert_eq!(x, y);
        let z = a.stream("tof/b").gauss();
        assert_ne!(x[0], z);
    }

    #[test]
    fn gauss_is_roughly_standard() {
        let mut s = SimRng::new(7).stream("g");
        let n = 20000;
        let v: Vec<f64> = (0..n).map(|_| s.gauss()).collect();
        let mean = v.iter().sum::<f64>() / n as f64;
        let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.03, "mean {mean}");
        assert!((var - 1.0).abs() < 0.05, "var {var}");
    }

    #[test]
    fn trng_seed_is_masked() {
        assert_eq!(trng_seed() & !mm_core::SEED_MASK, 0);
    }
}
