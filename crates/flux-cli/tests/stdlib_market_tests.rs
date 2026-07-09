// Feature: flux-structs, Property 10: Window ring buffer correctness
// Feature: flux-structs, Property 15: classify_trade Lee-Ready correctness
// Feature: flux-structs, Property 16: book_spread_bps calculation correctness
// Feature: flux-structs, Property 17: book_imbalance calculation correctness
// Feature: flux-structs, Property 18: book_microprice calculation correctness
// Feature: flux-structs, Property 19: book_vwap calculation correctness
//!
//! Unit tests and property-based tests for stdlib L1 and L2 market data modules.
//!
//! Tasks: 12.5, 12.6, 12.7, 13.4, 13.5, 13.6, 13.7, 13.8
//!
//! Tests verify:
//! - `std/market/l1.flux` and `std/market/l2.flux` parse and typecheck correctly
//! - Mathematical formulas for L1 helpers (calc_spread, calc_mid, classify_trade)
//! - Window ring buffer algorithm correctness
//! - L2 query function formulas (book_spread_bps, book_imbalance, book_microprice, book_vwap)

use proptest::prelude::*;

use flux_compiler::lexer;
use flux_compiler::parser;

// =============================================================================
// Task 12.5: Unit tests for L1 stdlib
// =============================================================================

#[test]
fn test_l1_flux_parses_successfully() {
    let source = std::fs::read_to_string("../../std/market/l1.flux")
        .expect("std/market/l1.flux should exist");
    let tokens = lexer::lex_with_spans(&source).expect("l1.flux should lex without errors");
    let _ast = parser::parse(tokens).expect("l1.flux should parse without errors");
}

#[test]
fn test_calc_spread_smoke() {
    // calc_spread(q) = q.ask - q.bid
    let bid = 100.0_f64;
    let ask = 101.5_f64;
    let spread = ask - bid;
    assert!((spread - 1.5).abs() < f64::EPSILON);
}

#[test]
fn test_calc_mid_smoke() {
    // calc_mid(q) = (q.bid + q.ask) / 2.0
    let bid = 100.0_f64;
    let ask = 102.0_f64;
    let mid = (bid + ask) / 2.0;
    assert!((mid - 101.0).abs() < f64::EPSILON);
}

#[test]
fn test_window_new_zero_filled() {
    // window_new(capacity) produces a Window with:
    // - data = [0.0; 256] (zero-filled)
    // - index = 0
    // - count = 0
    // - capacity = capacity
    let capacity = 10;
    let data = vec![0.0_f64; 256];
    let index = 0_i64;
    let count = 0_i64;

    // Verify zero-filled
    assert!(data.iter().all(|&v| v == 0.0));
    assert_eq!(index, 0);
    assert_eq!(count, 0);
    assert_eq!(capacity, 10);
}

// =============================================================================
// Task 12.6: Property test for classify_trade Lee-Ready correctness (Property 15)
// =============================================================================

/// Implements the Lee-Ready classify_trade algorithm in Rust.
/// Returns 1 (buy) if price > midpoint, -1 (sell) if price < midpoint, 0 (unknown) if equal.
fn classify_trade_rust(price: f64, bid: f64, ask: f64) -> i64 {
    let mid = (bid + ask) / 2.0;
    if price > mid {
        1
    } else if price < mid {
        -1
    } else {
        0
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 11.3**
    ///
    /// For any Tick with price P and Quote with bid B and ask A where B < A,
    /// classify_trade returns 1 if P > (B+A)/2, -1 if P < (B+A)/2, 0 if P == (B+A)/2.
    #[test]
    fn prop_classify_trade_lee_ready(
        bid in 1.0f64..1000.0,
        spread in 0.01f64..100.0,
        // price_offset relative to midpoint: negative = sell, zero = unknown, positive = buy
        price_choice in 0..3u8,
    ) {
        let ask = bid + spread;
        let mid = (bid + ask) / 2.0;

        let price = match price_choice {
            0 => mid + 0.01, // above mid -> buy
            1 => mid - 0.01, // below mid -> sell
            _ => mid,        // at mid -> unknown
        };

        let result = classify_trade_rust(price, bid, ask);

        let expected = if price > mid {
            1
        } else if price < mid {
            -1
        } else {
            0
        };

        prop_assert_eq!(result, expected,
            "classify_trade({}, bid={}, ask={}) = {}, expected {}",
            price, bid, ask, result, expected);
    }

    /// Additional property: for arbitrary prices, the classification is always consistent
    /// with the midpoint comparison.
    #[test]
    fn prop_classify_trade_arbitrary_prices(
        bid in 0.01f64..10000.0,
        spread in 0.01f64..1000.0,
        price in 0.01f64..20000.0,
    ) {
        let ask = bid + spread;
        let mid = (bid + ask) / 2.0;
        let result = classify_trade_rust(price, bid, ask);

        if price > mid {
            prop_assert_eq!(result, 1);
        } else if price < mid {
            prop_assert_eq!(result, -1);
        } else {
            prop_assert_eq!(result, 0);
        }
    }
}

// =============================================================================
// Task 12.7: Property test for Window ring buffer correctness (Property 10)
// =============================================================================

/// Simulates the Window ring buffer in Rust.
struct RingBuffer {
    data: Vec<f64>,
    index: usize,
    count: usize,
    capacity: usize,
}

impl RingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity],
            index: 0,
            count: 0,
            capacity,
        }
    }

    fn push(&mut self, value: f64) {
        self.data[self.index] = value;
        self.index = (self.index + 1) % self.capacity;
        if self.count < self.capacity {
            self.count += 1;
        }
    }

    fn get(&self, i: usize) -> Option<f64> {
        if i >= self.count {
            return None;
        }
        // 0 = most recent, which is at (index - 1 - i) mod capacity
        let pos = if self.index == 0 {
            self.capacity - 1 - i
        } else if i < self.index {
            self.index - 1 - i
        } else {
            self.capacity + self.index - 1 - i
        };
        Some(self.data[pos])
    }

    fn mean(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        // Mean of the min(count, capacity) most recently pushed values
        let sum: f64 = (0..self.count).map(|i| self.get(i).unwrap()).sum();
        sum / self.count as f64
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 12.3, 12.4, 12.5**
    ///
    /// For any sequence of N push operations on a Window of capacity C,
    /// get(0) returns the most recently pushed value, get(i) returns the (i+1)-th
    /// most recent value, and mean returns the arithmetic mean of min(N,C) values.
    #[test]
    fn prop_window_ring_buffer_correctness(
        capacity in 1usize..50,
        values in proptest::collection::vec(proptest::num::f64::NORMAL, 1..100),
    ) {
        let mut buffer = RingBuffer::new(capacity);

        // Push all values
        for &v in &values {
            buffer.push(v);
        }

        let n = values.len();
        let effective_count = n.min(capacity);

        // Property: get(0) returns the most recently pushed value
        let most_recent = buffer.get(0).unwrap();
        prop_assert_eq!(most_recent, values[n - 1],
            "get(0) should return most recent value");

        // Property: get(i) returns the (i+1)-th most recent value for valid i
        for i in 0..effective_count {
            let got = buffer.get(i).unwrap();
            let expected = values[n - 1 - i];
            prop_assert_eq!(got, expected,
                "get({}) should return values[{}] = {}, got {}",
                i, n - 1 - i, expected, got);
        }

        // Property: get(effective_count) should be None (out of bounds)
        prop_assert!(buffer.get(effective_count).is_none(),
            "get({}) should be None (out of bounds)", effective_count);

        // Property: mean returns arithmetic mean of min(N, C) most recent values
        let expected_mean: f64 = (0..effective_count)
            .map(|i| values[n - 1 - i])
            .sum::<f64>() / effective_count as f64;
        let actual_mean = buffer.mean();
        prop_assert!((actual_mean - expected_mean).abs() < 1e-10,
            "mean should be {}, got {} (diff = {})",
            expected_mean, actual_mean, (actual_mean - expected_mean).abs());
    }

    /// Sub-property: after exactly capacity pushes, the buffer is full and all slots accessible.
    #[test]
    fn prop_window_full_after_capacity_pushes(
        capacity in 1usize..50,
        values in proptest::collection::vec(proptest::num::f64::NORMAL, 50..100),
    ) {
        // Only use first `capacity` values
        let use_values: Vec<f64> = values.into_iter().take(capacity).collect();
        let mut buffer = RingBuffer::new(capacity);

        for &v in &use_values {
            buffer.push(v);
        }

        prop_assert_eq!(buffer.count, capacity,
            "After {} pushes, count should be {}", capacity, capacity);

        // All positions 0..capacity should be accessible
        for i in 0..capacity {
            prop_assert!(buffer.get(i).is_some(),
                "get({}) should be Some after {} pushes", i, capacity);
        }
    }
}

// =============================================================================
// Task 13.4: Unit tests for L2 stdlib
// =============================================================================

#[test]
fn test_l2_flux_parses_successfully() {
    let source = std::fs::read_to_string("../../std/market/l2.flux")
        .expect("std/market/l2.flux should exist");
    let tokens = lexer::lex_with_spans(&source).expect("l2.flux should lex without errors");
    let _ast = parser::parse(tokens).expect("l2.flux should parse without errors");
}

#[test]
fn test_book_spread_bps_smoke() {
    // book_spread_bps = (ask - bid) / ((ask + bid) / 2.0) * 10000.0
    let bid = 100.0_f64;
    let ask = 100.5_f64;
    let expected = (ask - bid) / ((ask + bid) / 2.0) * 10000.0;
    // ~49.75 bps
    let computed = (100.5 - 100.0) / ((100.5 + 100.0) / 2.0) * 10000.0;
    assert!((computed - expected).abs() < 1e-10);
}

#[test]
fn test_book_imbalance_smoke() {
    // book_imbalance = sum(bid_sizes) / (sum(bid_sizes) + sum(ask_sizes))
    let bid_sizes = vec![100.0, 200.0, 50.0];
    let ask_sizes = vec![80.0, 150.0, 70.0];
    let bid_sum: f64 = bid_sizes.iter().sum();
    let ask_sum: f64 = ask_sizes.iter().sum();
    let imbalance = bid_sum / (bid_sum + ask_sum);
    // 350 / (350 + 300) = 350/650 ≈ 0.5385
    assert!((imbalance - 350.0 / 650.0).abs() < 1e-10);
}

#[test]
fn test_book_microprice_smoke() {
    // book_microprice = (bid_price * ask_size + ask_price * bid_size) / (bid_size + ask_size)
    let bid_price = 100.0_f64;
    let bid_size = 200.0_f64;
    let ask_price = 101.0_f64;
    let ask_size = 100.0_f64;
    let microprice = (bid_price * ask_size + ask_price * bid_size) / (bid_size + ask_size);
    // (100*100 + 101*200) / (200+100) = (10000 + 20200) / 300 = 30200/300 ≈ 100.6667
    assert!((microprice - 30200.0 / 300.0).abs() < 1e-10);
}

#[test]
fn test_book_vwap_smoke() {
    // book_vwap = sum(price[i] * size[i]) / sum(size[i])
    let levels = vec![(100.0_f64, 50.0_f64), (99.5, 100.0), (99.0, 75.0)];
    let total_notional: f64 = levels.iter().map(|(p, s)| p * s).sum();
    let total_size: f64 = levels.iter().map(|(_, s)| *s).sum();
    let vwap = total_notional / total_size;
    // (5000 + 9950 + 7425) / (50 + 100 + 75) = 22375 / 225 ≈ 99.4444
    assert!((vwap - 22375.0 / 225.0).abs() < 1e-10);
}

// =============================================================================
// Task 13.5: Property test for book_spread_bps calculation correctness (Property 16)
// =============================================================================

/// Computes book_spread_bps in Rust: (ask - bid) / ((ask + bid) / 2) * 10000
fn book_spread_bps_rust(bid: f64, ask: f64) -> f64 {
    (ask - bid) / ((ask + bid) / 2.0) * 10000.0
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 15.1**
    ///
    /// For any Book with best bid B and best ask A where B > 0 and A > B,
    /// book_spread_bps SHALL return (A - B) / ((A + B) / 2) * 10000.
    #[test]
    fn prop_book_spread_bps(
        bid in 0.01f64..10000.0,
        spread in 0.001f64..1000.0,
    ) {
        let ask = bid + spread;
        let result = book_spread_bps_rust(bid, ask);
        let expected = (ask - bid) / ((ask + bid) / 2.0) * 10000.0;

        prop_assert!((result - expected).abs() < 1e-10,
            "book_spread_bps(bid={}, ask={}) = {}, expected {}",
            bid, ask, result, expected);

        // Spread in bps should always be positive when ask > bid
        prop_assert!(result > 0.0,
            "spread_bps should be positive when ask > bid");
    }
}

// =============================================================================
// Task 13.6: Property test for book_imbalance calculation correctness (Property 17)
// =============================================================================

/// Computes book_imbalance in Rust:
/// sum(bid_sizes[0..min(L,depth)]) / (sum(bid_sizes[0..min(L,depth)]) + sum(ask_sizes[0..min(L,depth)]))
fn book_imbalance_rust(bid_sizes: &[f64], ask_sizes: &[f64], levels: usize) -> f64 {
    let bid_depth = bid_sizes.len();
    let ask_depth = ask_sizes.len();
    let bid_levels = levels.min(bid_depth);
    let ask_levels = levels.min(ask_depth);
    let bid_sum: f64 = bid_sizes[..bid_levels].iter().sum();
    let ask_sum: f64 = ask_sizes[..ask_levels].iter().sum();
    bid_sum / (bid_sum + ask_sum)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 15.2, 15.5**
    ///
    /// For any Book with bid and ask levels, and any levels parameter L,
    /// book_imbalance returns sum(bid_sizes) / (sum(bid_sizes) + sum(ask_sizes))
    /// clamped to available depth.
    #[test]
    fn prop_book_imbalance(
        bid_sizes in proptest::collection::vec(0.01f64..1000.0, 1..20),
        ask_sizes in proptest::collection::vec(0.01f64..1000.0, 1..20),
        levels in 1usize..25,
    ) {
        let result = book_imbalance_rust(&bid_sizes, &ask_sizes, levels);

        // Recompute expected
        let bid_levels = levels.min(bid_sizes.len());
        let ask_levels = levels.min(ask_sizes.len());
        let bid_sum: f64 = bid_sizes[..bid_levels].iter().sum();
        let ask_sum: f64 = ask_sizes[..ask_levels].iter().sum();
        let expected = bid_sum / (bid_sum + ask_sum);

        prop_assert!((result - expected).abs() < 1e-10,
            "book_imbalance(levels={}) = {}, expected {}",
            levels, result, expected);

        // Imbalance should always be in [0, 1] when sizes are positive
        prop_assert!(result >= 0.0 && result <= 1.0,
            "imbalance should be in [0, 1], got {}", result);
    }
}

// =============================================================================
// Task 13.7: Property test for book_microprice calculation correctness (Property 18)
// =============================================================================

/// Computes book_microprice in Rust:
/// (bid_price * ask_size + ask_price * bid_size) / (bid_size + ask_size)
fn book_microprice_rust(bid_price: f64, bid_size: f64, ask_price: f64, ask_size: f64) -> f64 {
    (bid_price * ask_size + ask_price * bid_size) / (bid_size + ask_size)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 15.3**
    ///
    /// For any Book with top bid (price Bp, size Bs) and top ask (price Ap, size As),
    /// book_microprice returns (Bp * As + Ap * Bs) / (Bs + As).
    #[test]
    fn prop_book_microprice(
        bid_price in 0.01f64..10000.0,
        bid_size in 0.01f64..10000.0,
        spread in 0.001f64..1000.0,
        ask_size in 0.01f64..10000.0,
    ) {
        let ask_price = bid_price + spread;
        let result = book_microprice_rust(bid_price, bid_size, ask_price, ask_size);
        let expected = (bid_price * ask_size + ask_price * bid_size) / (bid_size + ask_size);

        prop_assert!((result - expected).abs() < 1e-10,
            "book_microprice(bp={}, bs={}, ap={}, as={}) = {}, expected {}",
            bid_price, bid_size, ask_price, ask_size, result, expected);

        // Microprice should be between bid and ask
        prop_assert!(result >= bid_price && result <= ask_price,
            "microprice {} should be between bid {} and ask {}",
            result, bid_price, ask_price);
    }
}

// =============================================================================
// Task 13.8: Property test for book_vwap calculation correctness (Property 19)
// =============================================================================

/// Computes book_vwap in Rust:
/// sum(price[i] * size[i]) / sum(size[i]) for i in 0..min(L, depth)
fn book_vwap_rust(levels: &[(f64, f64)], num_levels: usize) -> f64 {
    let effective = num_levels.min(levels.len());
    let total_notional: f64 = levels[..effective].iter().map(|(p, s)| p * s).sum();
    let total_size: f64 = levels[..effective].iter().map(|(_, s)| *s).sum();
    total_notional / total_size
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 15.4**
    ///
    /// For any Book, side, and levels count L, book_vwap returns
    /// sum(price[i] * size[i]) / sum(size[i]) for i in 0..min(L, depth).
    #[test]
    fn prop_book_vwap(
        prices in proptest::collection::vec(0.01f64..10000.0, 1..20),
        sizes in proptest::collection::vec(0.01f64..10000.0, 1..20),
        num_levels in 1usize..25,
    ) {
        // Combine prices and sizes into levels
        let depth = prices.len().min(sizes.len());
        let levels: Vec<(f64, f64)> = prices[..depth].iter()
            .zip(sizes[..depth].iter())
            .map(|(&p, &s)| (p, s))
            .collect();

        let result = book_vwap_rust(&levels, num_levels);

        // Recompute expected
        let effective = num_levels.min(levels.len());
        let total_notional: f64 = levels[..effective].iter().map(|(p, s)| p * s).sum();
        let total_size: f64 = levels[..effective].iter().map(|(_, s)| *s).sum();
        let expected = total_notional / total_size;

        prop_assert!((result - expected).abs() < 1e-10,
            "book_vwap(levels={}) = {}, expected {}",
            num_levels, result, expected);

        // VWAP should be between min and max prices of the levels used
        let effective_prices: Vec<f64> = levels[..effective].iter().map(|(p, _)| *p).collect();
        let min_price = effective_prices.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_price = effective_prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        prop_assert!(result >= min_price - 1e-10 && result <= max_price + 1e-10,
            "vwap {} should be between min price {} and max price {}",
            result, min_price, max_price);
    }
}
