//! A small, self-contained seeded PRNG (SplitMix64). Generated inputs must be
//! reproducible from `(seed, index)` alone, on every platform.

#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// The generator for input `index` of a run with `seed`.
    pub fn for_case(seed: u64, index: u64) -> Rng {
        let mut r = Rng(seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        r.next_u64();
        Rng(r.next_u64())
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0)");
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i64
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 <= p && p > 0.0
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }

    /// Pick from `(weight, item)` pairs.
    pub fn weighted<'a, T>(&mut self, xs: &'a [(u32, T)]) -> &'a T {
        let total: u32 = xs.iter().map(|x| x.0).sum();
        let mut n = self.below(total as usize) as u32;
        for (w, x) in xs {
            if n < *w {
                return x;
            }
            n -= w;
        }
        unreachable!("weights sum to total")
    }

    pub fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            let j = self.below(i + 1);
            xs.swap(i, j);
        }
    }
}
