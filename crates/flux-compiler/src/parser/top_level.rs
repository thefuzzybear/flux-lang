//! Top-level parsing for the Flux language.
//!
//! Handles: program structure, import statements, strategy declarations,
//! params blocks, state blocks, event handlers, and strategy-level properties.

use crate::error::{CompileError, Result};
use crate::lexer::{Span, Token};

use super::ast::*;
use super::parser_state::ParserState;

impl ParserState {
    /// Parse the entire program: imports, optional data block, optional connector block, then strategy, then Eof.
    pub fn parse_program(&mut self) -> Result<Program> {
        let start_span = self.current_span();
        let mut imports = Vec::new();
        let mut data_block: Option<DataBlock> = None;
        let mut connector_block: Option<ConnectorBlock> = None;

        // Parse imports, optional data block, and optional connector block (interleaved before strategy)
        loop {
            match self.peek().clone() {
                Token::From => {
                    imports.push(self.parse_import()?);
                }
                Token::Data => {
                    if data_block.is_some() {
                        return Err(CompileError::Parser(format!(
                            "at byte {}: only one data block is permitted per file",
                            self.current_span().start
                        )));
                    }
                    data_block = Some(self.parse_data_block()?);
                }
                Token::Connector => {
                    if connector_block.is_some() {
                        return Err(CompileError::Parser(format!(
                            "at byte {}: only one connector block is permitted per file",
                            self.current_span().start
                        )));
                    }
                    connector_block = Some(self.parse_connector_block()?);
                }
                _ => break,
            }
        }

        // Parse strategy
        let strategy = self.parse_strategy()?;

        // Assert Eof
        if !self.at_eof() {
            return Err(CompileError::Parser(format!(
                "at byte {}: unexpected tokens after strategy body",
                self.current_span().start
            )));
        }

        let span = self.span_from(start_span);
        Ok(Program {
            imports,
            data_block,
            connector_block,
            strategy,
            span,
        })
    }

    /// Parse an import statement: `from module.path import {name1, name2}`
    fn parse_import(&mut self) -> Result<Import> {
        let start_span = self.current_span();
        self.expect(&Token::From)?; // consume `from`

        // Parse dotted module path
        let (first_segment, _) = self.expect_ident()?;
        let mut path = first_segment;
        while self.check(&Token::Dot) {
            self.advance(); // consume dot
            let (segment, _) = self.expect_ident()?;
            path.push('.');
            path.push_str(&segment);
        }

        self.expect(&Token::Import)?; // consume `import`
        self.expect(&Token::OpenBrace)?; // consume `{`

        // Parse import names (at least one required)
        if self.check(&Token::CloseBrace) {
            return Err(CompileError::Parser(format!(
                "at byte {}: expected at least one import name",
                self.current_span().start
            )));
        }

        let mut names = Vec::new();
        let (first_name, _) = self.expect_ident()?;
        names.push(first_name);

        while self.check(&Token::Comma) {
            self.advance(); // consume comma
            if self.check(&Token::CloseBrace) {
                break; // trailing comma
            }
            let (name, _) = self.expect_ident()?;
            names.push(name);
        }

        self.expect(&Token::CloseBrace)?;
        let span = self.span_from(start_span);
        Ok(Import {
            module_path: path,
            names,
            span,
        })
    }

    /// Parse a data block: `data { key = value ... }`
    ///
    /// Valid keys: symbols, period, interval, source.
    /// `symbols` expects a string list `["A", "B"]`; other keys expect a string literal.
    fn parse_data_block(&mut self) -> Result<DataBlock> {
        let start_span = self.current_span();
        self.expect(&Token::Data)?;
        self.expect(&Token::OpenBrace)?;

        let mut symbols: Option<DataField<Vec<String>>> = None;
        let mut period: Option<DataField<String>> = None;
        let mut interval: Option<DataField<String>> = None;
        let mut source: Option<DataField<String>> = None;

        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            let key_span = self.current_span();
            let (key, _) = self.expect_ident()?;
            self.expect(&Token::Assign)?;

            match key.as_str() {
                "symbols" => {
                    let field_start = self.current_span();
                    let list = self.parse_string_list()?;
                    let field_end = self.span_from(field_start);
                    symbols = Some(DataField {
                        value: list,
                        span: Span::new(field_start.start, field_end.end),
                    });
                }
                "period" | "interval" | "source" => {
                    let (value, value_span) = self.expect_string()?;
                    let field = DataField {
                        value,
                        span: value_span,
                    };
                    match key.as_str() {
                        "period" => period = Some(field),
                        "interval" => interval = Some(field),
                        "source" => source = Some(field),
                        _ => unreachable!(),
                    }
                }
                other => {
                    return Err(CompileError::Parser(format!(
                        "at byte {}: unrecognized data block key '{}'. \
                         Valid keys: symbols, period, interval, source",
                        key_span.start, other
                    )));
                }
            }
        }

        let end_span = self.current_span();
        self.expect(&Token::CloseBrace)?;

        Ok(DataBlock {
            symbols,
            period,
            interval,
            source,
            span: Span::new(start_span.start, end_span.end),
        })
    }

    /// Parse a connector block: `connector { key = value ... }`
    ///
    /// Valid keys: type, url, symbols, interval, file.
    /// `symbols` expects a string list `["A", "B"]`; other keys expect a string literal.
    fn parse_connector_block(&mut self) -> Result<ConnectorBlock> {
        let start_span = self.current_span();
        self.expect(&Token::Connector)?;
        self.expect(&Token::OpenBrace)?;

        let mut connector_type: Option<DataField<String>> = None;
        let mut url: Option<DataField<String>> = None;
        let mut symbols: Option<DataField<Vec<String>>> = None;
        let mut interval: Option<DataField<String>> = None;
        let mut file: Option<DataField<String>> = None;

        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            let key_span = self.current_span();
            let (key, _) = self.expect_ident()?;
            self.expect(&Token::Assign)?;

            match key.as_str() {
                "type" => {
                    let (value, value_span) = self.expect_string()?;
                    connector_type = Some(DataField {
                        value,
                        span: value_span,
                    });
                }
                "url" => {
                    let (value, value_span) = self.expect_string()?;
                    url = Some(DataField {
                        value,
                        span: value_span,
                    });
                }
                "symbols" => {
                    let field_start = self.current_span();
                    let list = self.parse_string_list()?;
                    let field_end = self.span_from(field_start);
                    symbols = Some(DataField {
                        value: list,
                        span: Span::new(field_start.start, field_end.end),
                    });
                }
                "interval" => {
                    let (value, value_span) = self.expect_string()?;
                    interval = Some(DataField {
                        value,
                        span: value_span,
                    });
                }
                "file" => {
                    let (value, value_span) = self.expect_string()?;
                    file = Some(DataField {
                        value,
                        span: value_span,
                    });
                }
                other => {
                    return Err(CompileError::Parser(format!(
                        "at byte {}: unrecognized connector block key '{}'. \
                         Valid keys: type, url, symbols, interval, file",
                        key_span.start, other
                    )));
                }
            }
        }

        let end_span = self.current_span();
        self.expect(&Token::CloseBrace)?;

        Ok(ConnectorBlock {
            connector_type,
            url,
            symbols,
            interval,
            file,
            span: Span::new(start_span.start, end_span.end),
        })
    }

    /// Parse a string list: `["value1", "value2", ...]`
    fn parse_string_list(&mut self) -> Result<Vec<String>> {
        self.expect(&Token::OpenBracket)?;

        let mut items = Vec::new();

        if !self.check(&Token::CloseBracket) {
            let (first, _) = self.expect_string()?;
            items.push(first);

            while self.check(&Token::Comma) {
                self.advance(); // consume comma
                if self.check(&Token::CloseBracket) {
                    break; // trailing comma
                }
                let (item, _) = self.expect_string()?;
                items.push(item);
            }
        }

        self.expect(&Token::CloseBracket)?;
        Ok(items)
    }

    /// Parse strategy declaration: `strategy Name { body }`
    fn parse_strategy(&mut self) -> Result<Strategy> {
        let start_span = self.current_span();

        // If the current token is not Strategy, report error
        if !self.check(&Token::Strategy) {
            return Err(CompileError::Parser(format!(
                "at byte {}: expected strategy declaration, found {:?}",
                self.current_span().start,
                self.peek()
            )));
        }
        self.advance(); // consume `strategy`

        let (name, _) = self.expect_ident()?;
        self.expect(&Token::OpenBrace)?;

        let body = self.parse_strategy_body()?;

        self.expect(&Token::CloseBrace)?;
        let span = self.span_from(start_span);
        Ok(Strategy { name, body, span })
    }

    /// Parse strategy body items.
    fn parse_strategy_body(&mut self) -> Result<Vec<StrategyItem>> {
        let mut items = Vec::new();

        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            let item = match self.peek().clone() {
                Token::Params => StrategyItem::ParamsBlock(self.parse_params_block()?),
                Token::State => StrategyItem::StateBlock(self.parse_state_block()?),
                Token::On => StrategyItem::EventHandler(self.parse_event_handler()?),
                Token::Ident(ref name) if name.starts_with("on_") => {
                    StrategyItem::EventHandler(self.parse_event_handler()?)
                }
                Token::Ident(_) => StrategyItem::Property(self.parse_property()?),
                _ => {
                    return Err(
                        self.error_expected("strategy item (params, state, on_event, or property)")
                    );
                }
            };
            items.push(item);
        }

        Ok(items)
    }

    /// Parse a params block: `params { name = value, ... }`
    fn parse_params_block(&mut self) -> Result<ParamsBlock> {
        let start_span = self.current_span();
        self.advance(); // consume `params`
        self.expect(&Token::OpenBrace)?;

        let mut params = Vec::new();
        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            let param_start = self.current_span();
            let (name, _) = self.expect_ident()?;
            self.expect(&Token::Assign)?;
            let default_value = self.parse_expr(0)?;
            let param_span = self.span_from(param_start);
            params.push(Param {
                name,
                default_value,
                span: param_span,
            });

            // Optional comma separator (handles trailing commas)
            if self.check(&Token::Comma) {
                self.advance();
            }
        }

        self.expect(&Token::CloseBrace)?;
        let span = self.span_from(start_span);
        Ok(ParamsBlock { params, span })
    }

    /// Parse a state block: `state { name = value, ... }`
    fn parse_state_block(&mut self) -> Result<StateBlock> {
        let start_span = self.current_span();
        self.advance(); // consume `state`
        self.expect(&Token::OpenBrace)?;

        let mut variables = Vec::new();
        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            let var_start = self.current_span();
            let (name, _) = self.expect_ident()?;
            self.expect(&Token::Assign)?;
            let initial_value = self.parse_expr(0)?;
            let var_span = self.span_from(var_start);
            variables.push(StateVar {
                name,
                initial_value,
                span: var_span,
            });

            // Optional comma separator (handles trailing commas)
            if self.check(&Token::Comma) {
                self.advance();
            }
        }

        self.expect(&Token::CloseBrace)?;
        let span = self.span_from(start_span);
        Ok(StateBlock { variables, span })
    }

    /// Parse an event handler block.
    ///
    /// Handles both patterns:
    /// - `On` + `Ident(name)` → event_name = name
    /// - `Ident("on_bar")` → event_name = "bar" (strip "on_" prefix)
    fn parse_event_handler(&mut self) -> Result<EventHandler> {
        let start_span = self.current_span();

        let event_name = match self.peek().clone() {
            Token::On => {
                self.advance(); // consume `on`
                let (name, _) = self.expect_ident()?;
                name
            }
            Token::Ident(name) if name.starts_with("on_") => {
                let n = name.clone();
                self.advance();
                // Strip "on_" prefix to get just the event name
                n[3..].to_string()
            }
            _ => return Err(self.error_expected("event handler")),
        };

        self.expect(&Token::OpenBrace)?;
        let mut body = Vec::new();
        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            body.push(self.parse_statement()?);
        }
        self.expect(&Token::CloseBrace)?;

        let span = self.span_from(start_span);
        Ok(EventHandler {
            event_name,
            body,
            span,
        })
    }

    /// Parse a strategy-level property: `name = expr`
    fn parse_property(&mut self) -> Result<Property> {
        let start_span = self.current_span();
        let (name, _) = self.expect_ident()?;
        self.expect(&Token::Assign)?;
        let value = self.parse_expr(0)?;
        let span = self.span_from(start_span);
        Ok(Property { name, value, span })
    }
}

#[cfg(test)]
mod tests {
    use crate::lexer::{Token, SpannedToken, Span};
    use super::ParserState;
    use super::super::ast::*;

    fn make_tokens(tokens: Vec<Token>) -> Vec<SpannedToken> {
        let mut result = Vec::new();
        let mut pos = 0;
        for token in tokens {
            let len = match &token {
                Token::Ident(s) => s.len(),
                Token::Int(_) => 1,
                Token::Float(_) => 3,
                Token::String(s) => s.len() + 2,
                Token::Eof => 0,
                _ => 1,
            };
            result.push(SpannedToken { token, span: Span::new(pos, pos + len) });
            pos += len + 1;
        }
        result
    }

    fn parse_program(tokens: Vec<Token>) -> crate::error::Result<Program> {
        let spanned = make_tokens(tokens);
        let mut state = ParserState::new(spanned)?;
        state.parse_program()
    }

    // ===== 1. Minimal valid program =====

    #[test]
    fn minimal_valid_program() {
        // strategy Name {}
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("Name".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.imports.is_empty());
        assert_eq!(program.strategy.name, "Name");
        assert!(program.strategy.body.is_empty());
    }

    // ===== 2. Single import =====

    #[test]
    fn single_import_with_dot_path() {
        // from flux.indicators import {SMA} strategy X {}
        let program = parse_program(vec![
            Token::From,
            Token::Ident("flux".to_string()),
            Token::Dot,
            Token::Ident("indicators".to_string()),
            Token::Import,
            Token::OpenBrace,
            Token::Ident("SMA".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.imports[0].module_path, "flux.indicators");
        assert_eq!(program.imports[0].names, vec!["SMA".to_string()]);
        assert_eq!(program.strategy.name, "X");
    }

    // ===== 3. Multiple imports =====

    #[test]
    fn multiple_imports() {
        // from flux.indicators import {SMA} from flux.utils import {log} strategy X {}
        let program = parse_program(vec![
            Token::From,
            Token::Ident("flux".to_string()),
            Token::Dot,
            Token::Ident("indicators".to_string()),
            Token::Import,
            Token::OpenBrace,
            Token::Ident("SMA".to_string()),
            Token::CloseBrace,
            Token::From,
            Token::Ident("flux".to_string()),
            Token::Dot,
            Token::Ident("utils".to_string()),
            Token::Import,
            Token::OpenBrace,
            Token::Ident("log".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.imports.len(), 2);
        assert_eq!(program.imports[0].module_path, "flux.indicators");
        assert_eq!(program.imports[0].names, vec!["SMA".to_string()]);
        assert_eq!(program.imports[1].module_path, "flux.utils");
        assert_eq!(program.imports[1].names, vec!["log".to_string()]);
    }

    // ===== 4. Trailing commas in import list =====

    #[test]
    fn trailing_comma_in_import_list() {
        // from m import {a, b,} strategy X {}
        let program = parse_program(vec![
            Token::From,
            Token::Ident("m".to_string()),
            Token::Import,
            Token::OpenBrace,
            Token::Ident("a".to_string()),
            Token::Comma,
            Token::Ident("b".to_string()),
            Token::Comma,
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.imports[0].names, vec!["a".to_string(), "b".to_string()]);
    }

    // ===== 5. Params block with multiple parameters =====

    #[test]
    fn params_block_multiple_params() {
        // strategy X { params { period = 20  threshold = 2.0 } }
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::Params,
            Token::OpenBrace,
            Token::Ident("period".to_string()),
            Token::Assign,
            Token::Int(20),
            Token::Ident("threshold".to_string()),
            Token::Assign,
            Token::Float(2.0),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.strategy.body.len(), 1);
        match &program.strategy.body[0] {
            StrategyItem::ParamsBlock(params) => {
                assert_eq!(params.params.len(), 2);
                assert_eq!(params.params[0].name, "period");
                assert_eq!(params.params[0].default_value.kind, ExprKind::IntLiteral(20));
                assert_eq!(params.params[1].name, "threshold");
                assert_eq!(params.params[1].default_value.kind, ExprKind::FloatLiteral(2.0));
            }
            other => panic!("Expected ParamsBlock, got {:?}", other),
        }
    }

    // ===== 6. State block with list literal =====

    #[test]
    fn state_block_with_list_literal() {
        // strategy X { state { prices = [] } }
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::State,
            Token::OpenBrace,
            Token::Ident("prices".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::CloseBracket,
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.strategy.body.len(), 1);
        match &program.strategy.body[0] {
            StrategyItem::StateBlock(state) => {
                assert_eq!(state.variables.len(), 1);
                assert_eq!(state.variables[0].name, "prices");
                assert_eq!(state.variables[0].initial_value.kind, ExprKind::ListLiteral(vec![]));
            }
            other => panic!("Expected StateBlock, got {:?}", other),
        }
    }

    // ===== 7. Event handler as single ident (on_bar) =====

    #[test]
    fn event_handler_single_ident_on_bar() {
        // strategy X { on_bar { x = 1 } }
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::Ident("on_bar".to_string()),
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.strategy.body.len(), 1);
        match &program.strategy.body[0] {
            StrategyItem::EventHandler(handler) => {
                assert_eq!(handler.event_name, "bar");
                assert_eq!(handler.body.len(), 1);
            }
            other => panic!("Expected EventHandler, got {:?}", other),
        }
    }

    // ===== 8. Event handler as On + Ident =====

    #[test]
    fn event_handler_on_keyword_plus_ident() {
        // strategy X { on bar { x = 1 } }
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::On,
            Token::Ident("bar".to_string()),
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.strategy.body.len(), 1);
        match &program.strategy.body[0] {
            StrategyItem::EventHandler(handler) => {
                assert_eq!(handler.event_name, "bar");
                assert_eq!(handler.body.len(), 1);
            }
            other => panic!("Expected EventHandler, got {:?}", other),
        }
    }

    // ===== 9. Strategy property =====

    #[test]
    fn strategy_property() {
        // strategy X { book_side = LONG }
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::Ident("book_side".to_string()),
            Token::Assign,
            Token::Ident("LONG".to_string()),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.strategy.body.len(), 1);
        match &program.strategy.body[0] {
            StrategyItem::Property(prop) => {
                assert_eq!(prop.name, "book_side");
                assert_eq!(prop.value.kind, ExprKind::Ident("LONG".to_string()));
            }
            other => panic!("Expected Property, got {:?}", other),
        }
    }

    // ===== 10. Full program with imports + strategy + all block types =====

    #[test]
    fn full_program_all_block_types() {
        // from flux.indicators import {SMA}
        // strategy MyStrat {
        //   params { period = 20 }
        //   state { prices = [] }
        //   on_bar { x = 1 }
        // }
        let program = parse_program(vec![
            // import
            Token::From,
            Token::Ident("flux".to_string()),
            Token::Dot,
            Token::Ident("indicators".to_string()),
            Token::Import,
            Token::OpenBrace,
            Token::Ident("SMA".to_string()),
            Token::CloseBrace,
            // strategy
            Token::Strategy,
            Token::Ident("MyStrat".to_string()),
            Token::OpenBrace,
            // params block
            Token::Params,
            Token::OpenBrace,
            Token::Ident("period".to_string()),
            Token::Assign,
            Token::Int(20),
            Token::CloseBrace,
            // state block
            Token::State,
            Token::OpenBrace,
            Token::Ident("prices".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::CloseBracket,
            Token::CloseBrace,
            // event handler
            Token::Ident("on_bar".to_string()),
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            // end strategy
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.imports[0].module_path, "flux.indicators");
        assert_eq!(program.strategy.name, "MyStrat");
        assert_eq!(program.strategy.body.len(), 3);
        assert!(matches!(&program.strategy.body[0], StrategyItem::ParamsBlock(_)));
        assert!(matches!(&program.strategy.body[1], StrategyItem::StateBlock(_)));
        assert!(matches!(&program.strategy.body[2], StrategyItem::EventHandler(_)));
    }

    // ===== 11. Error: empty input =====

    #[test]
    fn error_empty_input() {
        let result = parse_program(vec![]);
        assert!(result.is_err());
    }

    // ===== 12. Error: Eof only (no strategy) =====

    #[test]
    fn error_eof_only_no_strategy() {
        let result = parse_program(vec![Token::Eof]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("expected strategy declaration"),
                    "Expected 'expected strategy declaration' in error, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // ===== 13. Error: extra tokens after strategy =====

    #[test]
    fn error_extra_tokens_after_strategy() {
        // strategy X {} extra_stuff
        let result = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Ident("extra".to_string()),
            Token::Eof,
        ]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("unexpected tokens after strategy body"),
                    "Expected 'unexpected tokens after strategy body' in error, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // ===== 14. Error: empty import list =====

    #[test]
    fn error_empty_import_list() {
        // from m import {} strategy X {}
        let result = parse_program(vec![
            Token::From,
            Token::Ident("m".to_string()),
            Token::Import,
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("at least one import name"),
                    "Expected 'at least one import name' in error, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // ===== 15. Error: missing module path ident (keyword where ident expected) =====

    #[test]
    fn error_missing_module_path_ident() {
        // from import {x} strategy X {}
        // After `From`, expect_ident sees Token::Import (a keyword), not an identifier
        let result = parse_program(vec![
            Token::From,
            Token::Import,
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("identifier"),
                    "Expected 'identifier' in error, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // ===== 16. Data block: full data block with all fields =====

    #[test]
    fn data_block_all_fields() {
        // data { symbols = ["AAPL", "MSFT"]  period = "1y"  interval = "1d"  source = "yahoo" }
        // strategy X {}
        let program = parse_program(vec![
            Token::Data,
            Token::OpenBrace,
            Token::Ident("symbols".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::String("AAPL".to_string()),
            Token::Comma,
            Token::String("MSFT".to_string()),
            Token::CloseBracket,
            Token::Ident("period".to_string()),
            Token::Assign,
            Token::String("1y".to_string()),
            Token::Ident("interval".to_string()),
            Token::Assign,
            Token::String("1d".to_string()),
            Token::Ident("source".to_string()),
            Token::Assign,
            Token::String("yahoo".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.data_block.is_some());
        let data = program.data_block.unwrap();
        assert_eq!(
            data.symbols.as_ref().unwrap().value,
            vec!["AAPL".to_string(), "MSFT".to_string()]
        );
        assert_eq!(data.period.as_ref().unwrap().value, "1y");
        assert_eq!(data.interval.as_ref().unwrap().value, "1d");
        assert_eq!(data.source.as_ref().unwrap().value, "yahoo");
    }

    // ===== 17. Data block: optional — no data block =====

    #[test]
    fn data_block_optional_no_block() {
        // strategy X {}
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.data_block.is_none());
    }

    // ===== 18. Data block: partial fields (only symbols) =====

    #[test]
    fn data_block_partial_fields() {
        // data { symbols = ["GOOG"] } strategy X {}
        let program = parse_program(vec![
            Token::Data,
            Token::OpenBrace,
            Token::Ident("symbols".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::String("GOOG".to_string()),
            Token::CloseBracket,
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.data_block.is_some());
        let data = program.data_block.unwrap();
        assert_eq!(data.symbols.as_ref().unwrap().value, vec!["GOOG".to_string()]);
        assert!(data.period.is_none());
        assert!(data.interval.is_none());
        assert!(data.source.is_none());
    }

    // ===== 19. Data block: empty data block (no fields) =====

    #[test]
    fn data_block_empty() {
        // data {} strategy X {}
        let program = parse_program(vec![
            Token::Data,
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.data_block.is_some());
        let data = program.data_block.unwrap();
        assert!(data.symbols.is_none());
        assert!(data.period.is_none());
        assert!(data.interval.is_none());
        assert!(data.source.is_none());
    }

    // ===== 20. Data block: between imports and strategy =====

    #[test]
    fn data_block_between_imports_and_strategy() {
        // from m import {x}  data { period = "6mo" }  strategy X {}
        let program = parse_program(vec![
            Token::From,
            Token::Ident("m".to_string()),
            Token::Import,
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::CloseBrace,
            Token::Data,
            Token::OpenBrace,
            Token::Ident("period".to_string()),
            Token::Assign,
            Token::String("6mo".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(program.imports.len(), 1);
        assert!(program.data_block.is_some());
        assert_eq!(program.data_block.unwrap().period.unwrap().value, "6mo");
    }

    // ===== 21. Error: duplicate data block =====

    #[test]
    fn error_duplicate_data_block() {
        // data { period = "1y" }  data { period = "6mo" }  strategy X {}
        let result = parse_program(vec![
            Token::Data,
            Token::OpenBrace,
            Token::Ident("period".to_string()),
            Token::Assign,
            Token::String("1y".to_string()),
            Token::CloseBrace,
            Token::Data,
            Token::OpenBrace,
            Token::Ident("period".to_string()),
            Token::Assign,
            Token::String("6mo".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("only one data block is permitted per file"),
                    "Expected duplicate data block error, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // ===== 22. Error: unrecognized data block key =====

    #[test]
    fn error_unrecognized_data_block_key() {
        // data { unknown = "value" } strategy X {}
        let result = parse_program(vec![
            Token::Data,
            Token::OpenBrace,
            Token::Ident("unknown".to_string()),
            Token::Assign,
            Token::String("value".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("unrecognized data block key 'unknown'"),
                    "Expected unrecognized key error, got: {msg}"
                );
                assert!(
                    msg.contains("Valid keys: symbols, period, interval, source"),
                    "Expected valid keys listing, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // ===== 23. Data block: symbols with trailing comma =====

    #[test]
    fn data_block_symbols_trailing_comma() {
        // data { symbols = ["AAPL", "MSFT",] } strategy X {}
        let program = parse_program(vec![
            Token::Data,
            Token::OpenBrace,
            Token::Ident("symbols".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::String("AAPL".to_string()),
            Token::Comma,
            Token::String("MSFT".to_string()),
            Token::Comma,
            Token::CloseBracket,
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        let data = program.data_block.unwrap();
        assert_eq!(
            data.symbols.unwrap().value,
            vec!["AAPL".to_string(), "MSFT".to_string()]
        );
    }

    // ===== Connector block tests =====

    #[test]
    fn connector_block_all_fields() {
        // connector { type = "websocket" url = "wss://example.com" symbols = ["AAPL", "MSFT"] interval = "1m" file = "data.csv" }
        // strategy X {}
        let program = parse_program(vec![
            Token::Connector,
            Token::OpenBrace,
            Token::Ident("type".to_string()),
            Token::Assign,
            Token::String("websocket".to_string()),
            Token::Ident("url".to_string()),
            Token::Assign,
            Token::String("wss://stream.example.com/v1".to_string()),
            Token::Ident("symbols".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::String("AAPL".to_string()),
            Token::Comma,
            Token::String("MSFT".to_string()),
            Token::CloseBracket,
            Token::Ident("interval".to_string()),
            Token::Assign,
            Token::String("1m".to_string()),
            Token::Ident("file".to_string()),
            Token::Assign,
            Token::String("data.csv".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.connector_block.is_some());
        let conn = program.connector_block.unwrap();
        assert_eq!(conn.connector_type.as_ref().unwrap().value, "websocket");
        assert_eq!(conn.url.as_ref().unwrap().value, "wss://stream.example.com/v1");
        assert_eq!(
            conn.symbols.as_ref().unwrap().value,
            vec!["AAPL".to_string(), "MSFT".to_string()]
        );
        assert_eq!(conn.interval.as_ref().unwrap().value, "1m");
        assert_eq!(conn.file.as_ref().unwrap().value, "data.csv");
    }

    #[test]
    fn connector_block_partial_fields() {
        // connector { type = "replay" file = "history.csv" } strategy X {}
        let program = parse_program(vec![
            Token::Connector,
            Token::OpenBrace,
            Token::Ident("type".to_string()),
            Token::Assign,
            Token::String("replay".to_string()),
            Token::Ident("file".to_string()),
            Token::Assign,
            Token::String("history.csv".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.connector_block.is_some());
        let conn = program.connector_block.unwrap();
        assert_eq!(conn.connector_type.as_ref().unwrap().value, "replay");
        assert!(conn.url.is_none());
        assert!(conn.symbols.is_none());
        assert!(conn.interval.is_none());
        assert_eq!(conn.file.as_ref().unwrap().value, "history.csv");
    }

    #[test]
    fn connector_block_with_data_block() {
        // data { symbols = ["AAPL"] } connector { type = "websocket" url = "wss://x.com" } strategy X {}
        let program = parse_program(vec![
            Token::Data,
            Token::OpenBrace,
            Token::Ident("symbols".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::String("AAPL".to_string()),
            Token::CloseBracket,
            Token::CloseBrace,
            Token::Connector,
            Token::OpenBrace,
            Token::Ident("type".to_string()),
            Token::Assign,
            Token::String("websocket".to_string()),
            Token::Ident("url".to_string()),
            Token::Assign,
            Token::String("wss://x.com".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.data_block.is_some());
        assert!(program.connector_block.is_some());
        let conn = program.connector_block.unwrap();
        assert_eq!(conn.connector_type.as_ref().unwrap().value, "websocket");
        assert_eq!(conn.url.as_ref().unwrap().value, "wss://x.com");
    }

    #[test]
    fn connector_block_before_data_block() {
        // connector { type = "poll" } data { symbols = ["GOOG"] } strategy X {}
        let program = parse_program(vec![
            Token::Connector,
            Token::OpenBrace,
            Token::Ident("type".to_string()),
            Token::Assign,
            Token::String("poll".to_string()),
            Token::CloseBrace,
            Token::Data,
            Token::OpenBrace,
            Token::Ident("symbols".to_string()),
            Token::Assign,
            Token::OpenBracket,
            Token::String("GOOG".to_string()),
            Token::CloseBracket,
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.data_block.is_some());
        assert!(program.connector_block.is_some());
        let conn = program.connector_block.unwrap();
        assert_eq!(conn.connector_type.as_ref().unwrap().value, "poll");
    }

    #[test]
    fn connector_block_no_block_means_none() {
        // strategy X {}
        let program = parse_program(vec![
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert!(program.connector_block.is_none());
    }

    #[test]
    fn connector_block_duplicate_is_error() {
        // connector {} connector {} strategy X {}
        let result = parse_program(vec![
            Token::Connector,
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Connector,
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("only one connector block is permitted"),
                    "Expected 'only one connector block is permitted' in error, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    #[test]
    fn connector_block_unrecognized_key_is_error() {
        // connector { unknown_key = "value" } strategy X {}
        let result = parse_program(vec![
            Token::Connector,
            Token::OpenBrace,
            Token::Ident("unknown_key".to_string()),
            Token::Assign,
            Token::String("value".to_string()),
            Token::CloseBrace,
            Token::Strategy,
            Token::Ident("X".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ]);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CompileError::Parser(msg) => {
                assert!(
                    msg.contains("unrecognized connector block key 'unknown_key'"),
                    "Expected 'unrecognized connector block key' in error, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    #[test]
    fn connector_block_source_level_parse() {
        // Full source-level test using lex_with_spans + parse
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;

        let source = r#"
connector {
    type = "websocket"
    url = "wss://stream.example.com/v1"
    symbols = ["AAPL", "MSFT"]
    interval = "1m"
}

strategy MyStrat {
    on bar {
        x = 1
    }
}
"#;
        let tokens = lex_with_spans(source).unwrap();
        let program = parse(tokens).unwrap();

        assert!(program.connector_block.is_some());
        let conn = program.connector_block.unwrap();
        assert_eq!(conn.connector_type.as_ref().unwrap().value, "websocket");
        assert_eq!(conn.url.as_ref().unwrap().value, "wss://stream.example.com/v1");
        assert_eq!(
            conn.symbols.as_ref().unwrap().value,
            vec!["AAPL".to_string(), "MSFT".to_string()]
        );
        assert_eq!(conn.interval.as_ref().unwrap().value, "1m");
        assert!(conn.file.is_none());
        assert_eq!(program.strategy.name, "MyStrat");
    }
}
