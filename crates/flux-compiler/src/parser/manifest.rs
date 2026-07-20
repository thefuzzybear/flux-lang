//! Manifest parser for `account.flux` files.
//!
//! Parses the declarative block structure of manifest files into a
//! `ManifestProgram` AST. Block keywords are recognized by identifier
//! matching (or `Token::Data` for the `data` block).

use std::collections::HashSet;

use crate::error::{CompileError, Result};
use crate::lexer::{SpannedToken, Token};

use super::ast::*;
use super::parser_state::ParserState;

/// Parse a token stream as a manifest file (account.flux).
///
/// Dispatches on identifier tokens to parse known block types.
/// Returns error for unrecognized or duplicate blocks.
pub fn parse_manifest(tokens: Vec<SpannedToken>) -> Result<ManifestProgram> {
    let mut state = ParserState::new(tokens)?;
    parse_manifest_program(&mut state)
}

/// Valid manifest block keywords.
const VALID_BLOCKS: &[&str] = &[
    "account",
    "gateway",
    "data",
    "database",
    "risk",
    "products",
    "strategies",
];

/// Get the block keyword name from the current token.
///
/// Handles both `Token::Ident(name)` for most keywords and `Token::Data`
/// for the `data` block (since the lexer treats `data` as a reserved keyword).
fn block_keyword_name(state: &ParserState) -> Option<String> {
    match state.peek() {
        Token::Ident(name) => Some(name.clone()),
        Token::Data => Some("data".to_string()),
        _ => None,
    }
}

/// Parse the top-level manifest program structure.
fn parse_manifest_program(state: &mut ParserState) -> Result<ManifestProgram> {
    let start = state.current_span();
    let mut blocks = Vec::new();
    let mut seen_kinds: HashSet<String> = HashSet::new();

    while !state.at_eof() {
        let span = state.current_span();

        // Get the block keyword from the current token
        let name = match block_keyword_name(state) {
            Some(name) => name,
            None => {
                return Err(CompileError::Parser(format!(
                    "at byte {}: expected manifest block keyword, found {:?}",
                    span.start,
                    state.peek()
                )));
            }
        };

        // Check for duplicate blocks
        if seen_kinds.contains(&name) {
            return Err(CompileError::Parser(format!(
                "at byte {}: duplicate '{}' block (only one permitted per file)",
                span.start, name
            )));
        }

        // Check for recognized block keywords
        if !VALID_BLOCKS.contains(&name.as_str()) {
            return Err(CompileError::Parser(format!(
                "at byte {}: unrecognized manifest block '{}'. Valid blocks: account, gateway, data, database, risk, products, strategies",
                span.start, name
            )));
        }

        // Advance past the block keyword token
        state.advance();

        // Dispatch to the appropriate block parser
        let block = match name.as_str() {
            "account" | "gateway" | "data" | "database" | "risk" => {
                parse_simple_block(state, &name, span)?
            }
            "products" | "strategies" => {
                parse_entry_block(state, &name, span)?
            }
            _ => unreachable!(),
        };

        seen_kinds.insert(name);
        blocks.push(block);
    }

    Ok(ManifestProgram {
        blocks,
        span: state.span_from(start),
    })
}

/// Parse a simple key-value block: `name { key = value, ... }`
fn parse_simple_block(
    state: &mut ParserState,
    name: &str,
    start: crate::lexer::Span,
) -> Result<ManifestBlock> {
    // Expect opening brace
    state.expect(&Token::OpenBrace)?;

    let mut fields = Vec::new();

    // Loop until closing brace
    while !state.check(&Token::CloseBrace) {
        if state.at_eof() {
            return Err(CompileError::Parser(format!(
                "at byte {}: expected '}}' to close '{}' block",
                state.current_span().start, name
            )));
        }

        // Parse field name
        let (field_name, field_start) = state.expect_ident()?;

        // Expect `=`
        state.expect(&Token::Assign)?;

        // Parse value
        let (value, value_span) = parse_manifest_value(state)?;

        // Record span from field name start to value end
        let field_span = crate::lexer::Span::new(field_start.start, value_span.end);

        fields.push(ManifestField {
            name: field_name,
            value,
            span: field_span,
        });

        // Optional comma between fields
        if state.check(&Token::Comma) {
            state.advance();
        }
    }

    // Expect closing brace
    let close_span = state.expect(&Token::CloseBrace)?;

    // Map block name to ManifestBlockKind
    let kind = match name {
        "account" => ManifestBlockKind::Account(fields),
        "gateway" => ManifestBlockKind::Gateway(fields),
        "data" => ManifestBlockKind::Data(fields),
        "database" => ManifestBlockKind::Database(fields),
        "risk" => ManifestBlockKind::Risk(fields),
        _ => {
            return Err(CompileError::Parser(format!(
                "at byte {}: unrecognized simple block '{}'",
                start.start, name
            )));
        }
    };

    Ok(ManifestBlock {
        kind,
        span: crate::lexer::Span::new(start.start, close_span.end),
    })
}

/// Parse a manifest field value (string, int, float, string list, or env call).
///
/// Returns the parsed value and the span covering it.
/// Handles negative numbers (`-42`, `-3.14`), `env("VAR")`, and string lists.
fn parse_manifest_value(state: &mut ParserState) -> Result<(ManifestValue, crate::lexer::Span)> {
    let start = state.current_span();

    // Handle negative numbers: Token::Minus followed by Int or Float
    if state.check(&Token::Minus) {
        state.advance(); // consume `-`
        let current = state.peek_spanned().clone();
        match &current.token {
            Token::Int(n) => {
                let val = -n;
                state.advance();
                let span = crate::lexer::Span::new(start.start, current.span.end);
                Ok((ManifestValue::Int(val), span))
            }
            Token::Float(f) => {
                let val = -f;
                state.advance();
                let span = crate::lexer::Span::new(start.start, current.span.end);
                Ok((ManifestValue::Float(val), span))
            }
            _ => Err(CompileError::Parser(format!(
                "at byte {}: expected number after '-', found {:?}",
                start.start,
                state.peek()
            ))),
        }
    } else {
        let current = state.peek_spanned().clone();
        match &current.token {
            Token::String(s) => {
                let val = s.clone();
                let span = current.span;
                state.advance();
                Ok((ManifestValue::String(val), span))
            }
            Token::Int(n) => {
                let val = *n;
                let span = current.span;
                state.advance();
                Ok((ManifestValue::Int(val), span))
            }
            Token::Float(f) => {
                let val = *f;
                let span = current.span;
                state.advance();
                Ok((ManifestValue::Float(val), span))
            }
            Token::OpenBracket => {
                // Parse string list: ["a", "b", ...]
                state.advance(); // consume `[`
                let mut items = Vec::new();
                while !state.check(&Token::CloseBracket) {
                    let (s, _) = state.expect_string()?;
                    items.push(s);
                    // Optional comma between elements
                    if state.check(&Token::Comma) {
                        state.advance();
                    }
                }
                let end_span = state.expect(&Token::CloseBracket)?;
                let span = crate::lexer::Span::new(start.start, end_span.end);
                Ok((ManifestValue::StringList(items), span))
            }
            Token::Ident(name) if name == "env" => {
                // Parse env("VAR_NAME")
                state.advance(); // consume `env`
                let paren_span = state.current_span();
                state.expect(&Token::OpenParen)?;

                // Expect exactly one string argument
                let arg_current = state.peek_spanned().clone();
                let var_name = match &arg_current.token {
                    Token::String(s) => {
                        let s = s.clone();
                        state.advance();
                        s
                    }
                    _ => {
                        return Err(CompileError::Parser(format!(
                            "at byte {}: env() requires exactly one string literal argument",
                            paren_span.start,
                        )));
                    }
                };

                // Validate env var name: [A-Za-z0-9_]+
                if var_name.is_empty()
                    || !var_name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return Err(CompileError::Parser(format!(
                        "at byte {}: env variable name '{}' contains invalid characters (must be [A-Za-z0-9_]+)",
                        paren_span.start, var_name
                    )));
                }

                // Check no extra arguments
                if state.check(&Token::Comma) {
                    return Err(CompileError::Parser(format!(
                        "at byte {}: env() requires exactly one argument",
                        paren_span.start,
                    )));
                }

                let end_span = state.expect(&Token::CloseParen)?;
                let span = crate::lexer::Span::new(start.start, end_span.end);
                Ok((ManifestValue::EnvCall(var_name), span))
            }
            _ => Err(CompileError::Parser(format!(
                "at byte {}: expected string, integer, float, list, or env() but found {:?}",
                current.span.start,
                current.token
            ))),
        }
    }
}

/// Parse an entry block: `name { KEY = { fields... }, ... }`
fn parse_entry_block(
    state: &mut ParserState,
    name: &str,
    start: crate::lexer::Span,
) -> Result<ManifestBlock> {
    state.expect(&Token::OpenBrace)?;

    let mut entries: Vec<ManifestEntry> = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();

    while !state.check(&Token::CloseBrace) {
        // Parse entry key name
        let (key_name, key_span) = state.expect_ident()?;

        // Validate key name length (1-64 chars)
        if key_name.len() > 64 {
            return Err(CompileError::Parser(format!(
                "at byte {}: key/field name '{}' exceeds 64 characters",
                key_span.start, key_name
            )));
        }

        // Check for duplicate keys
        if seen_keys.contains(&key_name) {
            return Err(CompileError::Parser(format!(
                "at byte {}: duplicate key '{}' in {} block",
                key_span.start, key_name, name
            )));
        }
        seen_keys.insert(key_name.clone());

        // Expect `=`
        state.expect(&Token::Assign)?;

        // Expect `{` (start of inline struct)
        state.expect(&Token::OpenBrace)?;

        // Parse inline struct fields
        let mut fields: Vec<ManifestField> = Vec::new();
        let mut seen_fields: HashSet<String> = HashSet::new();

        while !state.check(&Token::CloseBrace) {
            if state.at_eof() {
                return Err(CompileError::Parser(format!(
                    "at byte {}: expected '}}' to close inline struct for '{}'",
                    state.current_span().start, key_name
                )));
            }

            // Parse field name
            let (field_name, field_span) = state.expect_ident()?;

            // Validate field name length (1-64 chars)
            if field_name.len() > 64 {
                return Err(CompileError::Parser(format!(
                    "at byte {}: key/field name '{}' exceeds 64 characters",
                    field_span.start, field_name
                )));
            }

            // Check for duplicate field names
            if seen_fields.contains(&field_name) {
                return Err(CompileError::Parser(format!(
                    "at byte {}: duplicate field '{}' in entry '{}'",
                    field_span.start, field_name, key_name
                )));
            }
            seen_fields.insert(field_name.clone());

            // Expect `=`
            state.expect(&Token::Assign)?;

            // Parse value
            let (value, value_span) = parse_manifest_value(state)?;

            let field_total_span =
                crate::lexer::Span::new(field_span.start, value_span.end);

            fields.push(ManifestField {
                name: field_name,
                value,
                span: field_total_span,
            });

            // Accept optional comma
            if state.check(&Token::Comma) {
                state.advance();
            }
        }

        // Validate 1-32 fields per inline struct
        if fields.is_empty() {
            return Err(CompileError::Parser(format!(
                "at byte {}: inline struct for '{}' must have at least 1 field",
                key_span.start, key_name
            )));
        }
        if fields.len() > 32 {
            return Err(CompileError::Parser(format!(
                "at byte {}: inline struct for '{}' exceeds 32 fields",
                key_span.start, key_name
            )));
        }

        // Expect inner `}`
        let inner_close = state.expect(&Token::CloseBrace)?;

        let entry_span = crate::lexer::Span::new(key_span.start, inner_close.end);

        entries.push(ManifestEntry {
            name: key_name,
            fields,
            span: entry_span,
        });

        // Accept optional comma after closing `}`
        if state.check(&Token::Comma) {
            state.advance();
        }
    }

    // Expect outer `}`
    let outer_close = state.expect(&Token::CloseBrace)?;
    let block_span = crate::lexer::Span::new(start.start, outer_close.end);

    let kind = match name {
        "products" => ManifestBlockKind::Products(entries),
        "strategies" => ManifestBlockKind::Strategies(entries),
        _ => unreachable!(),
    };

    Ok(ManifestBlock {
        kind,
        span: block_span,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex_with_spans;

    /// Helper to lex and parse a manifest source string.
    fn parse_source(source: &str) -> Result<ManifestProgram> {
        let tokens = lex_with_spans(source).map_err(|e| {
            CompileError::Parser(format!("lex error in test: {}", e))
        })?;
        parse_manifest(tokens)
    }

    #[test]
    fn parse_minimal_valid_manifest() {
        let source = r#"
account {
    name = "test"
}
gateway {
    port = 4002
}
data {
    source = "ibkr"
}
database {
    url = "postgres://localhost"
}
risk {
    max_daily_loss = -1000.0
}
products {
    ES = { multiplier = 50.0 }
}
strategies {
    alpha = { path = "alpha/strategy.flux" }
}
"#;
        let result = parse_source(source);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
        let program = result.unwrap();
        assert_eq!(program.blocks.len(), 7);

        // Verify block kinds are in order
        assert!(matches!(program.blocks[0].kind, ManifestBlockKind::Account(_)));
        assert!(matches!(program.blocks[1].kind, ManifestBlockKind::Gateway(_)));
        assert!(matches!(program.blocks[2].kind, ManifestBlockKind::Data(_)));
        assert!(matches!(program.blocks[3].kind, ManifestBlockKind::Database(_)));
        assert!(matches!(program.blocks[4].kind, ManifestBlockKind::Risk(_)));
        assert!(matches!(program.blocks[5].kind, ManifestBlockKind::Products(_)));
        assert!(matches!(program.blocks[6].kind, ManifestBlockKind::Strategies(_)));
    }

    #[test]
    fn parse_manifest_all_fields_populated() {
        let source = r#"
account {
    name = "swing"
    broker = "ibkr"
    account_id = "DU12345"
    mode = "paper"
}

gateway {
    host = "127.0.0.1"
    port = 4002
}

data {
    source = "ibkr"
    symbols = ["ES", "NQ"]
    interval = "1d"
}

database {
    url = "postgres://localhost/flux"
    schema = "swing"
}

risk {
    max_daily_loss = -15000.0
    max_weekly_loss = -30000.0
    max_position_per_product = 10
    max_total_notional = 3000000.0
    max_drawdown_pct = 0.08
    correlation_warning_threshold = 4
    initial_equity = 500000.0
}

products {
    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }
    NQ = { multiplier = 20.0, tick_size = 0.25, margin = 21120.0 }
}

strategies {
    aether = { path = "aether/strategy.flux", allocation = 0.6, priority = 1 }
    kairos = { path = "kairos/strategy.flux", allocation = 0.4, priority = 2 }
}
"#;
        let result = parse_source(source);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
        let program = result.unwrap();
        assert_eq!(program.blocks.len(), 7);

        // Verify account fields
        if let ManifestBlockKind::Account(fields) = &program.blocks[0].kind {
            assert_eq!(fields.len(), 4);
            assert_eq!(fields[0].name, "name");
            assert_eq!(fields[0].value, ManifestValue::String("swing".to_string()));
            assert_eq!(fields[1].name, "broker");
            assert_eq!(fields[2].name, "account_id");
            assert_eq!(fields[3].name, "mode");
            assert_eq!(fields[3].value, ManifestValue::String("paper".to_string()));
        } else {
            panic!("Expected Account block");
        }

        // Verify gateway fields
        if let ManifestBlockKind::Gateway(fields) = &program.blocks[1].kind {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name, "host");
            assert_eq!(fields[0].value, ManifestValue::String("127.0.0.1".to_string()));
            assert_eq!(fields[1].name, "port");
            assert_eq!(fields[1].value, ManifestValue::Int(4002));
        } else {
            panic!("Expected Gateway block");
        }

        // Verify data fields (string list)
        if let ManifestBlockKind::Data(fields) = &program.blocks[2].kind {
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[1].name, "symbols");
            assert_eq!(
                fields[1].value,
                ManifestValue::StringList(vec!["ES".to_string(), "NQ".to_string()])
            );
        } else {
            panic!("Expected Data block");
        }

        // Verify risk fields (negative floats and ints)
        if let ManifestBlockKind::Risk(fields) = &program.blocks[4].kind {
            assert_eq!(fields.len(), 7);
            assert_eq!(fields[0].name, "max_daily_loss");
            assert_eq!(fields[0].value, ManifestValue::Float(-15000.0));
            assert_eq!(fields[2].name, "max_position_per_product");
            assert_eq!(fields[2].value, ManifestValue::Int(10));
        } else {
            panic!("Expected Risk block");
        }

        // Verify products entries
        if let ManifestBlockKind::Products(entries) = &program.blocks[5].kind {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "ES");
            assert_eq!(entries[0].fields.len(), 3);
            assert_eq!(entries[0].fields[0].name, "multiplier");
            assert_eq!(entries[0].fields[0].value, ManifestValue::Float(50.0));
            assert_eq!(entries[1].name, "NQ");
        } else {
            panic!("Expected Products block");
        }

        // Verify strategies entries
        if let ManifestBlockKind::Strategies(entries) = &program.blocks[6].kind {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "aether");
            assert_eq!(entries[0].fields.len(), 3);
            assert_eq!(entries[0].fields[0].name, "path");
            assert_eq!(
                entries[0].fields[0].value,
                ManifestValue::String("aether/strategy.flux".to_string())
            );
            assert_eq!(entries[0].fields[1].name, "allocation");
            assert_eq!(entries[0].fields[1].value, ManifestValue::Float(0.6));
            assert_eq!(entries[0].fields[2].name, "priority");
            assert_eq!(entries[0].fields[2].value, ManifestValue::Int(1));
        } else {
            panic!("Expected Strategies block");
        }
    }

    #[test]
    fn parse_manifest_with_env_calls() {
        let source = r#"
account {
    name = "swing"
    account_id = env("IBKR_ACCOUNT")
    mode = "paper"
}
gateway {
    host = "127.0.0.1"
    port = 4002
}
data {
    source = "ibkr"
}
database {
    url = env("DB_URL")
    schema = "swing"
}
risk {
    initial_equity = 500000.0
}
products {
    ES = { multiplier = 50.0 }
}
strategies {
    alpha = { path = "alpha.flux" }
}
"#;
        let result = parse_source(source);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
        let program = result.unwrap();

        // Verify env() calls in account block
        if let ManifestBlockKind::Account(fields) = &program.blocks[0].kind {
            assert_eq!(fields[1].name, "account_id");
            assert_eq!(
                fields[1].value,
                ManifestValue::EnvCall("IBKR_ACCOUNT".to_string())
            );
        } else {
            panic!("Expected Account block");
        }

        // Verify env() call in database block
        if let ManifestBlockKind::Database(fields) = &program.blocks[3].kind {
            assert_eq!(fields[0].name, "url");
            assert_eq!(
                fields[0].value,
                ManifestValue::EnvCall("DB_URL".to_string())
            );
        } else {
            panic!("Expected Database block");
        }
    }

    #[test]
    fn parse_inline_struct_with_trailing_comma() {
        let source = r#"
account {
    name = "test"
}
gateway {
    port = 4002
}
data {
    source = "ibkr"
}
database {
    url = "postgres://localhost"
}
risk {
    initial_equity = 100000.0
}
products {
    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0, }
}
strategies {
    aether = { path = "aether.flux", allocation = 0.6, priority = 1, }
}
"#;
        let result = parse_source(source);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
        let program = result.unwrap();

        // Verify trailing comma doesn't cause issues
        if let ManifestBlockKind::Products(entries) = &program.blocks[5].kind {
            assert_eq!(entries[0].fields.len(), 3);
            assert_eq!(entries[0].fields[2].name, "margin");
            assert_eq!(entries[0].fields[2].value, ManifestValue::Float(15840.0));
        } else {
            panic!("Expected Products block");
        }
    }

    #[test]
    fn error_unrecognized_block_keyword() {
        let source = r#"
account {
    name = "test"
}
foobar {
    x = 1
}
"#;
        let result = parse_source(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unrecognized manifest block"),
            "Expected 'unrecognized manifest block' in error: {}",
            err_msg
        );
        assert!(
            err_msg.contains("foobar"),
            "Expected 'foobar' in error: {}",
            err_msg
        );
    }

    #[test]
    fn error_duplicate_block() {
        let source = r#"
account {
    name = "test"
}
account {
    name = "duplicate"
}
"#;
        let result = parse_source(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("duplicate"),
            "Expected 'duplicate' in error: {}",
            err_msg
        );
        assert!(
            err_msg.contains("account"),
            "Expected 'account' in error: {}",
            err_msg
        );
    }

    #[test]
    fn error_duplicate_key_in_products() {
        let source = r#"
account {
    name = "test"
}
gateway {
    port = 4002
}
data {
    source = "ibkr"
}
database {
    url = "pg://localhost"
}
risk {
    initial_equity = 100000.0
}
products {
    ES = { multiplier = 50.0 }
    ES = { multiplier = 20.0 }
}
strategies {
    alpha = { path = "alpha.flux" }
}
"#;
        let result = parse_source(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("duplicate key"),
            "Expected 'duplicate key' in error: {}",
            err_msg
        );
        assert!(
            err_msg.contains("ES"),
            "Expected 'ES' in error: {}",
            err_msg
        );
    }

    #[test]
    fn error_duplicate_field_in_inline_struct() {
        let source = r#"
account {
    name = "test"
}
gateway {
    port = 4002
}
data {
    source = "ibkr"
}
database {
    url = "pg://localhost"
}
risk {
    initial_equity = 100000.0
}
products {
    ES = { multiplier = 50.0, multiplier = 20.0 }
}
strategies {
    alpha = { path = "alpha.flux" }
}
"#;
        let result = parse_source(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("duplicate field"),
            "Expected 'duplicate field' in error: {}",
            err_msg
        );
        assert!(
            err_msg.contains("multiplier"),
            "Expected 'multiplier' in error: {}",
            err_msg
        );
    }

    #[test]
    fn error_missing_equals_after_key() {
        let source = r#"
account {
    name "test"
}
"#;
        let result = parse_source(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // The parser should report an error about expecting '='
        assert!(
            err_msg.contains("expected") || err_msg.contains("="),
            "Expected error about missing '=': {}",
            err_msg
        );
    }

    #[test]
    fn error_unclosed_brace() {
        let source = r#"
account {
    name = "test"
"#;
        let result = parse_source(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("}") || err_msg.contains("close"),
            "Expected error about unclosed brace: {}",
            err_msg
        );
    }

    #[test]
    fn parse_data_block_in_manifest_context() {
        // The `data` keyword is a reserved Token::Data in the lexer,
        // not a regular identifier. The manifest parser must handle this.
        let source = r#"
account {
    name = "test"
}
gateway {
    port = 4002
}
data {
    source = "ibkr"
    symbols = ["ES", "NQ", "YM"]
    interval = "1d"
}
database {
    url = "pg://localhost"
}
risk {
    initial_equity = 100000.0
}
products {
    ES = { multiplier = 50.0 }
}
strategies {
    alpha = { path = "alpha.flux" }
}
"#;
        let result = parse_source(source);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
        let program = result.unwrap();

        // Find the data block
        let data_block = program.blocks.iter().find(|b| {
            matches!(b.kind, ManifestBlockKind::Data(_))
        });
        assert!(data_block.is_some(), "Expected a Data block");

        if let ManifestBlockKind::Data(fields) = &data_block.unwrap().kind {
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[0].name, "source");
            assert_eq!(fields[0].value, ManifestValue::String("ibkr".to_string()));
            assert_eq!(fields[1].name, "symbols");
            assert_eq!(
                fields[1].value,
                ManifestValue::StringList(vec![
                    "ES".to_string(),
                    "NQ".to_string(),
                    "YM".to_string(),
                ])
            );
            assert_eq!(fields[2].name, "interval");
            assert_eq!(fields[2].value, ManifestValue::String("1d".to_string()));
        } else {
            panic!("Expected Data block kind");
        }
    }
}
