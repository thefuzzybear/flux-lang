//! Statement parsing for the Flux language.
//!
//! Handles: assignment, if/elif/else, for loops, while loops, return statements,
//! expression statements, and block parsing.

use crate::error::Result;
use crate::lexer::{Span, Token};

use super::ast::*;
use super::parser_state::ParserState;

impl ParserState {
    /// Returns true if the current token can start a new statement.
    pub fn at_statement_start(&self) -> bool {
        matches!(
            self.peek(),
            Token::If
                | Token::For
                | Token::While
                | Token::Return
                | Token::Ident(_)
                | Token::Minus
                | Token::Not
                | Token::Bang
                | Token::Int(_)
                | Token::Float(_)
                | Token::String(_)
                | Token::True
                | Token::False
                | Token::Null
                | Token::OpenParen
                | Token::OpenBracket
        )
    }

    /// Returns true if the current token indicates statement/block end.
    pub fn at_statement_end(&self) -> bool {
        matches!(self.peek(), Token::CloseBrace | Token::Eof)
    }

    /// Returns true if the current token can start an expression.
    fn can_start_expr(&self) -> bool {
        matches!(
            self.peek(),
            Token::Ident(_)
                | Token::Int(_)
                | Token::Float(_)
                | Token::String(_)
                | Token::True
                | Token::False
                | Token::Null
                | Token::Minus
                | Token::Not
                | Token::Bang
                | Token::OpenParen
                | Token::OpenBracket
                | Token::SelfKw
                | Token::Match
        )
    }

    /// Parse a single statement, dispatching on the current token.
    pub fn parse_statement(&mut self) -> Result<Stmt> {
        match self.peek() {
            Token::If => self.parse_if_stmt(),
            Token::For => self.parse_for_loop(),
            Token::While => self.parse_while_loop(),
            Token::Return => self.parse_return_stmt(),
            _ => self.parse_assignment_or_expr_stmt(),
        }
    }

    /// Parse an assignment or expression statement.
    ///
    /// Parses an expression first, then checks if the next token is `Assign`.
    /// If so, the expression becomes the lvalue target. Otherwise it's an expression statement.
    fn parse_assignment_or_expr_stmt(&mut self) -> Result<Stmt> {
        let expr = self.parse_expr(0)?;

        if self.check(&Token::Assign) {
            // Assignment: expr = value
            self.advance(); // consume `=`
            let value = self.parse_expr(0)?;
            let span = Span::new(expr.span.start, value.span.end);
            Ok(Stmt::Assignment(Assignment {
                target: expr,
                value,
                span,
            }))
        } else {
            // Expression statement
            let span = expr.span;
            Ok(Stmt::Expr(ExprStmt { expr, span }))
        }
    }

    /// Parse an if/elif/else statement.
    fn parse_if_stmt(&mut self) -> Result<Stmt> {
        let start_span = self.current_span();
        self.advance(); // consume `if`

        let condition = self.with_struct_literal_forbidden(|state| state.parse_expr(0))?;
        let body = self.parse_block()?;

        let mut elif_branches = Vec::new();
        let mut else_body = None;

        // Parse elif branches
        while self.check(&Token::Elif) {
            let elif_start = self.current_span();
            self.advance(); // consume `elif`
            let elif_condition = self.with_struct_literal_forbidden(|state| state.parse_expr(0))?;
            let elif_body = self.parse_block()?;
            let elif_span = self.span_from(elif_start);
            elif_branches.push(ElifBranch {
                condition: elif_condition,
                body: elif_body,
                span: elif_span,
            });
        }

        // Parse optional else branch
        if self.check(&Token::Else) {
            self.advance(); // consume `else`
            else_body = Some(self.parse_block()?);
        }

        let span = self.span_from(start_span);
        Ok(Stmt::If(IfStmt {
            condition,
            body,
            elif_branches,
            else_body,
            span,
        }))
    }

    /// Parse a for loop: `for variable in iterable { body }`
    fn parse_for_loop(&mut self) -> Result<Stmt> {
        let start_span = self.current_span();
        self.advance(); // consume `for`

        let (variable, _) = self.expect_ident()?;

        // Now uses dedicated token instead of string matching
        self.expect(&Token::In)?;

        let iterable = self.with_struct_literal_forbidden(|state| state.parse_expr(0))?;
        let body = self.parse_block()?;
        let span = self.span_from(start_span);

        Ok(Stmt::For(ForLoop {
            variable,
            iterable,
            body,
            span,
        }))
    }

    /// Parse a while loop: `while condition { body }`
    fn parse_while_loop(&mut self) -> Result<Stmt> {
        let start_span = self.current_span();
        self.advance(); // consume `while`

        let condition = self.with_struct_literal_forbidden(|state| state.parse_expr(0))?;
        let body = self.parse_block()?;
        let span = self.span_from(start_span);

        Ok(Stmt::While(WhileLoop {
            condition,
            body,
            span,
        }))
    }

    /// Parse a return statement with optional value.
    fn parse_return_stmt(&mut self) -> Result<Stmt> {
        let start_span = self.current_span();
        self.advance(); // consume `return`

        let value = if self.can_start_expr() {
            Some(self.parse_expr(0)?)
        } else {
            None
        };

        let span = self.span_from(start_span);
        Ok(Stmt::Return(ReturnStmt { value, span }))
    }

    /// Parse a block: consumes `{`, parses statements until `}`, consumes `}`.
    pub fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.expect(&Token::OpenBrace)?;
        let mut stmts = Vec::new();
        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            stmts.push(self.parse_statement()?);
        }
        self.expect(&Token::CloseBrace)?;
        Ok(stmts)
    }
}


#[cfg(test)]
mod tests {
    use crate::lexer::{Span, SpannedToken, Token};

    use super::super::ast::*;
    use super::ParserState;

    /// Helper to create SpannedTokens with auto-calculated spans.
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
            result.push(SpannedToken {
                token,
                span: Span::new(pos, pos + len),
            });
            pos += len + 1;
        }
        result
    }

    /// Helper to parse a single statement from tokens.
    fn parse_stmt(tokens: Vec<Token>) -> crate::error::Result<Stmt> {
        let spanned = make_tokens(tokens);
        let mut state = ParserState::new(spanned)?;
        state.parse_statement()
    }

    /// Helper to parse a block from tokens (expects tokens to start with `{`).
    fn parse_block(tokens: Vec<Token>) -> crate::error::Result<Vec<Stmt>> {
        let spanned = make_tokens(tokens);
        let mut state = ParserState::new(spanned)?;
        state.parse_block()
    }

    // ===== 1. Simple assignment: x = 5 =====

    #[test]
    fn assignment_simple_ident() {
        // x = 5
        let stmt = parse_stmt(vec![
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(5),
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::Assignment(assign) => {
                assert_eq!(assign.target.kind, ExprKind::Ident("x".to_string()));
                assert_eq!(assign.value.kind, ExprKind::IntLiteral(5));
            }
            _ => panic!("Expected Assignment, got {:?}", stmt),
        }
    }

    // ===== 2. Member access assignment: obj.field = value =====

    #[test]
    fn assignment_member_access() {
        // obj.field = value
        let stmt = parse_stmt(vec![
            Token::Ident("obj".to_string()),
            Token::Dot,
            Token::Ident("field".to_string()),
            Token::Assign,
            Token::Ident("value".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::Assignment(assign) => {
                match &assign.target.kind {
                    ExprKind::MemberAccess { object, field } => {
                        assert_eq!(object.kind, ExprKind::Ident("obj".to_string()));
                        assert_eq!(field, "field");
                    }
                    _ => panic!("Expected MemberAccess target, got {:?}", assign.target.kind),
                }
                assert_eq!(assign.value.kind, ExprKind::Ident("value".to_string()));
            }
            _ => panic!("Expected Assignment, got {:?}", stmt),
        }
    }

    // ===== 3. Index access assignment: arr[0] = value =====

    #[test]
    fn assignment_index_access() {
        // arr[0] = value
        let stmt = parse_stmt(vec![
            Token::Ident("arr".to_string()),
            Token::OpenBracket,
            Token::Int(0),
            Token::CloseBracket,
            Token::Assign,
            Token::Ident("value".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::Assignment(assign) => {
                match &assign.target.kind {
                    ExprKind::IndexAccess { object, index } => {
                        assert_eq!(object.kind, ExprKind::Ident("arr".to_string()));
                        assert_eq!(index.kind, ExprKind::IntLiteral(0));
                    }
                    _ => panic!("Expected IndexAccess target, got {:?}", assign.target.kind),
                }
                assert_eq!(assign.value.kind, ExprKind::Ident("value".to_string()));
            }
            _ => panic!("Expected Assignment, got {:?}", stmt),
        }
    }

    // ===== 4. If with condition, body, no elif, no else =====

    #[test]
    fn if_simple_no_elif_no_else() {
        // if cond { x = 1 }
        let stmt = parse_stmt(vec![
            Token::If,
            Token::Ident("cond".to_string()),
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::If(if_stmt) => {
                assert_eq!(if_stmt.condition.kind, ExprKind::Ident("cond".to_string()));
                assert_eq!(if_stmt.body.len(), 1);
                assert!(if_stmt.elif_branches.is_empty());
                assert!(if_stmt.else_body.is_none());
            }
            _ => panic!("Expected If, got {:?}", stmt),
        }
    }

    // ===== 5. If with elif branch =====

    #[test]
    fn if_with_elif() {
        // if cond { } elif cond2 { }
        let stmt = parse_stmt(vec![
            Token::If,
            Token::Ident("cond".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Elif,
            Token::Ident("cond2".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::If(if_stmt) => {
                assert_eq!(if_stmt.condition.kind, ExprKind::Ident("cond".to_string()));
                assert!(if_stmt.body.is_empty());
                assert_eq!(if_stmt.elif_branches.len(), 1);
                assert_eq!(
                    if_stmt.elif_branches[0].condition.kind,
                    ExprKind::Ident("cond2".to_string())
                );
                assert!(if_stmt.else_body.is_none());
            }
            _ => panic!("Expected If, got {:?}", stmt),
        }
    }

    // ===== 6. If with else body =====

    #[test]
    fn if_with_else() {
        // if cond { } else { }
        let stmt = parse_stmt(vec![
            Token::If,
            Token::Ident("cond".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Else,
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::If(if_stmt) => {
                assert_eq!(if_stmt.condition.kind, ExprKind::Ident("cond".to_string()));
                assert!(if_stmt.body.is_empty());
                assert!(if_stmt.elif_branches.is_empty());
                assert!(if_stmt.else_body.is_some());
                assert!(if_stmt.else_body.unwrap().is_empty());
            }
            _ => panic!("Expected If, got {:?}", stmt),
        }
    }

    // ===== 7. Full if/elif/else chain =====

    #[test]
    fn if_elif_else_full_chain() {
        // if cond { } elif cond2 { } else { }
        let stmt = parse_stmt(vec![
            Token::If,
            Token::Ident("cond".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Elif,
            Token::Ident("cond2".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Else,
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::If(if_stmt) => {
                assert_eq!(if_stmt.condition.kind, ExprKind::Ident("cond".to_string()));
                assert_eq!(if_stmt.elif_branches.len(), 1);
                assert_eq!(
                    if_stmt.elif_branches[0].condition.kind,
                    ExprKind::Ident("cond2".to_string())
                );
                assert!(if_stmt.else_body.is_some());
            }
            _ => panic!("Expected If, got {:?}", stmt),
        }
    }

    // ===== 8. For loop =====

    #[test]
    fn for_loop_with_iterable() {
        // for x in items { x = 1 }
        let stmt = parse_stmt(vec![
            Token::For,
            Token::Ident("x".to_string()),
            Token::In,
            Token::Ident("items".to_string()),
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::For(for_loop) => {
                assert_eq!(for_loop.variable, "x");
                assert_eq!(for_loop.iterable.kind, ExprKind::Ident("items".to_string()));
                assert_eq!(for_loop.body.len(), 1);
            }
            _ => panic!("Expected ForLoop, got {:?}", stmt),
        }
    }

    // ===== 9. While loop =====

    #[test]
    fn while_loop_with_condition() {
        // while cond { x = 1 }
        let stmt = parse_stmt(vec![
            Token::While,
            Token::Ident("cond".to_string()),
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::While(while_loop) => {
                assert_eq!(while_loop.condition.kind, ExprKind::Ident("cond".to_string()));
                assert_eq!(while_loop.body.len(), 1);
            }
            _ => panic!("Expected WhileLoop, got {:?}", stmt),
        }
    }

    // ===== 10. Return with no value =====

    #[test]
    fn return_no_value() {
        // return followed by CloseBrace (end of block)
        let stmt = parse_stmt(vec![
            Token::Return,
            Token::CloseBrace,
        ])
        .unwrap();

        match stmt {
            Stmt::Return(ret) => {
                assert!(ret.value.is_none());
            }
            _ => panic!("Expected ReturnStmt, got {:?}", stmt),
        }
    }

    // ===== 11. Return with value =====

    #[test]
    fn return_with_value() {
        // return x
        let stmt = parse_stmt(vec![
            Token::Return,
            Token::Ident("x".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::Return(ret) => {
                assert!(ret.value.is_some());
                assert_eq!(ret.value.unwrap().kind, ExprKind::Ident("x".to_string()));
            }
            _ => panic!("Expected ReturnStmt, got {:?}", stmt),
        }
    }

    // ===== 12. Expression statement (bare function call) =====

    #[test]
    fn expr_stmt_function_call() {
        // f()
        let stmt = parse_stmt(vec![
            Token::Ident("f".to_string()),
            Token::OpenParen,
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match stmt {
            Stmt::Expr(expr_stmt) => {
                match &expr_stmt.expr.kind {
                    ExprKind::FunctionCall { function, args } => {
                        assert_eq!(function.kind, ExprKind::Ident("f".to_string()));
                        assert!(args.is_empty());
                    }
                    _ => panic!("Expected FunctionCall, got {:?}", expr_stmt.expr.kind),
                }
            }
            _ => panic!("Expected ExprStmt, got {:?}", stmt),
        }
    }

    // ===== 13. Multiple sequential statements (block boundary detection) =====

    #[test]
    fn block_multiple_statements() {
        // { x = 1  y = 2 }
        let stmts = parse_block(vec![
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::Ident("y".to_string()),
            Token::Assign,
            Token::Int(2),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(stmts.len(), 2);
        match &stmts[0] {
            Stmt::Assignment(assign) => {
                assert_eq!(assign.target.kind, ExprKind::Ident("x".to_string()));
                assert_eq!(assign.value.kind, ExprKind::IntLiteral(1));
            }
            _ => panic!("Expected Assignment for first stmt, got {:?}", stmts[0]),
        }
        match &stmts[1] {
            Stmt::Assignment(assign) => {
                assert_eq!(assign.target.kind, ExprKind::Ident("y".to_string()));
                assert_eq!(assign.value.kind, ExprKind::IntLiteral(2));
            }
            _ => panic!("Expected Assignment for second stmt, got {:?}", stmts[1]),
        }
    }

    // ===== 14. Error: missing `{` after if condition =====

    #[test]
    fn error_missing_open_brace_after_if() {
        // if cond x = 1 }  (missing `{`)
        let result = parse_stmt(vec![
            Token::If,
            Token::Ident("cond".to_string()),
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            Token::Eof,
        ]);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("OpenBrace"), "Error should mention OpenBrace, got: {msg}");
    }

    // ===== 15. Error: missing `in` in for loop =====

    #[test]
    fn error_missing_in_keyword_for_loop() {
        // for x items { }  (missing `in`)
        let result = parse_stmt(vec![
            Token::For,
            Token::Ident("x".to_string()),
            Token::Ident("items".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ]);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("In"),
            "Error should mention In keyword, got: {msg}"
        );
    }
}
