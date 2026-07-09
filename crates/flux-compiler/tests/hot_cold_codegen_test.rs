//! Integration test: @hot/@cold field-level decorator codegen.
//!
//! Verifies the full pipeline: parse → typecheck → codegen emits split structs
//! for structs with @hot/@cold annotated fields.

use flux_compiler::compile;

/// A Flux source using @hot/@cold field decorators to annotate cache-sensitive fields.
const HOT_COLD_SOURCE: &str = r#"
struct MarketTick {
    @hot
    price: f64,
    @hot
    volume: f64,
    @cold
    exchange_id: int,
    @cold
    debug_info: int
}

strategy HotColdDemo {
    params {
        threshold = 1.0
    }

    on bar {
        if close > threshold {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

#[test]
fn hot_cold_field_decorators_produce_split_structs() {
    let result = compile(HOT_COLD_SOURCE);
    assert!(result.is_ok(), "compile failed: {:?}", result.err());
    let output = result.unwrap();

    // Original struct emitted for backward compatibility
    assert!(
        output.contains("pub struct MarketTick {"),
        "original struct missing from output"
    );

    // Hot sub-struct with cache-line alignment
    assert!(
        output.contains("#[repr(align(64))]"),
        "cache-line alignment attribute missing"
    );
    assert!(
        output.contains("pub struct MarketTick_Hot {"),
        "hot sub-struct missing"
    );

    // Cold sub-struct
    assert!(
        output.contains("pub struct MarketTick_Cold {"),
        "cold sub-struct missing"
    );

    // Split method
    assert!(
        output.contains("pub fn split(&self) -> (MarketTick_Hot, MarketTick_Cold)"),
        "split() method missing"
    );

    // Verify hot fields are in the hot struct
    let hot_start = output.find("pub struct MarketTick_Hot {").unwrap();
    let hot_end = output[hot_start..].find('}').unwrap() + hot_start;
    let hot_body = &output[hot_start..hot_end];
    assert!(hot_body.contains("pub price: f64,"), "hot struct missing price");
    assert!(
        hot_body.contains("pub volume: f64,"),
        "hot struct missing volume"
    );
    assert!(
        !hot_body.contains("exchange_id"),
        "hot struct should not contain cold field"
    );

    // Verify cold fields are in the cold struct
    let cold_start = output.find("pub struct MarketTick_Cold {").unwrap();
    let cold_end = output[cold_start..].find('}').unwrap() + cold_start;
    let cold_body = &output[cold_start..cold_end];
    assert!(
        cold_body.contains("pub exchange_id: i64,"),
        "cold struct missing exchange_id"
    );
    assert!(
        cold_body.contains("pub debug_info: i64,"),
        "cold struct missing debug_info"
    );
    assert!(
        !cold_body.contains("price"),
        "cold struct should not contain hot field"
    );
}

#[test]
fn struct_without_hot_cold_emits_no_split() {
    let source = r#"
struct PlainData {
    x: f64,
    y: f64
}

strategy NoSplit {
    params {
        n = 1
    }

    on bar {
        if close > 0.0 {
            OPEN(symbol, 1.0)
        }
    }
}
"#;
    let result = compile(source);
    assert!(result.is_ok(), "compile failed: {:?}", result.err());
    let output = result.unwrap();

    assert!(output.contains("pub struct PlainData {"), "struct missing");
    assert!(!output.contains("PlainData_Hot"), "unexpected hot split");
    assert!(!output.contains("PlainData_Cold"), "unexpected cold split");
}

/// Verify that a struct with only @hot fields emits the hot sub-struct and split_hot method.
#[test]
fn hot_only_fields_emit_split_hot() {
    let source = r#"
struct FastPath {
    @hot
    price: f64,
    @hot
    size: f64,
    meta: int
}

strategy HotOnly {
    params {
        n = 1
    }

    on bar {
        if close > 0.0 {
            OPEN(symbol, 1.0)
        }
    }
}
"#;
    let result = compile(source);
    assert!(result.is_ok(), "compile failed: {:?}", result.err());
    let output = result.unwrap();

    assert!(
        output.contains("pub struct FastPath_Hot {"),
        "hot sub-struct missing"
    );
    assert!(
        !output.contains("FastPath_Cold"),
        "should not emit cold sub-struct when no @cold fields"
    );
    assert!(
        output.contains("pub fn split_hot(&self) -> FastPath_Hot"),
        "split_hot method missing"
    );
}
