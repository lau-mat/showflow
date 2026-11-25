//! Slows down code guessing: per address a growing pause, and when many addresses fail at once,
//! code entry pauses for everyone (and the host is told).

use std::collections::{HashMap, VecDeque};

/// Wrong tries on one connection before it is closed.
pub const MAX_TRIES_PER_CONNECTION: u32 = 5;
const FREE_TRIES_PER_ADDRESS: u32 = 2;
const MAX_ADDRESS_DELAY: f64 = 60.0;
const GLOBAL_FAILURES: usize = 20;
const GLOBAL_WINDOW: f64 = 60.0;
const GLOBAL_PAUSE: f64 = 60.0;

#[derive(Default)]
pub struct CodeLimiter {
    /// address -> (consecutive failures, blocked until)
    addresses: HashMap<String, (u32, f64)>,
    recent: VecDeque<f64>,
    paused_until: f64,
}

impl CodeLimiter {
    /// Seconds the address must still wait (rounded up), or `None` if it may try now.
    pub fn wait(&self, address: &str, now: f64) -> Option<u64> {
        let until = self.addresses.get(address).map_or(0.0, |entry| entry.1).max(self.paused_until);
        (until > now).then(|| (until - now).ceil() as u64)
    }

    /// Until when code entry is paused for everyone, if it is.
    pub fn paused_until(&self, now: f64) -> Option<f64> {
        (self.paused_until > now).then_some(self.paused_until)
    }

    pub fn failed(&mut self, address: &str, now: f64) {
        let entry = self.addresses.entry(address.to_owned()).or_insert((0, 0.0));
        entry.0 += 1;
        if entry.0 > FREE_TRIES_PER_ADDRESS {
            let doubling = (entry.0 - FREE_TRIES_PER_ADDRESS - 1).min(10);
            entry.1 = now + f64::from(1u32 << doubling).min(MAX_ADDRESS_DELAY);
        }
        self.recent.push_back(now);
        while self.recent.front().is_some_and(|&at| now - at > GLOBAL_WINDOW) {
            self.recent.pop_front();
        }
        if self.recent.len() >= GLOBAL_FAILURES {
            self.paused_until = now + GLOBAL_PAUSE;
            self.recent.clear();
        }
    }

    pub fn succeeded(&mut self, address: &str) {
        self.addresses.remove(address);
    }

    /// A new code starts a clean slate.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_waits_longer_after_each_failure() {
        let mut limiter = CodeLimiter::default();
        limiter.failed("a", 0.0);
        limiter.failed("a", 0.0);
        assert_eq!(limiter.wait("a", 0.0), None, "the first tries are free");
        let mut waits = Vec::new();
        for _ in 0..8 {
            limiter.failed("a", 0.0);
            waits.push(limiter.wait("a", 0.0).unwrap());
        }
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(limiter.wait("b", 0.0), None, "other addresses are unaffected");
        assert_eq!(limiter.wait("a", 61.0), None, "the pause ends");
        limiter.succeeded("a");
        assert_eq!(limiter.wait("a", 0.0), None);
    }

    #[test]
    fn many_failures_from_many_addresses_pause_everyone() {
        let mut limiter = CodeLimiter::default();
        for i in 0..GLOBAL_FAILURES {
            assert_eq!(limiter.paused_until(i as f64), None);
            limiter.failed(&format!("addr{i}"), i as f64);
        }
        let now = GLOBAL_FAILURES as f64;
        assert!(limiter.paused_until(now).is_some());
        assert!(limiter.wait("never-tried", now).is_some());
        assert_eq!(limiter.wait("never-tried", now + GLOBAL_PAUSE + 1.0), None);
        // Failures spread over a long time never add up.
        let mut slow = CodeLimiter::default();
        for i in 0..100 {
            slow.failed(&format!("x{i}"), f64::from(i) * 10.0);
        }
        assert_eq!(slow.paused_until(1000.0), None);
    }
}
