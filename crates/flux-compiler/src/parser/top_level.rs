//! Top-level parsing for the Flux language.
//!
//! Handles: program structure, import statements, strategy declarations,
//! params blocks, state blocks, event handlers, and strategy-level properties.

use crate::error::{CompileError, Result};
use crate::lexer::Token;

use super::ast::*;
use super::parser_state::ParserState;

impl ParserState {
    /// Parse the entire program: imports then strategy then Eof.
    pub fn parse_program(&mut self) -> Result<Program> {
        let start_span = self.current_span();
        let mut imports = Vec::new();

        // Parse imports
        while self.check(&Token::From) {
            imports.push(self.parse_import()?);
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
}
