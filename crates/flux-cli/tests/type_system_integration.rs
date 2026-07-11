//! End-to-end integration tests for the Flux type system.
//!
//! These tests exercise complete Flux programs through the full compiler pipeline:
//! - Interpret path: source → lex → parse → typecheck → interpret (backtest evaluation)
//! - Codegen path: source → lex → parse → typecheck → codegen (Rust code emission)
//!
//! Each test verifies that new type system features (enums, match, impl blocks, traits,
//! generics, HashMap) work correctly across all compiler stages.
//!
//! **Validates: Requirements 1.2, 1.3, 2.1, 2.2, 3.2, 3.9, 4.2, 4.9, 5.2, 6.1, 6.6**

use flux_cli::interpreter::Interpreter;
use flux_compiler::compile;
use flux_compiler::lexer;
use flux_compiler::parser;
use flux_compiler::typeck;
use flux_runtime::{BarContext, Signal};

// =============================================================================
// Helpers
// =============================================================================

/// Compile source through lex → parse → typecheck, returning an Interpreter.
fn compile_to_interpreter(source: &str) -> Interpreter {
    let tokens = lexer::lex_with_spans(source).expect("lexer failed");
    let ast = parser::parse(tokens).expect("parser failed");
    let typed_program = typeck::check(ast).expect("typechecker failed");
    Interpreter::new(&typed_program)
}

/// Create a BarContext with specified values.
fn bar(symbol: &str, close: f64, open: f64) -> BarContext {
    BarContext {
        symbol: symbol.to_string(),
        close,
        open,
        high: close + 1.0,
        low: open - 1.0,
        volume: 1000.0,
        in_position: false,
    }
}

/// Create a simple BarContext for quick tests.
fn simple_bar(symbol: &str, close: f64) -> BarContext {
    BarContext {
        symbol: symbol.to_string(),
        close,
        open: close - 1.0,
        high: close + 1.0,
        low: close - 2.0,
        volume: 10000.0,
        in_position: false,
    }
}

// =============================================================================
// SECTION 1: Enum + Match Integration Tests
// =============================================================================

// =============================================================================
// Test 1: Enum definition + match expression — full interpret pipeline
// Validates: Requirements 1.2, 1.3, 2.1, 2.2, 3.2, 3.9
// =============================================================================

/// A complete Flux program that:
/// 1. Defines an enum `OrderType` with a unit variant (Market) and data variant (Limit)
/// 2. Constructs enum values
/// 3. Uses `match` to branch on variants and produce trading signals
/// 4. Runs through parse → typecheck → interpret and produces correct signals
#[test]
fn e2e_enum_match_produces_correct_signals_market_variant() {
    let source = r#"
enum OrderType {
    Market,
    Limit(price: f64)
}

strategy EnumMatchStrategy {
    on bar {
        order = OrderType.Market

        match order {
            OrderType.Market => {
                OPEN(symbol, 100.0)
            }
            OrderType.Limit(p) => {
                OPEN(symbol, p)
            }
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = simple_bar("AAPL", 150.0);
    let signals = interp.on_bar(&ctx);

    // The Market arm should fire, producing OPEN("AAPL", 100.0)
    assert_eq!(
        signals.len(),
        1,
        "Expected 1 signal from Market arm, got {:?}",
        signals
    );
    match &signals[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!(
                (qty - 100.0).abs() < f64::EPSILON,
                "Expected qty=100.0, got {}",
                qty
            );
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

#[test]
fn e2e_enum_match_produces_correct_signals_limit_variant() {
    let source = r#"
enum OrderType {
    Market,
    Limit(price: f64)
}

strategy EnumMatchStrategy {
    on bar {
        order = OrderType.Limit(250.0)

        match order {
            OrderType.Market => {
                OPEN(symbol, 100.0)
            }
            OrderType.Limit(p) => {
                OPEN(symbol, p)
            }
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = simple_bar("TSLA", 200.0);
    let signals = interp.on_bar(&ctx);

    // The Limit arm should fire, binding p=250.0, producing OPEN("TSLA", 250.0)
    assert_eq!(
        signals.len(),
        1,
        "Expected 1 signal from Limit arm, got {:?}",
        signals
    );
    match &signals[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "TSLA");
            assert!(
                (qty - 250.0).abs() < f64::EPSILON,
                "Expected qty=250.0 (bound from Limit variant), got {}",
                qty
            );
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

// =============================================================================
// Test 2: Enum with multiple data fields — match binds all fields correctly
// Validates: Requirements 1.3, 2.2, 3.2, 3.9
// =============================================================================

#[test]
fn e2e_enum_match_binds_multiple_fields() {
    let source = r#"
enum TradeSignal {
    Buy(price: f64, qty: f64),
    Sell(price: f64, qty: f64),
    Hold
}

strategy MultiFieldEnum {
    on bar {
        sig = TradeSignal.Buy(close, 50.0)

        match sig {
            TradeSignal.Buy(p, q) => {
                OPEN(symbol, q)
            }
            TradeSignal.Sell(p, q) => {
                CLOSE(symbol)
            }
            TradeSignal.Hold => {
                CLOSE(symbol)
            }
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = simple_bar("MSFT", 300.0);
    let signals = interp.on_bar(&ctx);

    // Buy arm fires: binds p=300.0 (close), q=50.0 → OPEN("MSFT", 50.0)
    assert_eq!(
        signals.len(),
        1,
        "Expected 1 signal from Buy arm, got {:?}",
        signals
    );
    match &signals[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "MSFT");
            assert!(
                (qty - 50.0).abs() < f64::EPSILON,
                "Expected qty=50.0, got {}",
                qty
            );
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

// =============================================================================
// Test 3: Wildcard pattern in match expression
// Validates: Requirements 3.2, 3.9
// =============================================================================

#[test]
fn e2e_enum_match_wildcard_catches_unmatched_variants() {
    let source = r#"
enum Direction {
    Long,
    Short,
    Neutral
}

strategy WildcardMatch {
    on bar {
        dir = Direction.Neutral

        match dir {
            Direction.Long => {
                OPEN(symbol, 100.0)
            }
            _ => {
                CLOSE(symbol)
            }
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = simple_bar("SPY", 400.0);
    let signals = interp.on_bar(&ctx);

    // Neutral doesn't match Long, so wildcard fires → CLOSE("SPY")
    assert_eq!(
        signals.len(),
        1,
        "Expected 1 signal from wildcard arm, got {:?}",
        signals
    );
    match &signals[0] {
        Signal::Close { symbol } => {
            assert_eq!(symbol, "SPY");
        }
        other => panic!("Expected Close signal from wildcard, got {:?}", other),
    }
}

// =============================================================================
// Test 4: Match expression result used as a value
// Validates: Requirements 3.2, 3.9
// =============================================================================

#[test]
fn e2e_match_expression_returns_value_used_in_logic() {
    let source = r#"
enum Regime {
    Bull,
    Bear,
    Sideways
}

strategy MatchAsValue {
    on bar {
        regime = Regime.Bull

        qty = match regime {
            Regime.Bull => {
                200.0
            }
            Regime.Bear => {
                50.0
            }
            Regime.Sideways => {
                0.0
            }
        }

        if qty > 0.0 {
            OPEN(symbol, qty)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = simple_bar("AAPL", 180.0);
    let signals = interp.on_bar(&ctx);

    // Bull regime → qty=200.0 → OPEN("AAPL", 200.0)
    assert_eq!(
        signals.len(),
        1,
        "Expected 1 signal for Bull regime, got {:?}",
        signals
    );
    match &signals[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!(
                (qty - 200.0).abs() < f64::EPSILON,
                "Expected qty=200.0 for Bull, got {}",
                qty
            );
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

// =============================================================================
// Test 5: Codegen path — enum + match emits valid Rust code
// Validates: Requirements 1.2, 1.3, 2.1, 2.2, 3.2
// =============================================================================

#[test]
fn e2e_enum_match_codegen_emits_valid_rust() {
    let source = r#"
enum OrderType {
    Market,
    Limit(price: f64)
}

strategy CodegenEnum {
    on bar {
        order = OrderType.Market
        match order {
            OrderType.Market => {
                OPEN(symbol, 100.0)
            }
            OrderType.Limit(p) => {
                OPEN(symbol, p)
            }
        }
    }
}
"#;

    let output = compile(source).expect("full compilation (codegen) should succeed");

    // 1. The Rust output should contain the enum definition with derive attributes
    assert!(
        output.contains("#[derive(Debug, Clone, PartialEq)]"),
        "Generated Rust should include #[derive(Debug, Clone, PartialEq)]. Got:\n{}",
        output
    );

    assert!(
        output.contains("enum OrderType"),
        "Generated Rust should contain 'enum OrderType'. Got:\n{}",
        output
    );

    // 2. The enum should have a Market unit variant
    assert!(
        output.contains("Market"),
        "Generated Rust should contain Market variant. Got:\n{}",
        output
    );

    // 3. The enum should have a Limit struct variant with price field
    assert!(
        output.contains("Limit") && output.contains("price"),
        "Generated Rust should contain Limit variant with price field. Got:\n{}",
        output
    );

    // 4. Enum construction should use Rust :: syntax
    assert!(
        output.contains("OrderType::Market") || output.contains("OrderType::Limit"),
        "Generated Rust should use :: for enum variant access. Got:\n{}",
        output
    );

    // 5. Match expression should be present
    assert!(
        output.contains("match"),
        "Generated Rust should contain a match expression. Got:\n{}",
        output
    );
}

// =============================================================================
// Test 6: Parse phase — enum and match parse into correct AST structure
// Validates: Requirements 1.2, 1.3, 2.1, 2.2, 3.2
// =============================================================================

#[test]
fn e2e_enum_match_parses_correctly() {
    let source = r#"
enum OrderType {
    Market,
    Limit(price: f64)
}

strategy ParseTest {
    on bar {
        order = OrderType.Limit(99.5)
        match order {
            OrderType.Market => {
                OPEN(symbol, 1.0)
            }
            OrderType.Limit(p) => {
                OPEN(symbol, p)
            }
        }
    }
}
"#;

    let tokens = lexer::lex_with_spans(source).expect("lexing should succeed");
    let ast = parser::parse(tokens).expect("parsing should succeed");

    // Verify enum is in the AST
    assert_eq!(ast.enums.len(), 1, "Expected 1 enum definition");
    assert_eq!(ast.enums[0].name, "OrderType");
    assert_eq!(ast.enums[0].variants.len(), 2, "Expected 2 variants");

    // Verify variant names
    assert_eq!(ast.enums[0].variants[0].name, "Market");
    assert_eq!(ast.enums[0].variants[1].name, "Limit");

    // Verify Market is a unit variant (no fields)
    assert!(
        ast.enums[0].variants[0].fields.is_empty(),
        "Market should be a unit variant"
    );

    // Verify Limit has one field named 'price' of type f64
    assert_eq!(ast.enums[0].variants[1].fields.len(), 1);
    assert_eq!(ast.enums[0].variants[1].fields[0].name, "price");
}

// =============================================================================
// Test 7: Typecheck phase — enum + match typechecks successfully
// Validates: Requirements 1.2, 1.3, 2.1, 2.2, 3.2
// =============================================================================

#[test]
fn e2e_enum_match_typechecks_successfully() {
    let source = r#"
enum OrderType {
    Market,
    Limit(price: f64)
}

strategy TypecheckTest {
    on bar {
        order = OrderType.Market
        match order {
            OrderType.Market => {
                OPEN(symbol, 100.0)
            }
            OrderType.Limit(p) => {
                OPEN(symbol, p)
            }
        }
    }
}
"#;

    let tokens = lexer::lex_with_spans(source).expect("lexing should succeed");
    let ast = parser::parse(tokens).expect("parsing should succeed");
    let typed_program = typeck::check(ast);

    assert!(
        typed_program.is_ok(),
        "Typechecking should succeed for valid enum + match program. Error: {:?}",
        typed_program.err()
    );

    let typed = typed_program.unwrap();
    // Verify enum is registered in the typed program
    assert_eq!(typed.enums.len(), 1);
    assert_eq!(typed.enums[0].name, "OrderType");
}

// =============================================================================
// Test 8: Multiple bars — conditional logic that produces different variants
// Validates: Requirements 2.1, 2.2, 3.9
// =============================================================================

#[test]
fn e2e_enum_match_across_multiple_bars() {
    let source = r#"
enum OrderType {
    Market,
    Limit(price: f64)
}

strategy MultiBarEnum {
    on bar {
        if close > 100.0 {
            order = OrderType.Market
            match order {
                OrderType.Market => {
                    OPEN(symbol, 100.0)
                }
                OrderType.Limit(p) => {
                    OPEN(symbol, p)
                }
            }
        } else {
            order = OrderType.Limit(close)
            match order {
                OrderType.Market => {
                    OPEN(symbol, 100.0)
                }
                OrderType.Limit(p) => {
                    OPEN(symbol, p)
                }
            }
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // Bar 1: close=150 > 100 → Market → OPEN(symbol, 100.0)
    let ctx1 = simple_bar("AAPL", 150.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(signals1.len(), 1);
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!((qty - 100.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal for Market, got {:?}", other),
    }

    // Bar 2: close=80 <= 100 → Limit(80.0) → OPEN(symbol, 80.0)
    let ctx2 = simple_bar("AAPL", 80.0);
    let signals2 = interp.on_bar(&ctx2);
    assert_eq!(signals2.len(), 1);
    match &signals2[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!(
                (qty - 80.0).abs() < f64::EPSILON,
                "Expected qty=80.0 (Limit price), got {}",
                qty
            );
        }
        other => panic!("Expected Open signal for Limit, got {:?}", other),
    }
}

// =============================================================================
// SECTION 2: Impl Blocks + Trait Polymorphism Integration Tests
// =============================================================================

// =============================================================================
// Test: Impl blocks + trait polymorphism (full pipeline)
// Validates: Requirements 4.2, 4.9, 5.2, 6.1, 6.6
// =============================================================================

/// End-to-end test: struct with impl block methods, trait definition,
/// trait implementation, and method calls through the interpreter.
#[test]
fn e2e_impl_blocks_and_trait_polymorphism_interpret() {
    let source = r#"
struct OrderBook {
    best_bid: f64,
    best_ask: f64
}

impl OrderBook {
    fn spread(self) -> f64 {
        return self.best_ask - self.best_bid
    }

    fn mid_price(self) -> f64 {
        return (self.best_bid + self.best_ask) / 2.0
    }
}

trait Priced {
    fn price(self) -> f64
}

impl Priced for OrderBook {
    fn price(self) -> f64 {
        return self.best_bid
    }
}

strategy SpreadTrader {
    params {
        max_spread = 0.5
    }

    on bar {
        book = OrderBook { best_bid = close - 0.1, best_ask = close + 0.1 }
        spread = book.spread()
        mid = book.mid_price()
        p = book.price()

        if spread < max_spread and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // Bar with close=100.0 → spread is 0.2 (< 0.5 threshold) → should open
    let ctx1 = bar("AAPL", 100.0, 99.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(
        signals1.len(),
        1,
        "Expected 1 OPEN signal when spread < max_spread, got {:?}",
        signals1
    );
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!((qty - 100.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

/// End-to-end test: verify codegen emits correct Rust code for impl blocks and traits.
///
/// Validates: Requirements 4.2, 4.11, 5.2, 6.1, 6.7
#[test]
fn e2e_impl_blocks_and_trait_polymorphism_codegen() {
    let source = r#"
struct OrderBook {
    best_bid: f64,
    best_ask: f64
}

impl OrderBook {
    fn spread(self) -> f64 {
        return self.best_ask - self.best_bid
    }

    fn mid_price(self) -> f64 {
        return (self.best_bid + self.best_ask) / 2.0
    }
}

trait Priced {
    fn price(self) -> f64
}

impl Priced for OrderBook {
    fn price(self) -> f64 {
        return self.best_bid
    }
}

strategy SpreadTrader {
    params {
        max_spread = 0.5
    }

    on bar {
        book = OrderBook { best_bid = close - 0.1, best_ask = close + 0.1 }
        spread = book.spread()

        if spread < max_spread and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let output = compile(source).expect("full compilation should succeed");

    // Verify impl block is emitted
    assert!(
        output.contains("impl OrderBook"),
        "Generated code should contain 'impl OrderBook'. Got:\n{}",
        output
    );

    // Verify methods are emitted
    assert!(
        output.contains("fn spread(") || output.contains("fn spread(&self"),
        "Generated code should contain spread method. Got:\n{}",
        output
    );
    assert!(
        output.contains("fn mid_price(") || output.contains("fn mid_price(&self"),
        "Generated code should contain mid_price method. Got:\n{}",
        output
    );

    // Verify trait definition is emitted
    assert!(
        output.contains("trait Priced"),
        "Generated code should contain 'trait Priced'. Got:\n{}",
        output
    );

    // Verify trait impl is emitted
    assert!(
        output.contains("impl Priced for OrderBook"),
        "Generated code should contain 'impl Priced for OrderBook'. Got:\n{}",
        output
    );

    // Verify struct definition is present
    assert!(
        output.contains("struct OrderBook"),
        "Generated code should contain 'struct OrderBook'. Got:\n{}",
        output
    );
}

/// End-to-end test: trait method dispatch resolves correctly when both inherent
/// and trait methods exist on the same struct.
///
/// Validates: Requirements 4.9, 6.6 (method resolution priority)
#[test]
fn e2e_trait_method_dispatch_resolves_correctly() {
    let source = r#"
struct Instrument {
    value: f64
}

impl Instrument {
    fn inherent_value(self) -> f64 {
        return self.value * 2.0
    }
}

trait Valued {
    fn get_value(self) -> f64
}

impl Valued for Instrument {
    fn get_value(self) -> f64 {
        return self.value
    }
}

strategy ValueStrategy {
    on bar {
        inst = Instrument { value = close }
        v1 = inst.inherent_value()
        v2 = inst.get_value()

        if v1 > 200.0 and not in_position {
            OPEN(symbol, v2)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // close=150.0 → inherent_value=300.0 (> 200) → should open with qty=get_value()=150.0
    let ctx1 = bar("MSFT", 150.0, 149.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(
        signals1.len(),
        1,
        "Expected 1 OPEN signal when inherent_value > 200, got {:?}",
        signals1
    );
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "MSFT");
            assert!(
                (qty - 150.0).abs() < f64::EPSILON,
                "Expected qty=150.0 from trait method get_value(), got {}",
                qty
            );
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }

    // close=50.0 → inherent_value=100.0 (< 200) → no signal
    let ctx2 = bar("MSFT", 50.0, 49.0);
    let signals2 = interp.on_bar(&ctx2);
    assert!(
        signals2.is_empty(),
        "Expected no signals when inherent_value < 200, got {:?}",
        signals2
    );
}

// =============================================================================
// SECTION 3: Generics + HashMap Integration Tests
// =============================================================================

// =============================================================================
// Test: HashMap as a symbol registry — full interpret pipeline
// Validates: Requirements 7.1, 8.1, 9.1, 10.1, 10.9, 10.10
// =============================================================================

/// End-to-end test: HashMap[String, f64] as a symbol registry.
/// Creates a HashMap, inserts key-value pairs, retrieves values, and uses them
/// in trading logic to produce signals.
#[test]
fn e2e_hashmap_symbol_registry_interpret() {
    let source = r#"
strategy HashMapRegistry {
    on bar {
        registry = HashMap.new()
        registry = registry.insert("AAPL", 150.0)
        registry = registry.insert("GOOG", 2800.0)
        registry = registry.insert("TSLA", 700.0)

        target_price = registry.get("AAPL")

        if close > target_price and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // close=160.0 > target_price(150.0) → should open
    let ctx1 = simple_bar("AAPL", 160.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(
        signals1.len(),
        1,
        "Expected 1 OPEN signal when close > target_price, got {:?}",
        signals1
    );
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!((qty - 100.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }

    // close=140.0 < target_price(150.0) → no signal
    let ctx2 = simple_bar("AAPL", 140.0);
    let signals2 = interp.on_bar(&ctx2);
    assert!(
        signals2.is_empty(),
        "Expected no signals when close < target_price, got {:?}",
        signals2
    );
}

/// End-to-end test: HashMap contains_key check controls flow.
#[test]
fn e2e_hashmap_contains_key_controls_flow() {
    let source = r#"
strategy HashMapContainsKey {
    on bar {
        registry = HashMap.new()
        registry = registry.insert("AAPL", 100.0)

        if registry.contains_key(symbol) and not in_position {
            qty = registry.get(symbol)
            OPEN(symbol, qty)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // symbol="AAPL" is in registry → should open with qty=100.0
    let ctx1 = simple_bar("AAPL", 200.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(
        signals1.len(),
        1,
        "Expected 1 OPEN signal for known symbol, got {:?}",
        signals1
    );
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!((qty - 100.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }

    // symbol="MSFT" is NOT in registry → no signal
    let ctx2 = simple_bar("MSFT", 200.0);
    let signals2 = interp.on_bar(&ctx2);
    assert!(
        signals2.is_empty(),
        "Expected no signals for unknown symbol, got {:?}",
        signals2
    );
}

/// End-to-end test: generic function with trait-bounded generics.
/// Defines a trait, implements it for a struct, and calls a trait-bounded generic
/// function that processes the value.
///
/// Validates: Requirements 7.1, 8.1, 9.1
#[test]
fn e2e_generic_function_with_trait_bound_interpret() {
    let source = r#"
trait Priced {
    fn get_price(self) -> f64
}

struct Stock {
    price: f64
}

impl Priced for Stock {
    fn get_price(self) -> f64 {
        return self.price
    }
}

fn compute_signal[T: Priced](asset: T) -> f64 {
    return 1.0
}

strategy GenericTraitStrategy {
    on bar {
        stock = Stock { price = close }
        signal_val = compute_signal(stock)

        if signal_val > 0.0 and not in_position {
            OPEN(symbol, stock.get_price())
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // close=250.0 → stock.get_price()=250.0, signal_val=1.0 > 0 → OPEN(symbol, 250.0)
    let ctx1 = simple_bar("NVDA", 250.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(
        signals1.len(),
        1,
        "Expected 1 OPEN signal from generic trait-bounded function, got {:?}",
        signals1
    );
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "NVDA");
            assert!(
                (qty - 250.0).abs() < f64::EPSILON,
                "Expected qty=250.0 from get_price(), got {}",
                qty
            );
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

/// End-to-end test: codegen path for HashMap usage emits std::collections::HashMap.
///
/// Validates: Requirements 10.1, 10.10
#[test]
fn e2e_hashmap_codegen_emits_valid_rust() {
    let source = r#"
strategy HashMapCodegen {
    on bar {
        registry = HashMap.new()
        registry = registry.insert("AAPL", 150.0)
        registry = registry.insert("GOOG", 2800.0)
        price = registry.get("AAPL")

        if price > 100.0 and not in_position {
            OPEN(symbol, price)
        }
    }
}
"#;

    let output = compile(source).expect("HashMap codegen should succeed");

    // 1. Should emit std::collections::HashMap::new()
    assert!(
        output.contains("std::collections::HashMap::new()"),
        "Generated Rust should contain 'std::collections::HashMap::new()'. Got:\n{}",
        output
    );

    // 2. Should emit .insert() calls
    assert!(
        output.contains(".insert("),
        "Generated Rust should contain '.insert(' for HashMap insertions. Got:\n{}",
        output
    );

    // 3. Should emit .get() call (with & reference for key)
    assert!(
        output.contains(".get("),
        "Generated Rust should contain '.get(' for HashMap retrieval. Got:\n{}",
        output
    );
}

/// End-to-end test: codegen path for generic functions with trait bounds.
///
/// Validates: Requirements 7.1, 8.1, 9.1
#[test]
fn e2e_generic_trait_bound_codegen_emits_valid_rust() {
    let source = r#"
trait Priced {
    fn get_price(self) -> f64
}

struct Stock {
    price: f64
}

impl Priced for Stock {
    fn get_price(self) -> f64 {
        return self.price
    }
}

fn compute_signal[T: Priced](asset: T) -> f64 {
    return 1.0
}

strategy GenericCodegen {
    on bar {
        stock = Stock { price = close }
        sig = compute_signal(stock)

        if sig > 0.0 and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let output = compile(source).expect("generic trait-bound codegen should succeed");

    // 1. Should emit trait definition
    assert!(
        output.contains("trait Priced"),
        "Generated Rust should contain 'trait Priced'. Got:\n{}",
        output
    );

    // 2. Should emit generic function with angle-bracket syntax and trait bound
    assert!(
        output.contains("<T: Priced>") || output.contains("<T : Priced>"),
        "Generated Rust should contain '<T: Priced>' for trait-bounded generic. Got:\n{}",
        output
    );

    // 3. Should emit impl Priced for Stock
    assert!(
        output.contains("impl Priced for Stock"),
        "Generated Rust should contain 'impl Priced for Stock'. Got:\n{}",
        output
    );

    // 4. Should emit a generic function definition
    assert!(
        output.contains("fn compute_signal"),
        "Generated Rust should contain 'fn compute_signal'. Got:\n{}",
        output
    );
}

/// End-to-end test: parse + typecheck succeeds for a program combining HashMap and generics.
///
/// Validates: Requirements 7.1, 8.1, 9.1, 10.1
#[test]
fn e2e_hashmap_and_generics_typecheck_succeeds() {
    let source = r#"
trait Valued {
    fn value(self) -> f64
}

struct Asset {
    name: str,
    price: f64
}

impl Valued for Asset {
    fn value(self) -> f64 {
        return self.price
    }
}

fn get_value[T: Valued](item: T) -> f64 {
    return 1.0
}

strategy CombinedTest {
    on bar {
        registry = HashMap.new()
        registry = registry.insert("AAPL", 150.0)
        registry = registry.insert("GOOG", 2800.0)

        asset = Asset { name = "AAPL", price = close }
        v = get_value(asset)
        target = registry.get("AAPL")

        if close > target and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let tokens = lexer::lex_with_spans(source).expect("lexing should succeed");
    let ast = parser::parse(tokens).expect("parsing should succeed");
    let typed_program = typeck::check(ast);

    assert!(
        typed_program.is_ok(),
        "Typechecking should succeed for combined HashMap + generics program. Error: {:?}",
        typed_program.err()
    );
}

// =============================================================================
// End of Section 3
// =============================================================================

/// End-to-end test: impl method that accesses multiple struct fields.
///
/// Validates: Requirements 4.2, 4.9 (impl block method body with field access)
#[test]
fn e2e_impl_method_accesses_struct_fields() {
    let source = r#"
struct PriceLevel {
    price: f64,
    quantity: f64
}

impl PriceLevel {
    fn notional(self) -> f64 {
        return self.price * self.quantity
    }
}

strategy NotionalStrategy {
    params {
        min_notional = 5000.0
    }

    on bar {
        level = PriceLevel { price = close, quantity = 100.0 }
        n = level.notional()

        if n > min_notional and not in_position {
            OPEN(symbol, 50.0)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // close=60.0 → notional = 60*100 = 6000 > 5000 → open
    let ctx1 = bar("GOOG", 60.0, 59.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(
        signals1.len(),
        1,
        "Expected OPEN when notional > min_notional, got {:?}",
        signals1
    );

    // close=40.0 → notional = 40*100 = 4000 < 5000 → no signal
    let ctx2 = bar("GOOG", 40.0, 39.0);
    let signals2 = interp.on_bar(&ctx2);
    assert!(
        signals2.is_empty(),
        "Expected no signal when notional < min_notional, got {:?}",
        signals2
    );
}
