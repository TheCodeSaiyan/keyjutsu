//! Visual pacing for Auto Performance.
//!
//! These delays decide how quickly characters *appear*. They never decide when
//! a command has finished or when the next step may start; that comes from the
//! shell. The generator is seeded so a test, or a rehearsal, sees the same
//! rhythm every time.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Cadence {
    /// Mean gap between characters.
    pub base_ms: u32,
    /// Maximum random deviation either side of the mean. Zero disables it.
    pub variance_ms: u32,
    /// Extra pause after punctuation and operators.
    pub punctuation_pause_ms: u32,
    /// Pause before a command starts and before it is submitted.
    pub boundary_pause_ms: u32,
}

impl Default for Cadence {
    fn default() -> Self {
        Self { base_ms: 55, variance_ms: 30, punctuation_pause_ms: 110, boundary_pause_ms: 550 }
    }
}

/// xorshift64*: small, fast and plenty for typing rhythm. Not for anything
/// security-related.
#[derive(Debug, Clone)]
pub struct Humaniser {
    state: u64,
}

impl Humaniser {
    pub fn new(seed: u64) -> Self {
        Self { state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed } }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// The gap to leave after typing `just_typed`.
    pub fn delay_after(&mut self, cadence: &Cadence, just_typed: char) -> Duration {
        let mut ms = i64::from(cadence.base_ms);
        if cadence.variance_ms > 0 {
            let span = u64::from(cadence.variance_ms) * 2 + 1;
            ms += (self.next() % span) as i64 - i64::from(cadence.variance_ms);
        }
        if matches!(just_typed, '.' | ',' | ';' | '|' | '&' | '(' | ')' | '{' | '}' | '=' | '-') {
            ms += i64::from(cadence.punctuation_pause_ms);
        }
        Duration::from_millis(ms.max(5) as u64)
    }

    pub fn boundary(&self, cadence: &Cadence) -> Duration {
        Duration::from_millis(u64::from(cadence.boundary_pause_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_stay_within_the_configured_band() {
        let c = Cadence::default();
        let mut h = Humaniser::new(42);
        for _ in 0..1000 {
            let d = h.delay_after(&c, 'a').as_millis() as u32;
            assert!((c.base_ms - c.variance_ms..=c.base_ms + c.variance_ms).contains(&d), "{d}");
        }
        let d = h.delay_after(&c, '|').as_millis() as u32;
        assert!(d >= c.base_ms - c.variance_ms + c.punctuation_pause_ms);
    }

    #[test]
    fn the_same_seed_gives_the_same_rhythm() {
        let c = Cadence::default();
        let a: Vec<_> = {
            let mut h = Humaniser::new(7);
            (0..50).map(|_| h.delay_after(&c, 'x')).collect()
        };
        let b: Vec<_> = {
            let mut h = Humaniser::new(7);
            (0..50).map(|_| h.delay_after(&c, 'x')).collect()
        };
        assert_eq!(a, b);
    }

    #[test]
    fn zero_variance_is_perfectly_regular() {
        let c = Cadence { variance_ms: 0, ..Cadence::default() };
        let mut h = Humaniser::new(1);
        assert!((0..20).all(|_| h.delay_after(&c, 'x') == Duration::from_millis(u64::from(c.base_ms))));
    }
}
