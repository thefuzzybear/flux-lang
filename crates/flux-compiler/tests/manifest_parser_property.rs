//! Property-based tests for the manifest parser.
//!
//! Feature: account-manifest
//!
//! Tests three correctness properties:
//! - Property 1: Manifest Parse Round-Trip
//! - Property 2: Span Validity
//! - Property 3: Parser Rejects Invalid Manifests

use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::parse_manifest;
use flux_compiler::parser::ast::{
    ManifestProgram, ManifestBlockKind, ManifestField, ManifestEntry, ManifestValue,
};
use proptest::prelude::*;

// ============================================================================
// Generators
// ============================================================================

/// Generate a valid manifest string value (no special chars to keep things simple).
fn arb_manifest_string() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_/.:-]{1,30}"
}

/// Generate a positive integer value for manifest fields.
fn arb_positive_int() -> impl Strategy<Value = i64> {
    1i64..100_000
}

/// Generate a positive float value for manifest fields.
fn arb_positive_float() -> impl Strategy<Value = f64> {
    (1u32..100_000u32).prop_map(|n| n as f64 + 0.5)
}

/// Generate a negative float value (for risk fields like max_daily_loss).
fn arb_negative_float() -> impl Strategy<Value = f64> {
    (1u32..100_000u32).prop_map(|n| -(n as f64 + 0.5))
}

/// Generate a list of 1-4 unique symbols (uppercase, short strings).
fn arb_symbols() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec("[A-Z]{2,4}", 1..=4)
        .prop_filter("unique symbols", |syms| {
            let mut seen = std::collections::HashSet::new();
            syms.iter().all(|s| seen.insert(s.clone()))
        })
}

/// Format a float for source output, ensuring it always has a decimal point.
fn format_float_for_source(f: f64) -> String {
    let s = format!("{}", f);
    if s.contains('.') {
        s
    } else {
        format!("{}.0", s)
    }
}

// ============================================================================
// Source generators (generate valid manifest source strings)
// ============================================================================

/// Generate a complete valid manifest source string along with expected field values.
/// Uses nested tuples to avoid exceeding proptest's 12-element limit.
fn arb_manifest_source() -> impl Strategy<Value = String> {
    (
        // Group 1: account + gateway (5 elements)
        (
            arb_manifest_string(),  // account name
            arb_manifest_string(),  // broker
            arb_manifest_string(),  // mode
            arb_manifest_string(),  // gateway host
            arb_positive_int(),     // gateway port
        ),
        // Group 2: data block (3 elements)
        (
            arb_manifest_string(),  // data source
            arb_symbols(),          // data symbols
            arb_manifest_string(),  // data interval
        ),
        // Group 3: database + risk (5 elements)
        (
            arb_manifest_string(),  // database url
            arb_manifest_string(),  // database schema
            arb_negative_float(),   // risk max_daily_loss
            arb_positive_float(),   // risk initial_equity
            arb_positive_int(),     // risk max_position_per_product
        ),
        // Group 4: products + strategies (3 elements)
        (
            arb_positive_float(),   // product multiplier
            arb_manifest_string(),  // strategy path
            arb_positive_float(),   // strategy allocation
        ),
    )
        .prop_map(|(
            (acc_name, broker, mode, gw_host, gw_port),
            (data_source, data_symbols, data_interval),
            (db_url, db_schema, max_daily_loss, initial_equity, max_pos),
            (prod_mult, strat_path, strat_alloc),
        )| {
            let symbols_str = data_symbols
                .iter()
                .map(|s| format!("\"{}\"", s))
                .collect::<Vec<_>>()
                .join(", ");

            format!(
                r#"account {{
    name = "{acc_name}"
    broker = "{broker}"
    mode = "{mode}"
}}

gateway {{
    host = "{gw_host}"
    port = {gw_port}
}}

data {{
    source = "{data_source}"
    symbols = [{symbols_str}]
    interval = "{data_interval}"
}}

database {{
    url = "{db_url}"
    schema = "{db_schema}"
}}

risk {{
    max_daily_loss = {max_daily_loss}
    initial_equity = {initial_equity}
    max_position_per_product = {max_pos}
}}

products {{
    prod1 = {{ multiplier = {prod_mult} }}
}}

strategies {{
    strat1 = {{ path = "{strat_path}", allocation = {strat_alloc}, priority = 1 }}
}}
"#,
                acc_name = acc_name,
                broker = broker,
                mode = mode,
                gw_host = gw_host,
                gw_port = gw_port,
                data_source = data_source,
                symbols_str = symbols_str,
                data_interval = data_interval,
                db_url = db_url,
                db_schema = db_schema,
                max_daily_loss = format_float_for_source(max_daily_loss),
                initial_equity = format_float_for_source(initial_equity),
                max_pos = max_pos,
                prod_mult = format_float_for_source(prod_mult),
                strat_path = strat_path,
                strat_alloc = format_float_for_source(strat_alloc),
            )
        })
}

// ============================================================================
// Helpers
// ============================================================================

/// Pretty-print a ManifestProgram back to source text (local helper for round-trip tests).
///
/// This is a simplified formatter that emits valid manifest syntax from the AST directly,
/// used only for testing parse round-trip correctness.
fn pretty_print_manifest(program: &ManifestProgram) -> String {
    let mut out = String::new();
    let num_blocks = program.blocks.len();
    for (i, block) in program.blocks.iter().enumerate() {
        match &block.kind {
            ManifestBlockKind::Account(fields)
            | ManifestBlockKind::Gateway(fields)
            | ManifestBlockKind::Data(fields)
            | ManifestBlockKind::Database(fields)
            | ManifestBlockKind::Risk(fields) => {
                let name = block_kind_name(&block.kind);
                out.push_str(&format!("{} {{\n", name));
                for field in fields {
                    out.push_str(&format!("    {} = {}\n", field.name, format_value(&field.value)));
                }
                out.push_str("}\n");
            }
            ManifestBlockKind::Products(entries)
            | ManifestBlockKind::Strategies(entries) => {
                let name = block_kind_name(&block.kind);
                out.push_str(&format!("{} {{\n", name));
                for entry in entries {
                    let fields_str = entry
                        .fields
                        .iter()
                        .map(|f| format!("{} = {}", f.name, format_value(&f.value)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("    {} = {{ {} }}\n", entry.name, fields_str));
                }
                out.push_str("}\n");
            }
        }
        if i < num_blocks - 1 {
            out.push('\n');
        }
    }
    out
}

/// Get the block keyword name for a ManifestBlockKind.
fn block_kind_name(kind: &ManifestBlockKind) -> &'static str {
    match kind {
        ManifestBlockKind::Account(_) => "account",
        ManifestBlockKind::Gateway(_) => "gateway",
        ManifestBlockKind::Data(_) => "data",
        ManifestBlockKind::Database(_) => "database",
        ManifestBlockKind::Risk(_) => "risk",
        ManifestBlockKind::Products(_) => "products",
        ManifestBlockKind::Strategies(_) => "strategies",
    }
}

/// Format a ManifestValue to valid source text.
fn format_value(value: &ManifestValue) -> String {
    match value {
        ManifestValue::String(s) => format!("\"{}\"", escape_string(s)),
        ManifestValue::Int(n) => format!("{}", n),
        ManifestValue::Float(f) => {
            let s = format!("{}", f);
            if s.contains('.') {
                s
            } else {
                format!("{}.0", s)
            }
        }
        ManifestValue::StringList(items) => {
            let items_str = items
                .iter()
                .map(|s| format!("\"{}\"", escape_string(s)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{}]", items_str)
        }
        ManifestValue::EnvCall(var) => format!("env(\"{}\")", var),
    }
}

/// Escape a string value for manifest output.
fn escape_string(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Helper to lex and parse manifest source.
fn parse_source(source: &str) -> Result<ManifestProgram, String> {
    let tokens = lex_with_spans(source).map_err(|e| format!("lex error: {}", e))?;
    parse_manifest(tokens).map_err(|e| format!("parse error: {}", e))
}

/// Walk all blocks and fields collecting spans.
fn collect_all_spans(program: &ManifestProgram) -> Vec<flux_compiler::lexer::Span> {
    let mut spans = Vec::new();
    spans.push(program.span);
    for block in &program.blocks {
        spans.push(block.span);
        match &block.kind {
            ManifestBlockKind::Account(fields)
            | ManifestBlockKind::Gateway(fields)
            | ManifestBlockKind::Data(fields)
            | ManifestBlockKind::Database(fields)
            | ManifestBlockKind::Risk(fields) => {
                for field in fields {
                    spans.push(field.span);
                }
            }
            ManifestBlockKind::Products(entries)
            | ManifestBlockKind::Strategies(entries) => {
                for entry in entries {
                    spans.push(entry.span);
                    for field in &entry.fields {
                        spans.push(field.span);
                    }
                }
            }
        }
    }
    spans
}

/// Compare two ManifestProgram ASTs for structural equivalence.
/// Block kinds must match, field names must match, values must match.
fn assert_structural_equivalence(original: &ManifestProgram, reparsed: &ManifestProgram) -> Result<(), String> {
    if original.blocks.len() != reparsed.blocks.len() {
        return Err(format!(
            "block count mismatch: original={}, reparsed={}",
            original.blocks.len(),
            reparsed.blocks.len()
        ));
    }

    for (i, (orig_block, reparse_block)) in original.blocks.iter().zip(reparsed.blocks.iter()).enumerate() {
        let orig_name = block_kind_name(&orig_block.kind);
        let reparse_name = block_kind_name(&reparse_block.kind);
        if orig_name != reparse_name {
            return Err(format!(
                "block {} kind mismatch: original={}, reparsed={}",
                i, orig_name, reparse_name
            ));
        }

        match (&orig_block.kind, &reparse_block.kind) {
            (ManifestBlockKind::Account(orig_fields), ManifestBlockKind::Account(re_fields))
            | (ManifestBlockKind::Gateway(orig_fields), ManifestBlockKind::Gateway(re_fields))
            | (ManifestBlockKind::Data(orig_fields), ManifestBlockKind::Data(re_fields))
            | (ManifestBlockKind::Database(orig_fields), ManifestBlockKind::Database(re_fields))
            | (ManifestBlockKind::Risk(orig_fields), ManifestBlockKind::Risk(re_fields)) => {
                assert_fields_equiv(orig_name, orig_fields, re_fields)?;
            }
            (ManifestBlockKind::Products(orig_entries), ManifestBlockKind::Products(re_entries))
            | (ManifestBlockKind::Strategies(orig_entries), ManifestBlockKind::Strategies(re_entries)) => {
                assert_entries_equiv(orig_name, orig_entries, re_entries)?;
            }
            _ => {
                return Err(format!("block {} has mismatched kind variants", i));
            }
        }
    }
    Ok(())
}

fn assert_fields_equiv(block_name: &str, orig: &[ManifestField], reparsed: &[ManifestField]) -> Result<(), String> {
    if orig.len() != reparsed.len() {
        return Err(format!(
            "field count mismatch in '{}': original={}, reparsed={}",
            block_name,
            orig.len(),
            reparsed.len()
        ));
    }
    for (j, (of, rf)) in orig.iter().zip(reparsed.iter()).enumerate() {
        if of.name != rf.name {
            return Err(format!(
                "field {} name mismatch in '{}': original='{}', reparsed='{}'",
                j, block_name, of.name, rf.name
            ));
        }
        if !values_equiv(&of.value, &rf.value) {
            return Err(format!(
                "field '{}' value mismatch in '{}': original={:?}, reparsed={:?}",
                of.name, block_name, of.value, rf.value
            ));
        }
    }
    Ok(())
}

fn assert_entries_equiv(block_name: &str, orig: &[ManifestEntry], reparsed: &[ManifestEntry]) -> Result<(), String> {
    if orig.len() != reparsed.len() {
        return Err(format!(
            "entry count mismatch in '{}': original={}, reparsed={}",
            block_name,
            orig.len(),
            reparsed.len()
        ));
    }
    for (j, (oe, re)) in orig.iter().zip(reparsed.iter()).enumerate() {
        if oe.name != re.name {
            return Err(format!(
                "entry {} name mismatch in '{}': original='{}', reparsed='{}'",
                j, block_name, oe.name, re.name
            ));
        }
        assert_fields_equiv(&format!("{}.{}", block_name, oe.name), &oe.fields, &re.fields)?;
    }
    Ok(())
}

/// Compare ManifestValues with f64 tolerance for floating point.
fn values_equiv(a: &ManifestValue, b: &ManifestValue) -> bool {
    match (a, b) {
        (ManifestValue::String(s1), ManifestValue::String(s2)) => s1 == s2,
        (ManifestValue::Int(n1), ManifestValue::Int(n2)) => n1 == n2,
        (ManifestValue::Float(f1), ManifestValue::Float(f2)) => {
            // Compare with tolerance for float round-trip through string formatting
            (f1 - f2).abs() < 1e-10 || f1 == f2
        }
        (ManifestValue::StringList(l1), ManifestValue::StringList(l2)) => l1 == l2,
        (ManifestValue::EnvCall(v1), ManifestValue::EnvCall(v2)) => v1 == v2,
        _ => false,
    }
}

// ============================================================================
// Property 1: Manifest Parse Round-Trip
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.10, 3.1, 3.2, 3.4, 8.1, 8.3**
    ///
    /// For any valid manifest source, parsing it, pretty-printing the AST back to source,
    /// and re-parsing that source SHALL produce a structurally equivalent AST.
    #[test]
    fn prop_manifest_parse_roundtrip(source in arb_manifest_source()) {
        // Parse the generated source
        let program = parse_source(&source)
            .map_err(|e| TestCaseError::Fail(format!("Initial parse failed: {}", e).into()))?;

        // Pretty-print AST back to source
        let formatted = pretty_print_manifest(&program);

        // Re-parse the formatted source
        let reparsed = parse_source(&formatted)
            .map_err(|e| TestCaseError::Fail(
                format!("Re-parse failed: {}\n\nFormatted source:\n{}", e, formatted).into()
            ))?;

        // Assert structural equivalence
        assert_structural_equivalence(&program, &reparsed)
            .map_err(|e| TestCaseError::Fail(
                format!("Structural mismatch: {}\n\nOriginal source:\n{}\n\nFormatted source:\n{}", e, source, formatted).into()
            ))?;
    }
}

// ============================================================================
// Property 2: Span Validity
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 1.8**
    ///
    /// For any valid manifest source that parses successfully, every span in the
    /// resulting AST has start < end, start >= 0, and end <= source.len().
    #[test]
    fn prop_manifest_span_validity(source in arb_manifest_source()) {
        let program = parse_source(&source)
            .map_err(|e| TestCaseError::Fail(format!("Parse failed: {}", e).into()))?;

        let source_len = source.len();
        let all_spans = collect_all_spans(&program);

        for (i, span) in all_spans.iter().enumerate() {
            prop_assert!(
                span.start < span.end,
                "Span {} has start ({}) >= end ({})",
                i, span.start, span.end
            );
            prop_assert!(
                span.end <= source_len,
                "Span {} has end ({}) > source.len() ({})",
                i, span.end, source_len
            );
        }
    }
}

// ============================================================================
// Property 3: Parser Rejects Invalid Manifests
// ============================================================================

/// Generate an identifier that is NOT in the valid block set.
fn arb_invalid_block_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{2,15}".prop_filter("must not be a valid block name", |s| {
        ![
            "account", "gateway", "data", "database", "risk", "products", "strategies",
        ]
        .contains(&s.as_str())
            && ![
                "strategy", "params", "state", "on", "if", "elif", "else", "for", "in",
                "while", "return", "fn", "from", "import", "and", "or", "not", "true",
                "false", "null", "connector", "struct", "enum", "match", "self", "impl",
                "trait", "env",
            ]
            .contains(&s.as_str())
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 1.9, 1.11, 3.5, 3.6**
    ///
    /// For any manifest source containing an unrecognized block keyword or a
    /// duplicated block of the same type, the parser SHALL return an error
    /// whose message contains the offending block name.
    #[test]
    fn prop_manifest_rejects_invalid_block(invalid_name in arb_invalid_block_name()) {
        // Test case A: unrecognized block keyword
        let source = format!(
            "{} {{\n    x = 1\n}}\n",
            invalid_name
        );
        let result = parse_source(&source);
        prop_assert!(
            result.is_err(),
            "Parser should reject unrecognized block '{}', but it parsed successfully",
            invalid_name
        );
        let err_msg = result.unwrap_err();
        prop_assert!(
            err_msg.contains(&invalid_name),
            "Error message should contain the offending block name '{}', got: {}",
            invalid_name, err_msg
        );
    }

    /// **Validates: Requirements 1.11**
    ///
    /// For any valid block name that appears twice, the parser SHALL return an
    /// error indicating the duplicate.
    #[test]
    fn prop_manifest_rejects_duplicate_block(
        block_idx in 0usize..5,  // pick one of the 5 simple blocks to duplicate
        field_val in arb_manifest_string(),
    ) {
        let block_names = ["account", "gateway", "database", "risk", "data"];
        let block_name = block_names[block_idx];

        // Build source with duplicate blocks
        let source = format!(
            "{name} {{\n    field1 = \"{val}\"\n}}\n\n{name} {{\n    field2 = \"{val}\"\n}}\n",
            name = block_name,
            val = field_val
        );
        let result = parse_source(&source);
        prop_assert!(
            result.is_err(),
            "Parser should reject duplicate '{}' block, but it parsed successfully",
            block_name
        );
        let err_msg = result.unwrap_err();
        prop_assert!(
            err_msg.contains(block_name),
            "Error message should contain the duplicate block name '{}', got: {}",
            block_name, err_msg
        );
        prop_assert!(
            err_msg.contains("duplicate"),
            "Error message should contain 'duplicate', got: {}",
            err_msg
        );
    }
}
