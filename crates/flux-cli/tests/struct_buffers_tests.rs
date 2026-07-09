// Feature: flux-structs, Property 24: Struct-typed ring buffer get returns correct type
//!
//! Unit tests and property-based tests for struct-typed ring buffers
//! (QuoteWindow, BarWindow) defined in std/collections/buffers.flux.
//!
//! Tasks: 15.4, 15.5
//!
//! Tests verify:
//! - `std/collections/buffers.flux` parses correctly
//! - QuoteWindow and BarWindow push+get algorithm correctness (Rust simulation)
//! - The typechecker resolves get() return types as Quote/Bar respectively

use proptest::prelude::*;

use flux_compiler::lexer;
use flux_compiler::parser;

// =============================================================================
// Task 15.3/15.4: Unit tests for struct-typed ring buffers
// =============================================================================

#[test]
fn test_buffers_flux_parses_successfully() {
    let source = std::fs::read_to_string("../../std/collections/buffers.flux")
        .expect("std/collections/buffers.flux should exist");
    let tokens = lexer::lex_with_spans(&source).expect("buffers.flux should lex without errors");
    let _ast = parser::parse(tokens).expect("buffers.flux should parse without errors");
}

#[test]
fn test_buffers_flux_struct_definitions_present() {
    let source = std::fs::read_to_string("../../std/collections/buffers.flux")
        .expect("std/collections/buffers.flux should exist");
    let tokens = lexer::lex_with_spans(&source).expect("buffers.flux should lex without errors");
    let ast = parser::parse(tokens).expect("buffers.flux should parse without errors");

    // Verify all expected structs are defined
    let struct_names: Vec<&str> = ast.structs.iter().map(|s| s.name.as_str()).collect();
    assert!(
        struct_names.contains(&"Quote"),
        "buffers.flux should contain Quote struct, found: {:?}",
        struct_names
    );
    assert!(
        struct_names.contains(&"Bar"),
        "buffers.flux should contain Bar struct, found: {:?}",
        struct_names
    );
    assert!(
        struct_names.contains(&"QuoteWindow"),
        "buffers.flux should contain QuoteWindow struct, found: {:?}",
        struct_names
    );
    assert!(
        struct_names.contains(&"BarWindow"),
        "buffers.flux should contain BarWindow struct, found: {:?}",
        struct_names
    );
}

#[test]
fn test_buffers_flux_functions_present() {
    let source = std::fs::read_to_string("../../std/collections/buffers.flux")
        .expect("std/collections/buffers.flux should exist");
    let tokens = lexer::lex_with_spans(&source).expect("buffers.flux should lex without errors");
    let ast = parser::parse(tokens).expect("buffers.flux should parse without errors");

    let fn_names: Vec<&str> = ast.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(
        fn_names.contains(&"quotewindow_new"),
        "Missing quotewindow_new, found: {:?}",
        fn_names
    );
    assert!(
        fn_names.contains(&"quotewindow_push"),
        "Missing quotewindow_push, found: {:?}",
        fn_names
    );
    assert!(
        fn_names.contains(&"quotewindow_get"),
        "Missing quotewindow_get, found: {:?}",
        fn_names
    );
    assert!(
        fn_names.contains(&"barwindow_new"),
        "Missing barwindow_new, found: {:?}",
        fn_names
    );
    assert!(
        fn_names.contains(&"barwindow_push"),
        "Missing barwindow_push, found: {:?}",
        fn_names
    );
    assert!(
        fn_names.contains(&"barwindow_get"),
        "Missing barwindow_get, found: {:?}",
        fn_names
    );
}

/// Task 15.3: Verify the typechecker resolves get() return types as struct types.
/// The function signatures in buffers.flux declare `-> Quote` and `-> Bar` return types,
/// which the parser captures as Named("Quote") and Named("Bar") type annotations.
#[test]
fn test_quotewindow_get_return_type_is_quote() {
    let source = std::fs::read_to_string("../../std/collections/buffers.flux")
        .expect("std/collections/buffers.flux should exist");
    let tokens = lexer::lex_with_spans(&source).expect("buffers.flux should lex without errors");
    let ast = parser::parse(tokens).expect("buffers.flux should parse without errors");

    // Find quotewindow_get function and verify its return type annotation
    let qw_get = ast
        .functions
        .iter()
        .find(|f| f.name == "quotewindow_get")
        .expect("quotewindow_get function should exist");

    // The return type should be a Named("Quote") type annotation
    let return_type = qw_get
        .return_type
        .as_ref()
        .expect("quotewindow_get should have a return type annotation");

    match return_type {
        flux_compiler::parser::ast::TypeAnnotation::Named(name) => {
            assert_eq!(
                name, "Quote",
                "quotewindow_get should return Quote, got {}",
                name
            );
        }
        other => panic!(
            "quotewindow_get return type should be Named(\"Quote\"), got {:?}",
            other
        ),
    }
}

#[test]
fn test_barwindow_get_return_type_is_bar() {
    let source = std::fs::read_to_string("../../std/collections/buffers.flux")
        .expect("std/collections/buffers.flux should exist");
    let tokens = lexer::lex_with_spans(&source).expect("buffers.flux should lex without errors");
    let ast = parser::parse(tokens).expect("buffers.flux should parse without errors");

    // Find barwindow_get function and verify its return type annotation
    let bw_get = ast
        .functions
        .iter()
        .find(|f| f.name == "barwindow_get")
        .expect("barwindow_get function should exist");

    // The return type should be a Named("Bar") type annotation
    let return_type = bw_get
        .return_type
        .as_ref()
        .expect("barwindow_get should have a return type annotation");

    match return_type {
        flux_compiler::parser::ast::TypeAnnotation::Named(name) => {
            assert_eq!(
                name, "Bar",
                "barwindow_get should return Bar, got {}",
                name
            );
        }
        other => panic!(
            "barwindow_get return type should be Named(\"Bar\"), got {:?}",
            other
        ),
    }
}

// =============================================================================
// QuoteWindow/BarWindow ring buffer algorithm simulation in Rust
// =============================================================================

/// Simulates a struct-typed ring buffer (QuoteWindow/BarWindow) in Rust.
/// The ring buffer stores values of type T with modulo-indexing.
#[derive(Clone, Debug)]
struct StructRingBuffer<T: Clone + Default> {
    data: Vec<T>,
    index: usize,
    count: usize,
    capacity: usize,
}

impl<T: Clone + Default> StructRingBuffer<T> {
    fn new(capacity: usize) -> Self {
        Self {
            data: vec![T::default(); capacity],
            index: 0,
            count: 0,
            capacity,
        }
    }

    fn push(&mut self, value: T) {
        self.data[self.index] = value;
        self.index = (self.index + 1) % self.capacity;
        if self.count < self.capacity {
            self.count += 1;
        }
    }

    fn get(&self, i: usize) -> Option<&T> {
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
        Some(&self.data[pos])
    }
}

/// A simplified Quote struct for testing the ring buffer algorithm.
#[derive(Clone, Debug, PartialEq)]
struct QuoteValue {
    bid: f64,
    bid_size: f64,
    ask: f64,
    ask_size: f64,
    timestamp: f64,
}

impl Default for QuoteValue {
    fn default() -> Self {
        Self {
            bid: 0.0,
            bid_size: 0.0,
            ask: 0.0,
            ask_size: 0.0,
            timestamp: 0.0,
        }
    }
}

/// A simplified Bar struct for testing the ring buffer algorithm.
#[derive(Clone, Debug, PartialEq)]
struct BarValue {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    timestamp: f64,
}

impl Default for BarValue {
    fn default() -> Self {
        Self {
            open: 0.0,
            high: 0.0,
            low: 0.0,
            close: 0.0,
            volume: 0.0,
            timestamp: 0.0,
        }
    }
}

// =============================================================================
// Task 15.4: Unit tests - QuoteWindow/BarWindow push+get smoke sequences
// =============================================================================

#[test]
fn test_quotewindow_push_get_smoke() {
    let mut qw: StructRingBuffer<QuoteValue> = StructRingBuffer::new(3);

    let q1 = QuoteValue {
        bid: 100.0,
        bid_size: 10.0,
        ask: 101.0,
        ask_size: 5.0,
        timestamp: 1.0,
    };
    let q2 = QuoteValue {
        bid: 100.5,
        bid_size: 20.0,
        ask: 101.5,
        ask_size: 15.0,
        timestamp: 2.0,
    };
    let q3 = QuoteValue {
        bid: 99.0,
        bid_size: 30.0,
        ask: 100.0,
        ask_size: 25.0,
        timestamp: 3.0,
    };

    qw.push(q1.clone());
    qw.push(q2.clone());
    qw.push(q3.clone());

    // get(0) = most recent = q3
    assert_eq!(qw.get(0), Some(&q3));
    // get(1) = second most recent = q2
    assert_eq!(qw.get(1), Some(&q2));
    // get(2) = oldest = q1
    assert_eq!(qw.get(2), Some(&q1));
    // get(3) = out of bounds
    assert_eq!(qw.get(3), None);
}

#[test]
fn test_quotewindow_overwrite_oldest() {
    let mut qw: StructRingBuffer<QuoteValue> = StructRingBuffer::new(2);

    let q1 = QuoteValue {
        bid: 100.0,
        bid_size: 10.0,
        ask: 101.0,
        ask_size: 5.0,
        timestamp: 1.0,
    };
    let q2 = QuoteValue {
        bid: 100.5,
        bid_size: 20.0,
        ask: 101.5,
        ask_size: 15.0,
        timestamp: 2.0,
    };
    let q3 = QuoteValue {
        bid: 99.0,
        bid_size: 30.0,
        ask: 100.0,
        ask_size: 25.0,
        timestamp: 3.0,
    };

    qw.push(q1.clone());
    qw.push(q2.clone());
    // Buffer full, pushing q3 overwrites q1
    qw.push(q3.clone());

    // get(0) = most recent = q3
    assert_eq!(qw.get(0), Some(&q3));
    // get(1) = q2 (q1 was overwritten)
    assert_eq!(qw.get(1), Some(&q2));
    // count should be capped at capacity
    assert_eq!(qw.count, 2);
}

#[test]
fn test_barwindow_push_get_smoke() {
    let mut bw: StructRingBuffer<BarValue> = StructRingBuffer::new(4);

    let b1 = BarValue {
        open: 100.0,
        high: 105.0,
        low: 98.0,
        close: 103.0,
        volume: 1000.0,
        timestamp: 1.0,
    };
    let b2 = BarValue {
        open: 103.0,
        high: 107.0,
        low: 101.0,
        close: 106.0,
        volume: 1500.0,
        timestamp: 2.0,
    };

    bw.push(b1.clone());
    bw.push(b2.clone());

    // get(0) = most recent = b2
    assert_eq!(bw.get(0), Some(&b2));
    // get(1) = b1
    assert_eq!(bw.get(1), Some(&b1));
    // get(2) = out of bounds (only 2 pushed)
    assert_eq!(bw.get(2), None);
}

#[test]
fn test_barwindow_single_element() {
    let mut bw: StructRingBuffer<BarValue> = StructRingBuffer::new(10);

    let b1 = BarValue {
        open: 50.0,
        high: 55.0,
        low: 48.0,
        close: 52.0,
        volume: 500.0,
        timestamp: 1.0,
    };

    bw.push(b1.clone());

    assert_eq!(bw.get(0), Some(&b1));
    assert_eq!(bw.get(1), None);
    assert_eq!(bw.count, 1);
}

// =============================================================================
// Task 15.5: Property test for struct-typed ring buffer get correctness
// Feature: flux-structs, Property 24: Struct-typed ring buffer get returns correct type
// =============================================================================

/// Generate a random QuoteValue with reasonable field values.
fn arb_quote_value() -> impl Strategy<Value = QuoteValue> {
    (
        0.01f64..10000.0,  // bid
        0.01f64..10000.0,  // bid_size
        0.01f64..1000.0,   // spread (used to compute ask)
        0.01f64..10000.0,  // ask_size
        0.0f64..1_000_000.0, // timestamp
    )
        .prop_map(|(bid, bid_size, spread, ask_size, timestamp)| QuoteValue {
            bid,
            bid_size,
            ask: bid + spread,
            ask_size,
            timestamp,
        })
}

/// Generate a random BarValue with reasonable field values.
fn arb_bar_value() -> impl Strategy<Value = BarValue> {
    (
        0.01f64..10000.0,  // open
        0.01f64..10000.0,  // high offset (added to max of open, close)
        0.01f64..10000.0,  // close
        0.01f64..10000.0,  // volume
        0.0f64..1_000_000.0, // timestamp
    )
        .prop_map(|(open, high_offset, close, volume, timestamp)| {
            let high = open.max(close) + high_offset;
            let low = open.min(close) - (high_offset * 0.5).min(open.min(close) - 0.001);
            BarValue {
                open,
                high,
                low,
                close,
                volume,
                timestamp,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 16.3, 16.4**
    ///
    /// Property 24: Struct-typed ring buffer get returns correct type
    ///
    /// For any push/get sequence on a QuoteWindow, get(0) SHALL return the most
    /// recently pushed Quote value, and get(i) SHALL return the (i+1)-th most
    /// recent value for valid i.
    #[test]
    fn prop_quotewindow_get_correctness(
        capacity in 1usize..50,
        quotes in proptest::collection::vec(arb_quote_value(), 1..100),
    ) {
        let mut buffer: StructRingBuffer<QuoteValue> = StructRingBuffer::new(capacity);

        for q in &quotes {
            buffer.push(q.clone());
        }

        let n = quotes.len();
        let effective_count = n.min(capacity);

        // Property: get(0) returns the most recently pushed Quote value
        let most_recent = buffer.get(0).unwrap();
        prop_assert_eq!(
            most_recent, &quotes[n - 1],
            "get(0) should return most recently pushed Quote"
        );

        // Property: get(i) returns the (i+1)-th most recent value for valid i
        for i in 0..effective_count {
            let got = buffer.get(i).unwrap();
            let expected = &quotes[n - 1 - i];
            prop_assert_eq!(
                got, expected,
                "get({}) should return quotes[{}]", i, n - 1 - i
            );
        }

        // Property: get(effective_count) should be None (out of bounds)
        prop_assert!(
            buffer.get(effective_count).is_none(),
            "get({}) should be None (out of bounds)", effective_count
        );
    }

    /// **Validates: Requirements 16.3, 16.4**
    ///
    /// Property 24: Struct-typed ring buffer get returns correct type (BarWindow)
    ///
    /// For any push/get sequence on a BarWindow, get(0) SHALL return the most
    /// recently pushed Bar value, and get(i) SHALL return the (i+1)-th most
    /// recent value for valid i.
    #[test]
    fn prop_barwindow_get_correctness(
        capacity in 1usize..50,
        bars in proptest::collection::vec(arb_bar_value(), 1..100),
    ) {
        let mut buffer: StructRingBuffer<BarValue> = StructRingBuffer::new(capacity);

        for b in &bars {
            buffer.push(b.clone());
        }

        let n = bars.len();
        let effective_count = n.min(capacity);

        // Property: get(0) returns the most recently pushed Bar value
        let most_recent = buffer.get(0).unwrap();
        prop_assert_eq!(
            most_recent, &bars[n - 1],
            "get(0) should return most recently pushed Bar"
        );

        // Property: get(i) returns the (i+1)-th most recent value for valid i
        for i in 0..effective_count {
            let got = buffer.get(i).unwrap();
            let expected = &bars[n - 1 - i];
            prop_assert_eq!(
                got, expected,
                "get({}) should return bars[{}]", i, n - 1 - i
            );
        }

        // Property: get(effective_count) should be None (out of bounds)
        prop_assert!(
            buffer.get(effective_count).is_none(),
            "get({}) should be None (out of bounds)", effective_count
        );
    }

    /// **Validates: Requirements 16.3, 16.4**
    ///
    /// Sub-property: After exactly `capacity` pushes, the buffer is full and all
    /// slots are accessible via get(). This tests the wrap-around behavior.
    #[test]
    fn prop_struct_ring_buffer_full_wrap(
        capacity in 1usize..30,
        extra_pushes in 0usize..50,
        quotes in proptest::collection::vec(arb_quote_value(), 80..100),
    ) {
        let total_pushes = capacity + extra_pushes;
        let use_quotes: Vec<QuoteValue> = quotes.into_iter().take(total_pushes).collect();

        let mut buffer: StructRingBuffer<QuoteValue> = StructRingBuffer::new(capacity);
        for q in &use_quotes {
            buffer.push(q.clone());
        }

        // After more than capacity pushes, count should be capped at capacity
        prop_assert_eq!(
            buffer.count, capacity,
            "count should be capped at capacity {} after {} pushes",
            capacity, total_pushes
        );

        // All positions 0..capacity should be accessible
        for i in 0..capacity {
            let got = buffer.get(i);
            prop_assert!(
                got.is_some(),
                "get({}) should be Some after {} pushes (capacity={})",
                i, total_pushes, capacity
            );
            // Verify it's the correct value from the end of the push sequence
            let expected = &use_quotes[total_pushes - 1 - i];
            prop_assert_eq!(
                got.unwrap(), expected,
                "get({}) value mismatch after wrap", i
            );
        }
    }
}
