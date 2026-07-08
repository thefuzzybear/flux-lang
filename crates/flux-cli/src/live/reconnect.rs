//! Reconnection logic with exponential backoff.
//!
//! Implements the backoff formula `min(initial × multiplier^attempt, max)`
//! with ±10% jitter, and a reconnection loop that retries up to a
//! configurable maximum number of attempts.

use std::time::Duration;
use tokio::sync::mpsc;

use crate::live::connector::{Connector, ConnectorError, LiveBar, ReconnectPolicy};

/// Calculate the next backoff duration using exponential backoff.
///
/// Formula: `min(initial_backoff_ms * multiplier^attempt, max_backoff_ms)`
/// with ±10% jitter applied to prevent thundering herd problems.
///
/// The jitter is uniformly distributed in the range `[-10%, +10%]` of the
/// capped backoff value. The result is always non-negative.
pub fn next_backoff(policy: &ReconnectPolicy, attempt: u32) -> Duration {
    let base_ms = (policy.initial_backoff_ms as f64) * policy.multiplier.powi(attempt as i32);
    let capped_ms = base_ms.min(policy.max_backoff_ms as f64);

    // Add ±10% jitter
    let jitter_range = capped_ms * 0.1;
    let jitter = rand::random::<f64>() * jitter_range * 2.0 - jitter_range;
    let final_ms = (capped_ms + jitter).max(0.0) as u64;

    Duration::from_millis(final_ms)
}

/// Manage reconnection for a single connector.
///
/// Retries up to `policy.max_attempts` times with exponential backoff
/// between attempts. Logs each attempt with the connector id, attempt
/// number, and backoff duration to stderr.
///
/// Returns `Ok(())` if the connector successfully reconnects, or
/// `ConnectorError::ConnectionFailed` if all attempts are exhausted.
pub async fn reconnect_loop(
    connector: &mut dyn Connector,
    symbols: &[String],
    tx: mpsc::Sender<LiveBar>,
    policy: &ReconnectPolicy,
) -> Result<(), ConnectorError> {
    for attempt in 0..policy.max_attempts {
        let backoff = next_backoff(policy, attempt);
        eprintln!(
            "  [{}] reconnecting (attempt {}/{}, backoff {:.1}s)...",
            connector.id(),
            attempt + 1,
            policy.max_attempts,
            backoff.as_secs_f64()
        );

        tokio::time::sleep(backoff).await;

        match connector.connect(symbols, tx.clone()).await {
            Ok(()) => {
                eprintln!("  [{}] reconnected successfully", connector.id());
                return Ok(());
            }
            Err(e) => {
                eprintln!("  [{}] reconnection failed: {}", connector.id(), e);
            }
        }
    }

    Err(ConnectorError::ConnectionFailed(format!(
        "max reconnection attempts ({}) exceeded",
        policy.max_attempts
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a policy with known values for testing.
    fn test_policy() -> ReconnectPolicy {
        ReconnectPolicy {
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
            max_attempts: 10,
            multiplier: 2.0,
        }
    }

    #[test]
    fn next_backoff_is_within_expected_range_for_first_attempt() {
        let policy = test_policy();
        // attempt 0: base = 1000 * 2^0 = 1000ms, jitter ±10% → [900, 1100]
        for _ in 0..100 {
            let d = next_backoff(&policy, 0);
            let ms = d.as_millis() as u64;
            assert!(
                ms >= 900 && ms <= 1100,
                "attempt 0 backoff {} not in [900, 1100]",
                ms
            );
        }
    }

    #[test]
    fn next_backoff_grows_exponentially() {
        let policy = test_policy();
        // attempt 3: base = 1000 * 2^3 = 8000ms, jitter ±10% → [7200, 8800]
        for _ in 0..100 {
            let d = next_backoff(&policy, 3);
            let ms = d.as_millis() as u64;
            assert!(
                ms >= 7200 && ms <= 8800,
                "attempt 3 backoff {} not in [7200, 8800]",
                ms
            );
        }
    }

    #[test]
    fn next_backoff_caps_at_max() {
        let policy = test_policy();
        // attempt 10: base = 1000 * 2^10 = 1024000 → capped to 60000, jitter ±10% → [54000, 66000]
        for _ in 0..100 {
            let d = next_backoff(&policy, 10);
            let ms = d.as_millis() as u64;
            assert!(
                ms >= 54000 && ms <= 66000,
                "attempt 10 backoff {} not in [54000, 66000]",
                ms
            );
        }
    }

    #[test]
    fn next_backoff_is_non_negative() {
        let policy = ReconnectPolicy {
            initial_backoff_ms: 1,
            max_backoff_ms: 10,
            max_attempts: 5,
            multiplier: 1.0,
        };
        for attempt in 0..20 {
            let d = next_backoff(&policy, attempt);
            // Duration is inherently non-negative; verify the math didn't panic
            let _ = d.as_millis();
        }
    }

    #[test]
    fn next_backoff_with_zero_initial_returns_zero() {
        let policy = ReconnectPolicy {
            initial_backoff_ms: 0,
            max_backoff_ms: 60_000,
            max_attempts: 10,
            multiplier: 2.0,
        };
        let d = next_backoff(&policy, 5);
        assert_eq!(d.as_millis(), 0);
    }

    #[test]
    fn next_backoff_respects_custom_multiplier() {
        let policy = ReconnectPolicy {
            initial_backoff_ms: 100,
            max_backoff_ms: 100_000,
            max_attempts: 10,
            multiplier: 3.0,
        };
        // attempt 2: base = 100 * 3^2 = 900ms, jitter ±10% → [810, 990]
        for _ in 0..100 {
            let d = next_backoff(&policy, 2);
            let ms = d.as_millis() as u64;
            assert!(
                ms >= 810 && ms <= 990,
                "custom multiplier attempt 2 backoff {} not in [810, 990]",
                ms
            );
        }
    }
}
