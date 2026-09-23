//! `Clock` trait, `SystemClock` and `FixedClock`, per `docs/PHASE1_SPEC.md` §4.
//!
//! Every service that needs "now" takes a `Arc<dyn Clock>` and calls `clock.now()` rather
//! than `Utc::now()` directly, so tests can control time (see `CLAUDE.md` "Time").

use std::sync::Mutex;

use chrono::{DateTime, Utc};

/// Source of the current instant. Implementations must be `Send + Sync` so they can be
/// shared behind an `Arc` across async tasks.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Reads the real wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock tests can set and advance, so time-dependent logic (logical dates, the
/// scheduler's "is it time" check, ...) is deterministic.
#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<DateTime<Utc>>,
}

impl FixedClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    /// Sets the clock to an absolute instant.
    pub fn set(&self, now: DateTime<Utc>) {
        // A poisoned mutex would mean an earlier panic while holding the lock; there is no
        // sane recovery, so propagate rather than silently keep the stale value.
        *self.now.lock().expect("FixedClock mutex poisoned") = now;
    }

    /// Advances the clock by a duration (may be negative to move it back).
    pub fn advance(&self, delta: chrono::Duration) {
        let mut guard = self.now.lock().expect("FixedClock mutex poisoned");
        *guard += delta;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("FixedClock mutex poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn fixed_clock_returns_set_value() {
        let t = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let clock = FixedClock::new(t);
        assert_eq!(clock.now(), t);
    }

    #[test]
    fn fixed_clock_set_overrides_value() {
        let t0 = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 9, 24, 8, 30, 0).unwrap();
        let clock = FixedClock::new(t0);
        clock.set(t1);
        assert_eq!(clock.now(), t1);
    }

    #[test]
    fn fixed_clock_advance_moves_time_forward_and_backward() {
        let t = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let clock = FixedClock::new(t);
        clock.advance(chrono::Duration::hours(2));
        assert_eq!(clock.now(), t + chrono::Duration::hours(2));
        clock.advance(chrono::Duration::hours(-5));
        assert_eq!(clock.now(), t + chrono::Duration::hours(-3));
    }

    #[test]
    fn system_clock_now_is_close_to_wall_clock() {
        let before = Utc::now();
        let got = SystemClock.now();
        let after = Utc::now();
        assert!(before <= got && got <= after);
    }
}
