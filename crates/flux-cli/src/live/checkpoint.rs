//! Dual-trigger checkpoint scheduler (bar count + wall clock).
//!
//! The `CheckpointScheduler` tracks two independent triggers for when to persist
//! harness state to disk — whichever fires first wins:
//!
//! 1. **Bar-count trigger**: After every `bar_interval` bars (default 50).
//! 2. **Wall-clock trigger**: After `time_interval` elapses (default 5 minutes).
//!
//! The caller is responsible for actually writing the checkpoint; the scheduler
//! only decides *when* to trigger. After a successful save, the caller calls
//! `mark_checkpointed()` to reset counters.

use std::time::{Duration, Instant};

/// Dual-trigger checkpoint scheduler (bar count + wall clock).
pub struct CheckpointScheduler {
    /// Bars since last checkpoint.
    bars_since_checkpoint: u64,
    /// Bar interval trigger threshold.
    bar_interval: u64,
    /// Wall-clock interval trigger threshold.
    time_interval: Duration,
    /// Instant of last checkpoint.
    last_checkpoint_time: Instant,
    /// Total bars processed this session (for state metadata).
    total_bars: u64,
}

impl CheckpointScheduler {
    /// Create a new scheduler with the given bar interval and time interval.
    pub fn new(bar_interval: u64, time_interval: Duration) -> Self {
        Self {
            bars_since_checkpoint: 0,
            bar_interval,
            time_interval,
            last_checkpoint_time: Instant::now(),
            total_bars: 0,
        }
    }

    /// Called after each bar dispatch. Increments counters and returns true
    /// if a checkpoint should fire based on bar count.
    ///
    /// Note: This does NOT reset counters — the caller should call
    /// `mark_checkpointed()` after a successful save.
    pub fn on_bar(&mut self) -> bool {
        self.bars_since_checkpoint += 1;
        self.total_bars += 1;
        self.bars_since_checkpoint >= self.bar_interval
    }

    /// Check wall-clock trigger. Returns true if time elapsed since last
    /// checkpoint exceeds the configured threshold.
    pub fn should_checkpoint_time(&self) -> bool {
        self.last_checkpoint_time.elapsed() >= self.time_interval
    }

    /// Reset counters after a successful checkpoint.
    pub fn mark_checkpointed(&mut self) {
        self.bars_since_checkpoint = 0;
        self.last_checkpoint_time = Instant::now();
    }

    /// Total bars processed this session (for state metadata).
    pub fn total_bars(&self) -> u64 {
        self.total_bars
    }
}

impl Default for CheckpointScheduler {
    fn default() -> Self {
        Self::new(50, Duration::from_secs(300))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_at_bar_interval() {
        let mut sched = CheckpointScheduler::new(3, Duration::from_secs(600));
        assert!(!sched.on_bar()); // bar 1
        assert!(!sched.on_bar()); // bar 2
        assert!(sched.on_bar()); // bar 3 — triggers
    }

    #[test]
    fn mark_checkpointed_resets_bar_counter() {
        let mut sched = CheckpointScheduler::new(2, Duration::from_secs(600));
        assert!(!sched.on_bar()); // bar 1
        assert!(sched.on_bar()); // bar 2 — triggers
        sched.mark_checkpointed();
        assert!(!sched.on_bar()); // bar 1 after reset
        assert!(sched.on_bar()); // bar 2 after reset — triggers again
    }

    #[test]
    fn total_bars_accumulates_across_checkpoints() {
        let mut sched = CheckpointScheduler::new(2, Duration::from_secs(600));
        sched.on_bar();
        sched.on_bar();
        sched.mark_checkpointed();
        sched.on_bar();
        assert_eq!(sched.total_bars(), 3);
    }

    #[test]
    fn default_uses_50_bars_and_300s() {
        let sched = CheckpointScheduler::default();
        assert_eq!(sched.bar_interval, 50);
        assert_eq!(sched.time_interval, Duration::from_secs(300));
    }

    #[test]
    fn should_checkpoint_time_false_initially() {
        let sched = CheckpointScheduler::new(50, Duration::from_secs(300));
        // Just created — elapsed time should be near zero.
        assert!(!sched.should_checkpoint_time());
    }

    #[test]
    fn should_checkpoint_time_true_after_elapsed() {
        let mut sched = CheckpointScheduler::new(50, Duration::from_secs(0));
        // With 0-second interval, should trigger immediately on next check.
        // We need at least a tiny bit of time to pass.
        std::thread::sleep(Duration::from_millis(1));
        assert!(sched.should_checkpoint_time());

        // After marking checkpointed with a long interval, should not trigger.
        sched.time_interval = Duration::from_secs(600);
        sched.mark_checkpointed();
        assert!(!sched.should_checkpoint_time());
    }

    #[test]
    fn on_bar_continues_triggering_without_reset() {
        let mut sched = CheckpointScheduler::new(2, Duration::from_secs(600));
        sched.on_bar();
        assert!(sched.on_bar()); // triggers
        // Without mark_checkpointed, continues triggering
        assert!(sched.on_bar());
        assert!(sched.on_bar());
    }
}
