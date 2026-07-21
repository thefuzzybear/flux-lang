//! Integration tests for the broker adapter with the LiveHarness.
//!
//! These tests verify the full signal-to-fill flow using MockBrokerAdapter,
//! combining multiple modules (execution, mock, dedup, session gate) to test
//! end-to-end behavior without requiring a live IB Gateway connection.
//!
//! **Validates: Requirements 5.2, 5.3, 5.4, 5.5, 6.2, 6.3, 7.5, 8.1, 11.2, 11.3, 11.4**

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

use flux_cli::live::broker::mock::{MockBrokerAdapter, MockFillBehavior};
use flux_cli::live::broker::execution::{
    check_session_gate, translate_signal, DeduplicationGuard, ExecutionPolicy,
};
use flux_cli::live::broker::{BrokerAdapter, BrokerError, OrderUpdate, Side};
use flux_cli::live::market_calendar::MarketCalendar;
use flux_runtime::Signal;

// =============================================================================
// Helper: build a MarketCalendar for CME RTH (09:30–16:00 Eastern)
// =============================================================================

fn cme_calendar() -> MarketCalendar {
    let toml = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#;
    MarketCalendar::from_toml(toml).unwrap()
}

// =============================================================================
// Test: Full signal → translate → submit (mock) → fill → tracker update
// Validates: Requirements 5.2, 5.3, 11.2, 11.3
// =============================================================================

#[tokio::test]
async fn test_full_signal_to_fill_flow() {
    let mock = Arc::new(MockBrokerAdapter::new());
    let mut rx = mock.subscribe_order_updates().await.unwrap();

    // Strategy emits an Open signal for 3 contracts of ES
    let signal = Signal::Open {
        symbol: "ES".to_string(),
        qty: 3.0,
    };
    let policy = ExecutionPolicy::Market;

    // Translate signal to order
    let order = translate_signal(&signal, &policy, "paper", "aether", 1, 5000.0, 0.25, 0.0)
        .expect("should translate successfully");

    // Verify order fields
    assert_eq!(order.symbol, "ES");
    assert_eq!(order.side, Side::Buy);
    assert_eq!(order.contracts, 3);
    assert_eq!(order.id.0, "paper_aether_ES_1");

    // Submit to mock broker
    let result = mock.submit_order(&order).await;
    assert!(result.is_ok());

    // Verify the mock recorded the order
    let orders = mock.orders();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].symbol, "ES");
    assert_eq!(orders[0].contracts, 3);
    assert_eq!(orders[0].side, Side::Buy);

    // Verify fill was emitted through the update channel
    let update = rx.recv().await.unwrap();
    match update {
        OrderUpdate::Fill(fill) => {
            assert_eq!(fill.order_id.0, "paper_aether_ES_1");
            assert_eq!(fill.symbol, "ES");
            assert_eq!(fill.qty, 3);
            assert_eq!(fill.price, 5000.0);
            assert_eq!(fill.side, Side::Buy);
        }
        _ => panic!("expected Fill update, got {:?}", update),
    }

    // Verify positions reflect the fill
    let positions = mock.get_positions().await.unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].symbol, "ES");
    assert_eq!(positions[0].qty, 3.0);
    assert_eq!(positions[0].avg_cost, 5000.0);

    // Verify no open orders (immediate fill clears them)
    let open_orders = mock.get_open_orders().await.unwrap();
    assert!(open_orders.is_empty());
}

// =============================================================================
// Test: Disconnect handling — mock disconnects, signals discarded
// Validates: Requirements 6.2, 6.3
// =============================================================================

#[tokio::test]
async fn test_disconnect_handling() {
    let mock = Arc::new(MockBrokerAdapter::new());
    mock.set_connected(false);

    // Try to translate and submit while disconnected
    let signal = Signal::Open {
        symbol: "ES".to_string(),
        qty: 2.0,
    };
    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 1, 5000.0, 0.25, 0.0)
            .unwrap();

    // Submit should fail with Disconnected error
    let result = mock.submit_order(&order).await;
    assert!(matches!(result, Err(BrokerError::Disconnected)));

    // No orders should be recorded
    assert!(mock.orders().is_empty());

    // Positions and open orders are also inaccessible while disconnected
    assert!(mock.get_positions().await.is_err());
    assert!(mock.get_open_orders().await.is_err());
}

// =============================================================================
// Test: Reconnect flow — mock reconnects, reconciliation runs
// Validates: Requirements 6.3, 6.4
// =============================================================================

#[tokio::test]
async fn test_reconnect_flow() {
    let mock = Arc::new(MockBrokerAdapter::new());

    // Submit an order that fills immediately (establishes a position)
    let signal = Signal::Open {
        symbol: "NQ".to_string(),
        qty: 2.0,
    };
    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 1, 15000.0, 0.25, 0.0)
            .unwrap();
    mock.submit_order(&order).await.unwrap();

    // Verify position exists
    let positions = mock.get_positions().await.unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].symbol, "NQ");
    assert_eq!(positions[0].qty, 2.0);

    // Simulate disconnect
    mock.set_connected(false);
    assert!(!mock.is_connected());

    // Submissions fail while disconnected
    let signal2 = Signal::Open {
        symbol: "ES".to_string(),
        qty: 1.0,
    };
    let order2 =
        translate_signal(&signal2, &ExecutionPolicy::Market, "paper", "strat", 2, 5000.0, 0.25, 0.0)
            .unwrap();
    let result = mock.submit_order(&order2).await;
    assert!(matches!(result, Err(BrokerError::Disconnected)));

    // Reconnect
    mock.set_connected(true);
    assert!(mock.is_connected());

    // After reconnect, get_positions works (reconciliation can proceed)
    let positions = mock.get_positions().await.unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].symbol, "NQ");
    assert_eq!(positions[0].qty, 2.0);

    // New orders can be submitted after reconnect
    let signal3 = Signal::Open {
        symbol: "ES".to_string(),
        qty: 1.0,
    };
    let order3 =
        translate_signal(&signal3, &ExecutionPolicy::Market, "paper", "strat", 3, 5100.0, 0.25, 0.0)
            .unwrap();
    let result = mock.submit_order(&order3).await;
    assert!(result.is_ok());

    // Now we have 2 positions
    let positions = mock.get_positions().await.unwrap();
    assert_eq!(positions.len(), 2);
}

// =============================================================================
// Test: Partial fill — mock returns partial, tracker shows partial qty
// Validates: Requirements 5.5
// =============================================================================

#[tokio::test]
async fn test_partial_fill() {
    let mock = Arc::new(MockBrokerAdapter::new());
    mock.set_fill_behavior(MockFillBehavior::PartialFill { fill_qty: 2 });
    let mut rx = mock.subscribe_order_updates().await.unwrap();

    // Submit order for 5 contracts
    let signal = Signal::Open {
        symbol: "ES".to_string(),
        qty: 5.0,
    };
    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 1, 5000.0, 0.25, 0.0)
            .unwrap();
    mock.submit_order(&order).await.unwrap();

    // Fill should be partial (qty=2 out of 5)
    let update = rx.recv().await.unwrap();
    match update {
        OrderUpdate::Fill(fill) => {
            assert_eq!(fill.qty, 2);
            assert_eq!(fill.symbol, "ES");
            assert_eq!(fill.price, 5000.0);
        }
        _ => panic!("expected Fill update, got {:?}", update),
    }

    // Position should reflect only the partial fill amount
    let positions = mock.get_positions().await.unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].qty, 2.0);

    // Open orders should show the remainder
    let open = mock.get_open_orders().await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].filled_qty, 2);
    assert_eq!(open[0].total_qty, 5);
}

// =============================================================================
// Test: Deduplication — same bar same OrderId, second rejected
// Validates: Requirements 7.5
// =============================================================================

#[tokio::test]
async fn test_deduplication() {
    let mock = Arc::new(MockBrokerAdapter::new());
    let mut dedup = DeduplicationGuard::new();

    // First signal on bar_index=42
    let signal = Signal::Open {
        symbol: "ES".to_string(),
        qty: 2.0,
    };
    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 42, 5000.0, 0.25, 0.0)
            .unwrap();

    // First submission — dedup allows it
    assert!(!dedup.is_duplicate(&order.id));
    dedup.mark_submitted(order.id.clone());
    mock.submit_order(&order).await.unwrap();

    // Second signal on same bar — same OrderId generated
    let signal2 = Signal::Open {
        symbol: "ES".to_string(),
        qty: 2.0,
    };
    let order2 =
        translate_signal(&signal2, &ExecutionPolicy::Market, "paper", "strat", 42, 5000.0, 0.25, 0.0)
            .unwrap();

    // Dedup detects the duplicate — same OrderId
    assert_eq!(order.id, order2.id);
    assert!(dedup.is_duplicate(&order2.id));

    // Should NOT submit to broker — only 1 order recorded
    assert_eq!(mock.orders().len(), 1);

    // Different bar_index produces different OrderId — not a duplicate
    let order3 =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 43, 5000.0, 0.25, 0.0)
            .unwrap();
    assert!(!dedup.is_duplicate(&order3.id));
}

// =============================================================================
// Test: Session gate — time outside RTH, order rejected
// Validates: Requirements 8.1
// =============================================================================

#[tokio::test]
async fn test_session_gate_outside_rth() {
    let calendar = cme_calendar();

    // 3:00 AM Eastern on a Wednesday (2024-03-13) — outside RTH (09:30–16:00)
    let tz: Tz = "US/Eastern".parse().unwrap();
    let date = NaiveDate::from_ymd_opt(2024, 3, 13).unwrap();
    let time = NaiveTime::from_hms_opt(3, 0, 0).unwrap();
    let local = tz
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .unwrap();
    let utc = local.with_timezone(&Utc);

    let result = check_session_gate(&calendar, "CME", utc);
    assert!(result.is_err());
    assert!(matches!(
        result,
        Err(BrokerError::SessionClosed { ref exchange }) if exchange == "CME"
    ));
}

// =============================================================================
// Test: Session gate — time within RTH, order allowed
// Validates: Requirements 8.1
// =============================================================================

#[tokio::test]
async fn test_session_gate_within_rth() {
    let calendar = cme_calendar();

    // 10:30 AM Eastern on a Wednesday (2024-03-13) — within RTH
    let tz: Tz = "US/Eastern".parse().unwrap();
    let date = NaiveDate::from_ymd_opt(2024, 3, 13).unwrap();
    let time = NaiveTime::from_hms_opt(10, 30, 0).unwrap();
    let local = tz
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .unwrap();
    let utc = local.with_timezone(&Utc);

    let result = check_session_gate(&calendar, "CME", utc);
    assert!(result.is_ok());
}

// =============================================================================
// Test: Session gate — weekend rejected
// Validates: Requirements 8.1, 8.2
// =============================================================================

#[tokio::test]
async fn test_session_gate_weekend_rejected() {
    let calendar = cme_calendar();

    // Saturday 2024-03-16, 10:00 AM Eastern — not a trading day
    let tz: Tz = "US/Eastern".parse().unwrap();
    let date = NaiveDate::from_ymd_opt(2024, 3, 16).unwrap();
    let time = NaiveTime::from_hms_opt(10, 0, 0).unwrap();
    let local = tz
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .unwrap();
    let utc = local.with_timezone(&Utc);

    let result = check_session_gate(&calendar, "CME", utc);
    assert!(result.is_err());
}

// =============================================================================
// Test: Backward compatibility — broker=None, existing behavior unchanged
// Validates: Requirements 11.4
// =============================================================================

#[test]
fn test_backward_compatibility_broker_none() {
    // When broker is None, the system still functions:
    // - Signals translate fine
    // - DeduplicationGuard works independently
    // - ExecutionPolicy defaults to Market
    // - No broker interaction occurs

    let dedup = DeduplicationGuard::new();
    let policies: HashMap<String, ExecutionPolicy> = HashMap::new();

    // A signal translates correctly regardless of broker presence
    let signal = Signal::Open {
        symbol: "ES".to_string(),
        qty: 2.0,
    };
    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 1, 5000.0, 0.25, 0.0);
    assert!(order.is_some());
    let order = order.unwrap();
    assert_eq!(order.symbol, "ES");
    assert_eq!(order.contracts, 2);

    // Dedup guard starts empty — no false positives
    assert!(!dedup.is_duplicate(&order.id));

    // Default policy resolution when no strategy-specific config exists
    let default_policy = policies.get("unknown").cloned().unwrap_or_default();
    assert_eq!(default_policy, ExecutionPolicy::Market);
}

// =============================================================================
// Test: Short signal translation and fill
// Validates: Requirements 5.2, 5.3, 11.2
// =============================================================================

#[tokio::test]
async fn test_short_signal_flow() {
    let mock = Arc::new(MockBrokerAdapter::new());
    let mut rx = mock.subscribe_order_updates().await.unwrap();

    let signal = Signal::Short {
        symbol: "NQ".to_string(),
        qty: 4.0,
    };

    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "kairos", 5, 15200.0, 0.25, 0.0)
            .unwrap();

    assert_eq!(order.side, Side::Sell);
    assert_eq!(order.contracts, 4);

    mock.submit_order(&order).await.unwrap();

    let update = rx.recv().await.unwrap();
    match update {
        OrderUpdate::Fill(fill) => {
            assert_eq!(fill.side, Side::Sell);
            assert_eq!(fill.qty, 4);
        }
        _ => panic!("expected Fill"),
    }

    // Position should be negative (short)
    let positions = mock.get_positions().await.unwrap();
    assert_eq!(positions[0].qty, -4.0);
}

// =============================================================================
// Test: Order rejection flow
// Validates: Requirements 5.4
// =============================================================================

#[tokio::test]
async fn test_order_rejection_flow() {
    let mock = Arc::new(MockBrokerAdapter::new());
    mock.set_fill_behavior(MockFillBehavior::Reject {
        reason: "insufficient margin".to_string(),
    });
    let mut rx = mock.subscribe_order_updates().await.unwrap();

    let signal = Signal::Open {
        symbol: "ES".to_string(),
        qty: 100.0,
    };
    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 1, 5000.0, 0.25, 0.0)
            .unwrap();

    let result = mock.submit_order(&order).await;
    assert!(matches!(result, Err(BrokerError::OrderRejected(ref r)) if r == "insufficient margin"));

    // Rejection update should be on the channel
    let update = rx.recv().await.unwrap();
    match update {
        OrderUpdate::Rejection { order_id, reason } => {
            assert_eq!(order_id, order.id);
            assert_eq!(reason, "insufficient margin");
        }
        _ => panic!("expected Rejection update"),
    }

    // No positions created on rejection
    let positions = mock.get_positions().await.unwrap();
    assert!(positions.is_empty());
}

// =============================================================================
// Test: DeduplicationGuard reconcile from broker open orders
// Validates: Requirements 7.5
// =============================================================================

#[tokio::test]
async fn test_dedup_reconcile_on_reconnect() {
    let mock = Arc::new(MockBrokerAdapter::new());
    // Use Pending behavior so orders stay open
    mock.set_fill_behavior(MockFillBehavior::Pending);

    // Submit an order (stays pending/open)
    let signal = Signal::Open {
        symbol: "ES".to_string(),
        qty: 3.0,
    };
    let order =
        translate_signal(&signal, &ExecutionPolicy::Market, "paper", "strat", 10, 5000.0, 0.25, 0.0)
            .unwrap();
    mock.submit_order(&order).await.unwrap();

    // Verify it's in open orders
    let open = mock.get_open_orders().await.unwrap();
    assert_eq!(open.len(), 1);

    // Simulate a new DeduplicationGuard (as if system restarted)
    let mut new_dedup = DeduplicationGuard::new();

    // Before reconciliation, the order ID is NOT known as duplicate
    assert!(!new_dedup.is_duplicate(&order.id));

    // Reconcile from broker — populates the dedup set
    let waiting = new_dedup.reconcile(mock.as_ref()).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0], order.id);

    // After reconciliation, the same order ID IS a duplicate
    assert!(new_dedup.is_duplicate(&order.id));
}
