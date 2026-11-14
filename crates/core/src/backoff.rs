//! Per-provider exponential backoff: a failing provider is retried after
//! base, 2x, 4x ... up to [`MAX_BACKOFF_SECS`]; one success resets it.

pub const MAX_BACKOFF_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backoff {
    failures: u32,
    next_due: i64,
}

impl Backoff {
    pub fn is_due(&self, now: i64) -> bool {
        now >= self.next_due
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Unix time from which this provider may be fetched again.
    pub fn next_due(&self) -> i64 {
        self.next_due
    }

    /// Delay after `failures` consecutive failures.
    pub fn delay(base_secs: u64, failures: u32) -> u64 {
        let base = base_secs.max(1);
        let factor = 1u64.checked_shl(failures.min(20)).unwrap_or(u64::MAX);
        base.saturating_mul(factor).min(MAX_BACKOFF_SECS.max(base))
    }

    pub fn record(&mut self, ok: bool, base_secs: u64, now: i64) {
        self.failures = if ok { 0 } else { self.failures.saturating_add(1) };
        self.next_due = now + Self::delay(base_secs, self.failures) as i64;
    }

    /// Makes the provider due immediately (manual refresh).
    pub fn reset_due(&mut self) {
        self.next_due = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_and_caps() {
        assert_eq!(Backoff::delay(180, 0), 180);
        assert_eq!(Backoff::delay(180, 1), 360);
        assert_eq!(Backoff::delay(180, 3), 1440);
        assert_eq!(Backoff::delay(180, 4), MAX_BACKOFF_SECS);
        assert_eq!(Backoff::delay(180, 99), MAX_BACKOFF_SECS);
        // A base above the cap is respected.
        assert_eq!(Backoff::delay(3600, 2), 3600);
    }

    #[test]
    fn success_resets() {
        let mut b = Backoff::default();
        assert!(b.is_due(0));
        b.record(false, 180, 0);
        b.record(false, 180, 360);
        assert_eq!(b.failures(), 2);
        assert!(!b.is_due(360 + 719));
        assert!(b.is_due(360 + 720));
        b.record(true, 180, 1000);
        assert_eq!(b.failures(), 0);
        assert!(b.is_due(1180));
        b.reset_due();
        assert!(b.is_due(0));
    }
}
