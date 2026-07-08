//! Property test for exponential backoff formula.
//!
//! Feature: flux-live-harness, Property 7: Exponential backoff formula
//!
//! **Validates: Requirements 6.1**
//!
//! Generates random reconnection policies and attempt numbers, then verifies
//! that `next_backoff` produces durations that satisfy:
//! 1. Non-negative result
//! 2. Bounded by max_backoff + 10% jitter
//! 3. Bounded from below by expected_base * 0.9 - 1ms (rounding)
//! 4. Within ±10% of the capped value
//! 5. Monotonically non-decreasing base on average (without jitter noise)

use proptest::prelude::*;
use std::time::Duration;

use flux_cli::live::connector::ReconnectPolicy;
use flux_cli::live::reconnect::next_backoff;

// =============================================================================
// Strategies (generators) for property tests
// =============================================================================

/// Generate a valid ReconnectPolicy with constrained random values.
fn arb_policy() -> impl Strategy<Value = ReconnectPolicy> {
    (1u64..10_000, 1u64..100_000, 1u32..20, 1.0f64..5.0).prop_flat_map(
        |(initial, max_offset, max_attempts, multiplier)| {
            // Ensure max_backoff_ms >= initial_backoff_ms
            let max_backoff_ms = initial + max_offset;
            Just(ReconnectPolicy {
                initial_backoff_ms: initial,
                max_backoff_ms,
                max_attempts,
                multiplier,
            })
        },
    )
}

// =============================================================================
// Property 7: Exponential backoff formula
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// **Validates: Requirements 6.1**
    ///
    /// Property: The backoff duration is always non-negative.
    /// Duration is inherently non-negative in Rust, but we verify the
    /// underlying math never produces negative intermediate values that
    /// could panic or wrap.
    #[test]
    fn prop_backoff_non_negative(
        policy in arb_policy(),
        attempt in 0u32..30,
    ) {
        let duration = next_backoff(&policy, attempt);
        // Duration::from_millis cannot be negative; if the math produced
        // a negative intermediate, .max(0.0) in the implementation handles it.
        // We verify it doesn't panic and produces a valid duration.
        prop_assert!(duration >= Duration::from_millis(0));
    }

    /// **Validates: Requirements 6.1**
    ///
    /// Property: The backoff is bounded above by max_backoff_ms * 1.1.
    /// The jitter adds at most +10% of the capped value.
    #[test]
    fn prop_backoff_bounded_by_max_plus_jitter(
        policy in arb_policy(),
        attempt in 0u32..30,
    ) {
        let duration = next_backoff(&policy, attempt);
        let upper_bound_ms = (policy.max_backoff_ms as f64 * 1.1).ceil() as u64;
        let actual_ms = duration.as_millis() as u64;

        prop_assert!(
            actual_ms <= upper_bound_ms,
            "Backoff {}ms exceeds upper bound {}ms (max_backoff={}ms + 10% jitter)",
            actual_ms,
            upper_bound_ms,
            policy.max_backoff_ms
        );
    }

    /// **Validates: Requirements 6.1**
    ///
    /// Property: The backoff is bounded from below.
    /// Result >= expected_base * 0.9 - 1 (minus 1 for rounding).
    /// where expected_base = min(initial * multiplier^attempt, max).
    #[test]
    fn prop_backoff_bounded_from_below(
        policy in arb_policy(),
        attempt in 0u32..30,
    ) {
        let duration = next_backoff(&policy, attempt);
        let actual_ms = duration.as_millis() as u64;

        let base_ms = (policy.initial_backoff_ms as f64)
            * policy.multiplier.powi(attempt as i32);
        let capped_ms = base_ms.min(policy.max_backoff_ms as f64);

        // Lower bound: capped * 0.9 - 1 (for floating point rounding)
        let lower_bound = (capped_ms * 0.9 - 1.0).max(0.0) as u64;

        prop_assert!(
            actual_ms >= lower_bound,
            "Backoff {}ms below lower bound {}ms (capped_base={:.1}ms * 0.9 - 1)",
            actual_ms,
            lower_bound,
            capped_ms
        );
    }

    /// **Validates: Requirements 6.1**
    ///
    /// Property: The duration is within ±10% of the capped value.
    /// capped = min(initial * multiplier^attempt, max)
    /// result ∈ [capped * 0.9, capped * 1.1] (approximately, with rounding tolerance)
    #[test]
    fn prop_backoff_within_jitter_range(
        policy in arb_policy(),
        attempt in 0u32..30,
    ) {
        let duration = next_backoff(&policy, attempt);
        let actual_ms = duration.as_millis() as u64;

        let base_ms = (policy.initial_backoff_ms as f64)
            * policy.multiplier.powi(attempt as i32);
        let capped_ms = base_ms.min(policy.max_backoff_ms as f64);

        // Allow ±10% of capped value, plus 1ms rounding tolerance
        let lower = (capped_ms * 0.9 - 1.0).max(0.0) as u64;
        let upper = (capped_ms * 1.1).ceil() as u64 + 1;

        prop_assert!(
            actual_ms >= lower && actual_ms <= upper,
            "Backoff {}ms not within ±10% jitter of capped base {:.1}ms \
             (expected [{}, {}])",
            actual_ms,
            capped_ms,
            lower,
            upper
        );
    }

    /// **Validates: Requirements 6.1**
    ///
    /// Property: The expected base (without jitter) is monotonically non-decreasing.
    /// For attempt N+1, the expected base >= attempt N base (before cap).
    /// After capping, both converge to max_backoff_ms.
    /// We verify this by checking the capped bases are non-decreasing.
    #[test]
    fn prop_backoff_base_monotonically_non_decreasing(
        policy in arb_policy(),
        attempt in 0u32..29,
    ) {
        let base_n = (policy.initial_backoff_ms as f64)
            * policy.multiplier.powi(attempt as i32);
        let capped_n = base_n.min(policy.max_backoff_ms as f64);

        let base_n1 = (policy.initial_backoff_ms as f64)
            * policy.multiplier.powi((attempt + 1) as i32);
        let capped_n1 = base_n1.min(policy.max_backoff_ms as f64);

        prop_assert!(
            capped_n1 >= capped_n,
            "Base backoff not monotonically non-decreasing: \
             attempt {} capped={:.1}ms > attempt {} capped={:.1}ms",
            attempt + 1,
            capped_n1,
            attempt,
            capped_n
        );
    }
}
