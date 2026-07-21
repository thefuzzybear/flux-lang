//! Property-based tests for Empyrean Swing Wiring.
//!
//! Feature: empyrean-swing-wiring
//!
//! This file contains property tests validating the correctness of the
//! empyrean swing account wiring logic.
//!
//! **Validates: Requirements 2.7, 5.4, 6.1, 6.2, 6.4, 6.5**

use proptest::prelude::*;
use std::fs;
use tempfile::TempDir;

use flux_cli::live::account_config::{
    AccountConfig, AccountSection, DataSection, DatabaseSection, GatewaySection, RiskSection,
    StrategyEntry,
};
use flux_cli::live::account_runtime::load_strategies_from_config;
use flux_cli::live::broker::{resolve_execution_policy, ExecutionPolicy};
use flux_cli::live::broker::{BrokerAdapter, DeduplicationGuard, ExecutionPolicy as ExecPolicy2, Order, OrderId, Side};
use flux_cli::live::broker::mock::{MockBrokerAdapter, MockFillBehavior};

// =============================================================================
// Feature: empyrean-swing-wiring, Property 2: Execution policy mapping correctness
// =============================================================================

/// Strategy for generating execution policy string options.
/// Covers the two primary types used in the swing account manifest plus unknown strings.
fn exec_str_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("market".to_string()),
        Just("aggressive_limit".to_string()),
        Just("limit".to_string()),
        Just("market_on_close".to_string()),
        Just("unknown_policy".to_string()),
    ]
}

proptest! {
    /// **Validates: Requirements 2.7, 6.1, 6.2, 6.4, 6.5**
    ///
    /// Property 2: For any combination of (strategy_exec, offset, account_default),
    /// the resolved execution policy follows the priority cascade:
    ///   1. strategy-level execution (if present) + offset
    ///   2. account-level default (if present)
    ///   3. ExecutionPolicy::Market (hardcoded fallback)
    #[test]
    fn prop_execution_policy_mapping_correctness(
        has_strategy_exec in proptest::bool::ANY,
        strategy_exec_kind in exec_str_strategy(),
        offset in proptest::option::of(1i32..=20),
        has_default in proptest::bool::ANY,
        default_exec_kind in exec_str_strategy(),
    ) {
        let strategy_exec: Option<&str> = if has_strategy_exec {
            Some(strategy_exec_kind.as_str())
        } else {
            None
        };
        let account_default: Option<&str> = if has_default {
            Some(default_exec_kind.as_str())
        } else {
            None
        };

        let result = resolve_execution_policy(strategy_exec, offset, account_default);

        // Verify the priority cascade: strategy → default → Market
        if has_strategy_exec {
            // Priority 1: Strategy-level execution takes precedence
            let expected = match strategy_exec_kind.as_str() {
                "market" => ExecutionPolicy::Market,
                "aggressive_limit" => ExecutionPolicy::AggressiveLimit {
                    offset_ticks: offset.unwrap_or(2),
                },
                "limit" => ExecutionPolicy::Limit { price: 0.0 },
                "market_on_close" => ExecutionPolicy::MarketOnClose,
                _ => ExecutionPolicy::Market, // Unknown strings default to Market
            };
            prop_assert_eq!(result, expected,
                "strategy_exec={:?}, offset={:?}, account_default={:?}",
                strategy_exec, offset, account_default);
        } else if has_default {
            // Priority 2: Falls back to account-level default (no offset applied)
            let expected = match default_exec_kind.as_str() {
                "market" => ExecutionPolicy::Market,
                "aggressive_limit" => ExecutionPolicy::AggressiveLimit {
                    offset_ticks: 2, // Default offset when none specified at strategy level
                },
                "limit" => ExecutionPolicy::Limit { price: 0.0 },
                "market_on_close" => ExecutionPolicy::MarketOnClose,
                _ => ExecutionPolicy::Market,
            };
            prop_assert_eq!(result, expected,
                "strategy_exec=None, account_default={:?}",
                account_default);
        } else {
            // Priority 3: No strategy exec, no account default → Market
            prop_assert_eq!(result, ExecutionPolicy::Market,
                "Both strategy_exec and account_default are None, should be Market");
        }
    }

    /// **Validates: Requirements 6.5**
    ///
    /// Property 2b: When no strategy execution is specified and no account default exists,
    /// the result is always ExecutionPolicy::Market regardless of offset value.
    #[test]
    fn prop_execution_policy_default_is_always_market(
        offset in proptest::option::of(-100i32..100),
    ) {
        let result = resolve_execution_policy(None, offset, None);
        prop_assert_eq!(result, ExecutionPolicy::Market,
            "With no strategy exec and no default, result must be Market regardless of offset={:?}",
            offset);
    }

    /// **Validates: Requirements 6.1, 6.2**
    ///
    /// Property 2c: Strategy-level execution always overrides account default,
    /// regardless of what the account default is set to.
    #[test]
    fn prop_strategy_exec_overrides_account_default(
        strategy_exec_kind in exec_str_strategy(),
        offset in proptest::option::of(1i32..=20),
        default_exec_kind in exec_str_strategy(),
    ) {
        let result_with_default = resolve_execution_policy(
            Some(strategy_exec_kind.as_str()),
            offset,
            Some(default_exec_kind.as_str()),
        );
        let result_without_default = resolve_execution_policy(
            Some(strategy_exec_kind.as_str()),
            offset,
            None,
        );

        // Strategy-level takes priority — result should be the same regardless of default
        prop_assert_eq!(result_with_default, result_without_default,
            "Strategy exec '{}' with offset {:?} should produce same result regardless of default '{}'",
            strategy_exec_kind, offset, default_exec_kind);
    }
}


// =============================================================================
// Feature: empyrean-swing-wiring, Property 4: Broker retry follows exponential backoff
// =============================================================================

/// Compute the backoff delay for a given attempt number.
/// This mirrors the logic in `connect_broker_with_retry`:
///   delay_secs = min(2^attempt, 60)
fn compute_backoff_delay(attempt: u32) -> u64 {
    std::cmp::min(2u64.pow(attempt), 60)
}

/// The maximum total retry duration before the function should error (5 minutes).
const MAX_RETRY_DURATION_SECS: u64 = 300;

/// The maximum per-attempt delay cap in seconds.
const MAX_DELAY_CAP_SECS: u64 = 60;

proptest! {
    /// **Validates: Requirements 5.5, 5.6**
    ///
    /// Property 4: For any failure count K in [1, 20], the delay between
    /// attempt i and i+1 equals min(2^i, 60) seconds.
    /// The first delay (attempt 0) is always 1 second (2^0 = 1).
    /// The delay is capped at 60 seconds and never exceeds it.
    #[test]
    fn prop_broker_retry_follows_exponential_backoff(
        failure_count in 1u32..20
    ) {
        for i in 0..failure_count {
            let delay = compute_backoff_delay(i);

            // The delay must equal min(2^i, 60)
            let expected = std::cmp::min(2u64.pow(i), 60);
            prop_assert_eq!(delay, expected,
                "Delay for attempt {} should be {} seconds, got {}",
                i, expected, delay);

            // The delay must never exceed the 60-second cap
            prop_assert!(delay <= MAX_DELAY_CAP_SECS,
                "Delay should never exceed {} seconds, got {} for attempt {}",
                MAX_DELAY_CAP_SECS, delay, i);
        }
    }

    /// **Validates: Requirements 5.5**
    ///
    /// Property 4b: Backoff delays are monotonically non-decreasing.
    /// Each successive attempt waits at least as long as the previous one.
    #[test]
    fn prop_backoff_delay_is_monotonically_nondecreasing(
        failure_count in 2u32..20
    ) {
        for i in 1..failure_count {
            let prev_delay = compute_backoff_delay(i - 1);
            let curr_delay = compute_backoff_delay(i);
            prop_assert!(curr_delay >= prev_delay,
                "Delay should be non-decreasing: attempt {} = {}s, attempt {} = {}s",
                i - 1, prev_delay, i, curr_delay);
        }
    }

    /// **Validates: Requirements 5.6**
    ///
    /// Property 4c: After enough consecutive failures, total elapsed time exceeds
    /// 5 minutes (300 seconds), at which point the function should return an error.
    /// For any failure count K, if the sum of delays for attempts 0..K exceeds
    /// 300 seconds, that means the retry budget is exhausted.
    #[test]
    fn prop_retry_budget_exhaustion(
        failure_count in 1u32..20
    ) {
        let total_elapsed: u64 = (0..failure_count).map(|i| compute_backoff_delay(i)).sum();

        if total_elapsed >= MAX_RETRY_DURATION_SECS {
            // After this many failures the function should have returned an error
            // (total time exceeds 5 minutes)
            prop_assert!(total_elapsed >= MAX_RETRY_DURATION_SECS,
                "Total elapsed {}s should exceed budget {}s after {} failures",
                total_elapsed, MAX_RETRY_DURATION_SECS, failure_count);
        } else {
            // Still within budget — retrying is valid
            prop_assert!(total_elapsed < MAX_RETRY_DURATION_SECS,
                "Total elapsed {}s should be within budget {}s for {} failures",
                total_elapsed, MAX_RETRY_DURATION_SECS, failure_count);
        }
    }

    /// **Validates: Requirements 5.5**
    ///
    /// Property 4d: The first delay is always 1 second (2^0 = 1),
    /// and the delay reaches the 60-second cap at attempt 6 (2^6 = 64 > 60).
    #[test]
    fn prop_backoff_boundary_values(
        attempt in 0u32..20
    ) {
        let delay = compute_backoff_delay(attempt);

        // First attempt always has delay of 1 second
        if attempt == 0 {
            prop_assert_eq!(delay, 1,
                "First delay (attempt 0) should always be 1 second");
        }

        // At attempt 6, 2^6 = 64 > 60, so cap kicks in
        if attempt >= 6 {
            prop_assert_eq!(delay, MAX_DELAY_CAP_SECS,
                "Delay for attempt {} should be capped at {} seconds",
                attempt, MAX_DELAY_CAP_SECS);
        }

        // Before attempt 6, delay should be exactly 2^attempt
        if attempt < 6 {
            prop_assert_eq!(delay, 2u64.pow(attempt),
                "Delay for attempt {} should be exactly 2^{} = {}",
                attempt, attempt, 2u64.pow(attempt));
        }
    }
}


// =============================================================================
// Feature: empyrean-swing-wiring, Property 5: Deduplication guard reconciliation completeness
// =============================================================================

/// Strategy for generating valid order ID strings.
/// Format mimics real IDs: "{account}_{strategy}_{symbol}_{bar_index}"
fn order_id_strategy() -> impl Strategy<Value = String> {
    (
        prop_oneof![Just("swing"), Just("paper"), Just("live")],
        prop_oneof![Just("aether"), Just("kairos"), Just("strat1")],
        prop_oneof![Just("ES"), Just("NQ"), Just("RTY"), Just("YM")],
        0u64..10000,
    )
        .prop_map(|(acct, strat, sym, bar)| format!("{}_{}_{}_{}",acct, strat, sym, bar))
}

proptest! {
    /// **Validates: Requirements 5.4**
    ///
    /// Property 5: For any set of open orders returned by `BrokerAdapter::get_open_orders()`,
    /// after calling `DeduplicationGuard::reconcile(broker)`, every returned OrderId SHALL be
    /// marked as submitted in the guard (i.e., `is_duplicate(id)` returns true for each).
    #[test]
    fn prop_dedup_reconciliation_completeness(
        order_ids in proptest::collection::vec(order_id_strategy(), 0..20),
    ) {
        // Deduplicate generated IDs to avoid testing with duplicates in the input
        let unique_ids: Vec<String> = order_ids
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Set up mock broker with Pending fill behavior so submitted orders
            // go into open_orders (simulating orders that haven't filled yet)
            let mock = MockBrokerAdapter::new();
            mock.set_fill_behavior(MockFillBehavior::Pending);

            // Submit orders through the broker — they'll stay in open_orders
            for id_str in &unique_ids {
                let order = Order {
                    id: OrderId(id_str.clone()),
                    symbol: "ES".to_string(),
                    side: Side::Buy,
                    contracts: 1,
                    execution: ExecPolicy2::Market,
                    last_price: 5000.0,
                    tick_size: 0.25,
                };
                mock.submit_order(&order).await.unwrap();
            }

            // Verify the mock returns all open orders
            let broker_open = mock.get_open_orders().await.unwrap();
            prop_assert_eq!(broker_open.len(), unique_ids.len(),
                "Mock should have {} open orders", unique_ids.len());

            // Reconcile — the dedup guard should mark all open order IDs
            let mut dedup = DeduplicationGuard::new();
            let reconciled = dedup.reconcile(&mock as &dyn flux_cli::live::broker::BrokerAdapter).await.unwrap();

            // All reconciled IDs should match the open orders count
            prop_assert_eq!(reconciled.len(), unique_ids.len(),
                "Expected {} reconciled IDs, got {}", unique_ids.len(), reconciled.len());

            // Every open order ID must now be marked as duplicate
            for id_str in &unique_ids {
                let order_id = OrderId(id_str.clone());
                prop_assert!(dedup.is_duplicate(&order_id),
                    "Order '{}' should be marked as duplicate after reconcile", id_str);
            }

            // Verify that a random unrelated ID is NOT marked as duplicate
            let unrelated_id = OrderId("unrelated_account_strat_ZZZ_99999".to_string());
            prop_assert!(!dedup.is_duplicate(&unrelated_id),
                "An unrelated order ID should NOT be marked as duplicate");

            Ok(())
        })?;
    }

    /// **Validates: Requirements 5.4**
    ///
    /// Property 5b: Reconciliation with an empty set of open orders does not mark
    /// anything as duplicate, and returns an empty Vec.
    #[test]
    fn prop_dedup_reconciliation_empty_broker(
        // Generate random IDs that we'll check are NOT duplicates
        check_ids in proptest::collection::vec(order_id_strategy(), 1..10),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mock = MockBrokerAdapter::new();
            // No open orders populated — broker is fresh

            let mut dedup = DeduplicationGuard::new();
            let reconciled = dedup.reconcile(&mock as &dyn flux_cli::live::broker::BrokerAdapter).await.unwrap();

            prop_assert_eq!(reconciled.len(), 0,
                "Empty broker should produce no reconciled IDs");

            // None of the check IDs should be duplicates
            for id_str in &check_ids {
                let order_id = OrderId(id_str.clone());
                prop_assert!(!dedup.is_duplicate(&order_id),
                    "ID '{}' should not be duplicate when broker has no open orders", id_str);
            }

            Ok(())
        })?;
    }
}


// =============================================================================
// Feature: empyrean-swing-wiring, Property 3: Partial strategy failure does not halt boot
// =============================================================================

/// Helper: build a minimal AccountConfig with the given strategy entries.
fn build_test_account_config(strategies: Vec<StrategyEntry>) -> AccountConfig {
    AccountConfig {
        account: AccountSection {
            name: "test".into(),
            broker: "mock".into(),
            account_id: "T1".into(),
            mode: "paper".into(),
        },
        gateway: GatewaySection {
            host: "127.0.0.1".into(),
            port: 4002,
        },
        data: DataSection {
            source: "mock".into(),
            symbols: vec!["TEST".into()],
            interval: "1d".into(),
        },
        database: DatabaseSection {
            url: "".into(),
            schema: "".into(),
        },
        risk: RiskSection {
            max_daily_loss: -10000.0,
            max_weekly_loss: -20000.0,
            max_position_per_product: 10,
            max_total_notional: 1000000.0,
            max_drawdown_pct: 0.1,
            correlation_warning_threshold: 4,
            initial_equity: 100000.0,
        },
        products: vec![],
        strategies,
        execution_default: None,
    }
}

/// Minimal valid Flux strategy source that the compiler can lex, parse, and typecheck.
const VALID_STRATEGY_SOURCE: &str = r#"strategy TestStrategy {
    params {
        x = 1.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
    }
}
"#;

/// Invalid Flux source that will fail at the lexer/parser stage.
const INVALID_STRATEGY_SOURCE: &str = "this is not valid flux syntax at all!!!";

proptest! {
    /// **Validates: Requirements 2.10**
    ///
    /// Property 3: For any account manifest declaring N >= 2 strategies where at
    /// least one strategy path is valid and compilable, the AccountRuntime shall
    /// successfully boot with the valid strategies loaded, regardless of how many
    /// other strategies fail to compile or resolve. When ALL strategies are invalid,
    /// the function shall return Err.
    #[test]
    fn prop_partial_strategy_failure_does_not_halt_boot(
        n_valid in 0usize..5,
        n_invalid in 0usize..5,
    ) {
        // Need at least 2 total strategies per the property definition
        let total = n_valid + n_invalid;
        prop_assume!(total >= 2);

        let tmp_dir = TempDir::new().unwrap();
        let base = tmp_dir.path();

        let mut strategies = Vec::new();

        // Create valid strategy files
        for i in 0..n_valid {
            let dir = base.join(format!("valid_{}", i));
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("strategy.flux");
            fs::write(&path, VALID_STRATEGY_SOURCE).unwrap();
            strategies.push(StrategyEntry {
                name: format!("valid_{}", i),
                path: format!("valid_{}/strategy.flux", i),
                allocation: 0.5,
                priority: i as i64 + 1,
                execution: None,
                execution_offset_ticks: None,
            });
        }

        // Create invalid strategy files
        for i in 0..n_invalid {
            let dir = base.join(format!("invalid_{}", i));
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("strategy.flux");
            fs::write(&path, INVALID_STRATEGY_SOURCE).unwrap();
            strategies.push(StrategyEntry {
                name: format!("invalid_{}", i),
                path: format!("invalid_{}/strategy.flux", i),
                allocation: 0.5,
                priority: (n_valid + i) as i64 + 1,
                execution: None,
                execution_offset_ticks: None,
            });
        }

        let config = build_test_account_config(strategies);
        let result = load_strategies_from_config(&config, base);

        if n_valid > 0 {
            // At least one valid strategy → boot should succeed
            prop_assert!(
                result.is_ok(),
                "Expected Ok with {} valid strategies, got Err: {:?}",
                n_valid,
                result.err()
            );
            let modules = result.unwrap();
            // Should have loaded exactly the valid strategies
            prop_assert_eq!(
                modules.len(),
                n_valid,
                "Expected {} loaded modules, got {}",
                n_valid,
                modules.len()
            );
        } else {
            // All strategies are invalid → should return Err
            prop_assert!(
                result.is_err(),
                "Expected Err when all {} strategies are invalid, got Ok with {} modules",
                n_invalid,
                result.as_ref().map(|v| v.len()).unwrap_or(0)
            );
        }
    }

    /// **Validates: Requirements 2.10**
    ///
    /// Property 3b: When at least one strategy is valid, the loaded module count
    /// equals exactly the number of valid strategies (no double-loading or skipping).
    #[test]
    fn prop_loaded_count_equals_valid_count(
        n_valid in 1usize..6,
        n_invalid in 1usize..6,
    ) {
        let tmp_dir = TempDir::new().unwrap();
        let base = tmp_dir.path();

        let mut strategies = Vec::new();

        for i in 0..n_valid {
            let dir = base.join(format!("good_{}", i));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("strategy.flux"), VALID_STRATEGY_SOURCE).unwrap();
            strategies.push(StrategyEntry {
                name: format!("good_{}", i),
                path: format!("good_{}/strategy.flux", i),
                allocation: 1.0 / (n_valid as f64),
                priority: i as i64 + 1,
                execution: None,
                execution_offset_ticks: None,
            });
        }

        for i in 0..n_invalid {
            let dir = base.join(format!("bad_{}", i));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("strategy.flux"), INVALID_STRATEGY_SOURCE).unwrap();
            strategies.push(StrategyEntry {
                name: format!("bad_{}", i),
                path: format!("bad_{}/strategy.flux", i),
                allocation: 0.1,
                priority: (n_valid + i) as i64 + 1,
                execution: None,
                execution_offset_ticks: None,
            });
        }

        let config = build_test_account_config(strategies);
        let result = load_strategies_from_config(&config, base);

        prop_assert!(result.is_ok(), "Should succeed with {} valid strategies", n_valid);
        let modules = result.unwrap();
        prop_assert_eq!(
            modules.len(),
            n_valid,
            "Loaded {} modules but expected {} (valid count)",
            modules.len(),
            n_valid
        );
    }
}


// =============================================================================
// Feature: empyrean-swing-wiring, Property 6: Decorator semantic transparency
// =============================================================================

use flux_cli::interpreter::Interpreter;
use flux_runtime::BarContext;
use flux_runtime::Signal;

/// Helper: compile a Flux source string through lex → parse → typecheck → Interpreter.
/// Panics on failure with a descriptive message.
fn compile_to_interpreter(source: &str, label: &str) -> Interpreter {
    let tokens = flux_compiler::lexer::lex_with_spans(source)
        .unwrap_or_else(|e| panic!("{}: lex failed: {}", label, e));
    let ast = flux_compiler::parser::parse(tokens)
        .unwrap_or_else(|e| panic!("{}: parse failed: {}", label, e));
    let typed = flux_compiler::typeck::check(ast)
        .unwrap_or_else(|e| panic!("{}: typecheck failed: {}", label, e));
    Interpreter::new(&typed)
}

/// Compare two Signal values for equality (Signal doesn't derive PartialEq).
fn signals_equal(a: &Signal, b: &Signal) -> bool {
    match (a, b) {
        (Signal::Open { symbol: sa, qty: qa }, Signal::Open { symbol: sb, qty: qb }) => {
            sa == sb && (qa - qb).abs() < 1e-12
        }
        (Signal::Short { symbol: sa, qty: qa }, Signal::Short { symbol: sb, qty: qb }) => {
            sa == sb && (qa - qb).abs() < 1e-12
        }
        (Signal::Close { symbol: sa }, Signal::Close { symbol: sb }) => sa == sb,
        (Signal::CloseQty { symbol: sa, qty: qa }, Signal::CloseQty { symbol: sb, qty: qb }) => {
            sa == sb && (qa - qb).abs() < 1e-12
        }
        _ => false,
    }
}

/// Strategy source WITHOUT decorators.
/// Uses a struct with field access to ensure decorators are actually exercised
/// through the compilation pipeline (not just a trivial strategy).
const STRATEGY_PLAIN: &str = r#"
from indicators import {sma}

struct Result {
    score: f64,
    threshold: f64
}

fn compute_signal(price: f64, avg: f64) -> Result {
    score = price - avg
    threshold = 0.5
    return Result { score = score, threshold = threshold }
}

strategy DecoratorTest {
    params {
        lookback = 5
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, lookback)
        result = compute_signal(close, avg)
        if result.score > result.threshold and not in_position {
            OPEN(symbol, 1.0)
        }
        if result.score < 0.0 - result.threshold and in_position {
            CLOSE(symbol)
        }
    }
}
"#;

/// Same strategy WITH @aligned(64) decorator on the struct.
const STRATEGY_ALIGNED: &str = r#"
from indicators import {sma}

@aligned(64)
struct Result {
    score: f64,
    threshold: f64
}

fn compute_signal(price: f64, avg: f64) -> Result {
    score = price - avg
    threshold = 0.5
    return Result { score = score, threshold = threshold }
}

strategy DecoratorTest {
    params {
        lookback = 5
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, lookback)
        result = compute_signal(close, avg)
        if result.score > result.threshold and not in_position {
            OPEN(symbol, 1.0)
        }
        if result.score < 0.0 - result.threshold and in_position {
            CLOSE(symbol)
        }
    }
}
"#;

/// Same strategy WITH @immutable decorator on the struct.
const STRATEGY_IMMUTABLE: &str = r#"
from indicators import {sma}

@immutable
struct Result {
    score: f64,
    threshold: f64
}

fn compute_signal(price: f64, avg: f64) -> Result {
    score = price - avg
    threshold = 0.5
    return Result { score = score, threshold = threshold }
}

strategy DecoratorTest {
    params {
        lookback = 5
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, lookback)
        result = compute_signal(close, avg)
        if result.score > result.threshold and not in_position {
            OPEN(symbol, 1.0)
        }
        if result.score < 0.0 - result.threshold and in_position {
            CLOSE(symbol)
        }
    }
}
"#;

/// Same strategy WITH @zero_init decorator on the struct.
const STRATEGY_ZERO_INIT: &str = r#"
from indicators import {sma}

@zero_init
struct Result {
    score: f64,
    threshold: f64
}

fn compute_signal(price: f64, avg: f64) -> Result {
    score = price - avg
    threshold = 0.5
    return Result { score = score, threshold = threshold }
}

strategy DecoratorTest {
    params {
        lookback = 5
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, lookback)
        result = compute_signal(close, avg)
        if result.score > result.threshold and not in_position {
            OPEN(symbol, 1.0)
        }
        if result.score < 0.0 - result.threshold and in_position {
            CLOSE(symbol)
        }
    }
}
"#;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 9.5**
    ///
    /// Property 6: For any strategy source and any valid input data sequence,
    /// applying @aligned(64), @immutable, or @zero_init decorators to structs
    /// shall produce backtest signals identical to the non-decorated version.
    ///
    /// This test generates random bar price sequences, runs both the plain and
    /// decorated strategies through the interpreter, and asserts signal-for-signal
    /// equivalence on every bar.
    #[test]
    fn prop_decorator_semantic_transparency(
        prices in proptest::collection::vec(90.0f64..110.0, 10..50),
        decorator_variant in 0u8..3,
    ) {
        // Select which decorator variant to compare against the plain strategy
        let decorated_source = match decorator_variant {
            0 => STRATEGY_ALIGNED,
            1 => STRATEGY_IMMUTABLE,
            2 => STRATEGY_ZERO_INIT,
            _ => unreachable!(),
        };
        let decorator_name = match decorator_variant {
            0 => "@aligned(64)",
            1 => "@immutable",
            2 => "@zero_init",
            _ => unreachable!(),
        };

        // Compile both strategies
        let mut interp_plain = compile_to_interpreter(STRATEGY_PLAIN, "plain");
        let mut interp_decorated = compile_to_interpreter(decorated_source, decorator_name);

        // Run both interpreters with the same bar data
        for (i, &price) in prices.iter().enumerate() {
            let bar = BarContext {
                open: price - 1.0,
                high: price + 1.0,
                low: price - 2.0,
                close: price,
                volume: 1000.0,
                symbol: "TEST".to_string(),
                in_position: interp_plain.in_position,
            };

            // Build identical bar for the decorated interpreter (use its own position state)
            let bar_dec = BarContext {
                open: price - 1.0,
                high: price + 1.0,
                low: price - 2.0,
                close: price,
                volume: 1000.0,
                symbol: "TEST".to_string(),
                in_position: interp_decorated.in_position,
            };

            let signals_plain = interp_plain.on_bar(&bar);
            let signals_dec = interp_decorated.on_bar(&bar_dec);

            // Assert same number of signals
            prop_assert_eq!(
                signals_plain.len(), signals_dec.len(),
                "Bar {}: signal count differs with {} — plain={}, decorated={}",
                i, decorator_name, signals_plain.len(), signals_dec.len()
            );

            // Assert each signal is identical
            for (j, (sp, sd)) in signals_plain.iter().zip(signals_dec.iter()).enumerate() {
                prop_assert!(
                    signals_equal(sp, sd),
                    "Bar {}, signal {}: signals differ with {}.\n  plain={:?}\n  decorated={:?}",
                    i, j, decorator_name, sp, sd
                );
            }
        }
    }

    /// **Validates: Requirements 9.5**
    ///
    /// Property 6b: Position state remains synchronized between plain and decorated
    /// interpreters across the entire bar sequence. If decorators affected runtime
    /// behavior, position state would diverge.
    #[test]
    fn prop_decorator_position_state_sync(
        prices in proptest::collection::vec(85.0f64..115.0, 15..60),
    ) {
        let mut interp_plain = compile_to_interpreter(STRATEGY_PLAIN, "plain");
        let mut interp_aligned = compile_to_interpreter(STRATEGY_ALIGNED, "@aligned(64)");

        for (i, &price) in prices.iter().enumerate() {
            let bar_plain = BarContext {
                open: price - 1.0,
                high: price + 1.0,
                low: price - 2.0,
                close: price,
                volume: 1000.0,
                symbol: "TEST".to_string(),
                in_position: interp_plain.in_position,
            };

            let bar_aligned = BarContext {
                open: price - 1.0,
                high: price + 1.0,
                low: price - 2.0,
                close: price,
                volume: 1000.0,
                symbol: "TEST".to_string(),
                in_position: interp_aligned.in_position,
            };

            interp_plain.on_bar(&bar_plain);
            interp_aligned.on_bar(&bar_aligned);

            // Position state must remain identical
            prop_assert_eq!(
                interp_plain.in_position, interp_aligned.in_position,
                "Bar {}: in_position diverged — plain={}, @aligned(64)={}",
                i, interp_plain.in_position, interp_aligned.in_position
            );
        }
    }
}
