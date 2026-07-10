// Feature: flux-fmt-syntax-highlight, Property 7: Top-Level Block Separation
//!
//! Property: For any valid Flux strategy containing multiple top-level blocks
//! (params, state, on_bar), the formatted output SHALL have exactly one blank
//! line separating each pair of adjacent top-level blocks.
//!
//! **Validates: Requirements 3.5**

use proptest::prelude::*;

use flux_compiler::lexer::Span;
use flux_compiler::parser::ast::{
    Assignment, EventHandler, Expr, ExprKind,
    Param, ParamsBlock, Program, StateBlock, StateVar,
    StrategyItem, Stmt,
};
use flux_compiler::parser::ast::Strategy as FluxStrategy;
use flux_cli::formatter::Formatter;

// =============================================================================
// AST Generators — produce multi-block strategies for separation testing
// =============================================================================

/// Reserved keywords that cannot be used as identifiers in Flux.
const FLUX_KEYWORDS: &[&str] = &[
    "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
    "return", "from", "import", "and", "or", "not", "true", "false", "null",
    "in", "bar", "fn", "struct", "data", "connector",
];

/// Generate a valid Flux identifier (not a keyword).
fn arb_ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{1,8}"
        .prop_filter("must not be a keyword", |s| {
            !FLUX_KEYWORDS.contains(&s.as_str())
        })
}

/// Generate a simple expression (leaf nodes only).
fn arb_simple_expr() -> impl Strategy<Value = Expr> {
    prop_oneof![
        (1i64..1000).prop_map(|v| Expr {
            kind: ExprKind::IntLiteral(v),
            span: Span::new(0, 0),
        }),
        (1.0f64..1000.0).prop_map(|v| {
            let rounded = (v * 100.0).round() / 100.0;
            Expr {
                kind: ExprKind::FloatLiteral(rounded),
                span: Span::new(0, 0),
            }
        }),
        any::<bool>().prop_map(|v| Expr {
            kind: ExprKind::BoolLiteral(v),
            span: Span::new(0, 0),
        }),
        arb_ident().prop_map(|name| Expr {
            kind: ExprKind::Ident(name),
            span: Span::new(0, 0),
        }),
    ]
}

/// Generate a ParamsBlock with 1-3 parameters.
fn arb_params_block() -> impl Strategy<Value = StrategyItem> {
    proptest::collection::vec(
        (arb_ident(), arb_simple_expr()).prop_map(|(name, default_value)| Param {
            name,
            default_value,
            span: Span::new(0, 0),
        }),
        1..4,
    )
    .prop_map(|params| StrategyItem::ParamsBlock(ParamsBlock {
        params,
        span: Span::new(0, 0),
    }))
}

/// Generate a StateBlock with 1-3 state variables.
fn arb_state_block() -> impl Strategy<Value = StrategyItem> {
    proptest::collection::vec(
        (arb_ident(), arb_simple_expr()).prop_map(|(name, initial_value)| StateVar {
            name,
            initial_value,
            span: Span::new(0, 0),
        }),
        1..4,
    )
    .prop_map(|variables| StrategyItem::StateBlock(StateBlock {
        variables,
        span: Span::new(0, 0),
    }))
}

/// Generate an EventHandler (on bar) with 1-3 simple statements.
fn arb_on_bar_block() -> impl Strategy<Value = StrategyItem> {
    proptest::collection::vec(
        (arb_ident(), arb_simple_expr()).prop_map(|(name, value)| {
            Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident(name),
                    span: Span::new(0, 0),
                },
                value,
                span: Span::new(0, 0),
            })
        }),
        1..4,
    )
    .prop_map(|body| StrategyItem::EventHandler(EventHandler {
        event_name: "bar".to_string(),
        body,
        span: Span::new(0, 0),
    }))
}

/// Generate a multi-block strategy with at least 2 top-level blocks.
/// Combinations: params+on_bar, state+on_bar, params+state+on_bar.
fn arb_multi_block_program() -> impl Strategy<Value = Program> {
    let blocks_strategy = prop_oneof![
        // params + on_bar (2 blocks)
        (arb_params_block(), arb_on_bar_block())
            .prop_map(|(p, o)| vec![p, o]),
        // state + on_bar (2 blocks)
        (arb_state_block(), arb_on_bar_block())
            .prop_map(|(s, o)| vec![s, o]),
        // params + state + on_bar (3 blocks)
        (arb_params_block(), arb_state_block(), arb_on_bar_block())
            .prop_map(|(p, s, o)| vec![p, s, o]),
        // params + state (2 blocks, different combination)
        (arb_params_block(), arb_state_block())
            .prop_map(|(p, s)| vec![p, s]),
    ];

    (
        "[A-Z][a-zA-Z]{2,10}".prop_filter("must not be a keyword", |s| {
            !FLUX_KEYWORDS.contains(&s.to_lowercase().as_str())
        }),
        blocks_strategy,
    )
        .prop_map(|(name, body)| Program {
            structs: vec![], enums: vec![],
            imports: Vec::new(),
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: FluxStrategy {
                name,
                body,
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        })
}

// =============================================================================
// Helper: Count blank lines between adjacent top-level blocks
// =============================================================================

/// Checks that between every pair of adjacent top-level blocks in the formatted
/// output, there is exactly one blank line.
///
/// Detection approach:
/// - Top-level block closers are `}` lines at indent level 1 (4 spaces).
/// - After such a `}`, count consecutive blank lines before the next non-blank line.
/// - The next non-blank line should be a top-level block opener.
/// - Between them, there should be exactly 1 blank line.
fn verify_block_separation(formatted: &str) -> Result<(), String> {
    let lines: Vec<&str> = formatted.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Detect a top-level block closer: exactly "    }" (4 spaces + closing brace)
        // This closes params, state, or on bar blocks inside the strategy body.
        if line.trim() == "}" && line.starts_with("    ") && !line.starts_with("        ") {
            // We found a closing brace at indent level 1.
            // Now check if there's another top-level block after it.
            let mut blank_count = 0;
            let mut j = i + 1;

            // Count consecutive blank lines
            while j < lines.len() && lines[j].trim().is_empty() {
                blank_count += 1;
                j += 1;
            }

            // If we've reached end of file or the strategy closing brace, skip
            if j >= lines.len() {
                break;
            }

            let next_line = lines[j];

            // Check if the next non-blank line is another top-level block opener
            // (at indent level 1: 4 spaces) or the strategy closing brace (0 indent "}")
            let is_top_level_opener = next_line.starts_with("    ")
                && !next_line.starts_with("        ")
                && (next_line.trim().starts_with("params ")
                    || next_line.trim().starts_with("params{")
                    || next_line.trim().starts_with("state ")
                    || next_line.trim().starts_with("state{")
                    || next_line.trim().starts_with("on "));

            if is_top_level_opener {
                if blank_count != 1 {
                    return Err(format!(
                        "Expected exactly 1 blank line between top-level blocks, \
                         found {} blank lines between line {} ('{}') and line {} ('{}')",
                        blank_count,
                        i + 1,
                        line,
                        j + 1,
                        next_line,
                    ));
                }
            }
        }

        i += 1;
    }

    Ok(())
}

// =============================================================================
// Property Test: Top-Level Block Separation
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 3.5**
    ///
    /// For any valid Flux strategy containing multiple top-level blocks (params,
    /// state, on_bar), the formatted output has exactly one blank line separating
    /// each pair of adjacent top-level blocks.
    #[test]
    fn prop_top_level_block_separation(program in arb_multi_block_program()) {
        // Format the generated AST (no comments — empty source)
        let formatted = Formatter::format(&program, "");

        // Verify that the program has at least 2 strategy items
        // (guaranteed by our generator, but double-check)
        prop_assert!(
            program.strategy.body.len() >= 2,
            "Generated program must have at least 2 top-level blocks, got {}",
            program.strategy.body.len()
        );

        // Verify block separation
        match verify_block_separation(&formatted) {
            Ok(()) => {},
            Err(msg) => {
                prop_assert!(
                    false,
                    "Block separation property violated:\n{}\n\nFormatted output:\n{}",
                    msg,
                    formatted,
                );
            }
        }
    }
}
