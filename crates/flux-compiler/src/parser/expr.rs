//! Expression parsing using a Pratt (top-down operator precedence) parser.
//!
//! Binding power table:
//! | Level | Operators               | Left BP, Right BP |
//! |-------|-------------------------|-------------------|
//! | 1     | `or`, `||`              | (1, 2)            |
//! | 2     | `and`, `&&`             | (3, 4)            |
//! | 3     | `==`, `!=`              | (5, 6)            |
//! | 4     | `<`, `<=`, `>`, `>=`    | (7, 8)            |
//! | 5     | `+`, `-`               | (9, 10)           |
//! | 6     | `*`, `/`, `%`           | (11, 12)          |
//! | 7     | Unary `-`, `not`, `!`   | (—, 13)           |
//! | 8     | `.field`, `[idx]`, `()` | POSTFIX_BP = 15   |

use crate::error::Result;
use crate::lexer::{Span, Token};

use super::ast::*;
use super::parser_state::ParserState;

/// Postfix binding power for call, index, and member access.
const POSTFIX_BP: u8 = 15;

/// Returns (left_bp, right_bp) for infix binary operators.
fn infix_binding_power(op: &Token) -> Option<(u8, u8)> {
    match op {
        Token::Or | Token::OrOr => Some((1, 2)),
        Token::And | Token::AndAnd => Some((3, 4)),
        Token::Eq | Token::Ne => Some((5, 6)),
        Token::Lt | Token::Le | Token::Gt | Token::Ge => Some((7, 8)),
        Token::Plus | Token::Minus => Some((9, 10)),
        Token::Star | Token::Slash | Token::Percent => Some((11, 12)),
        _ => None,
    }
}

/// Returns the right binding power for prefix (unary) operators.
fn prefix_binding_power(op: &Token) -> Option<u8> {
    match op {
        Token::Minus | Token::Not | Token::Bang => Some(13),
        _ => None,
    }
}

/// Convert a token to its corresponding binary operator.
fn token_to_binop(token: &Token) -> BinOp {
    match token {
        Token::Plus => BinOp::Add,
        Token::Minus => BinOp::Sub,
        Token::Star => BinOp::Mul,
        Token::Slash => BinOp::Div,
        Token::Percent => BinOp::Mod,
        Token::Eq => BinOp::Eq,
        Token::Ne => BinOp::Ne,
        Token::Lt => BinOp::Lt,
        Token::Le => BinOp::Le,
        Token::Gt => BinOp::Gt,
        Token::Ge => BinOp::Ge,
        Token::And | Token::AndAnd => BinOp::And,
        Token::Or | Token::OrOr => BinOp::Or,
        _ => unreachable!("not a binary operator: {:?}", token),
    }
}

/// Convert a token to its corresponding unary operator.
fn token_to_unaryop(token: &Token) -> UnaryOp {
    match token {
        Token::Minus => UnaryOp::Neg,
        Token::Not | Token::Bang => UnaryOp::Not,
        _ => unreachable!("not a unary operator: {:?}", token),
    }
}

impl ParserState {
    /// Parse an expression with the given minimum binding power.
    pub fn parse_expr(&mut self, min_bp: u8) -> Result<Expr> {
        let mut lhs = self.parse_prefix()?;

        loop {
            // Check for postfix operators (call, index, member access)
            match self.peek() {
                Token::OpenParen => {
                    if POSTFIX_BP < min_bp {
                        break;
                    }
                    lhs = self.parse_call_expr(lhs)?;
                    continue;
                }
                Token::OpenBracket => {
                    if POSTFIX_BP < min_bp {
                        break;
                    }
                    lhs = self.parse_index_expr(lhs)?;
                    continue;
                }
                Token::Dot => {
                    if POSTFIX_BP < min_bp {
                        break;
                    }
                    lhs = self.parse_dot_expr(lhs)?;
                    continue;
                }
                _ => {}
            }

            // Check for infix binary operators
            let op_token = self.peek().clone();
            if let Some((l_bp, r_bp)) = infix_binding_power(&op_token) {
                if l_bp < min_bp {
                    break;
                }
                self.advance(); // consume operator
                let rhs = self.parse_expr(r_bp)?;
                let span = Span::new(lhs.span.start, rhs.span.end);
                let bin_op = token_to_binop(&op_token);
                lhs = Expr {
                    kind: ExprKind::BinaryOp {
                        left: Box::new(lhs),
                        op: bin_op,
                        right: Box::new(rhs),
                    },
                    span,
                };
                continue;
            }

            break;
        }

        Ok(lhs)
    }

    /// Check if an identifier name looks like an enum name.
    /// Enum names start with an uppercase letter (PascalCase).
    fn is_enum_name_like(name: &str) -> bool {
        name.chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
    }

    /// Parse a prefix expression (literal, identifier, unary op, grouped expr, list, match).
    fn parse_prefix(&mut self) -> Result<Expr> {
        let token = self.peek().clone();
        match &token {
            // Unary operators
            Token::Minus | Token::Not | Token::Bang => {
                let op_span = self.current_span();
                self.advance();
                let r_bp = prefix_binding_power(&token).unwrap();
                let operand = self.parse_expr(r_bp)?;
                let span = Span::new(op_span.start, operand.span.end);
                let unary_op = token_to_unaryop(&token);
                Ok(Expr {
                    kind: ExprKind::UnaryOp {
                        op: unary_op,
                        operand: Box::new(operand),
                    },
                    span,
                })
            }
            // Match expression
            Token::Match => self.parse_match_expr(),
            // Grouped expression
            Token::OpenParen => self.parse_grouped_expr(),
            // List literal
            Token::OpenBracket => self.parse_list_literal(),
            // Literals
            Token::Int(_) | Token::Float(_) | Token::String(_) | Token::True | Token::False
            | Token::Null => self.parse_literal(),
            // Identifier
            Token::Ident(_) => self.parse_ident_expr(),
            _ => Err(self.error_expected("expression")),
        }
    }

    /// Parse a literal expression (Int, Float, String, Bool, Null).
    fn parse_literal(&mut self) -> Result<Expr> {
        let spanned = self.peek_spanned().clone();
        let span = spanned.span;
        let kind = match &spanned.token {
            Token::Int(v) => ExprKind::IntLiteral(*v),
            Token::Float(v) => ExprKind::FloatLiteral(*v),
            Token::String(v) => ExprKind::StringLiteral(v.clone()),
            Token::True => ExprKind::BoolLiteral(true),
            Token::False => ExprKind::BoolLiteral(false),
            Token::Null => ExprKind::NullLiteral,
            _ => return Err(self.error_expected("literal")),
        };
        self.advance();
        Ok(Expr { kind, span })
    }

    /// Parse an identifier expression, or a struct literal if the
    /// identifier is immediately followed by `{` and struct literal
    /// parsing isn't currently suppressed (see `forbid_struct_literal`).
    fn parse_ident_expr(&mut self) -> Result<Expr> {
        let (name, span) = self.expect_ident()?;

        if self.check(&Token::OpenBrace) && !self.struct_literal_forbidden() {
            return self.parse_struct_literal(name, span);
        }

        Ok(Expr {
            kind: ExprKind::Ident(name),
            span,
        })
    }

    /// Parse a struct literal: `StructName { field = value, ... }`.
    /// `struct_name` and `name_span` are the already-consumed leading
    /// identifier; the cursor is positioned at the `{`.
    fn parse_struct_literal(&mut self, struct_name: String, name_span: Span) -> Result<Expr> {
        self.advance(); // consume `{`

        let mut fields = Vec::new();
        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            let (field_name, _) = self.expect_ident()?;
            self.expect(&Token::Assign)?;
            // Field values may themselves contain struct literals (e.g.
            // nested `Outer { inner = Inner { x = 1 } }`), so allow them
            // here regardless of the enclosing suppression state.
            let value = self.with_struct_literal_allowed(|state| state.parse_expr(0))?;
            fields.push((field_name, value));

            if self.check(&Token::Comma) {
                self.advance(); // consume comma
            } else {
                break;
            }
        }

        let end_span = self.expect(&Token::CloseBrace)?;
        let span = Span::new(name_span.start, end_span.end);
        Ok(Expr {
            kind: ExprKind::StructLiteral { struct_name, fields },
            span,
        })
    }

    /// Parse a function call expression: `lhs(arg1, arg2, ...)`
    fn parse_call_expr(&mut self, function: Expr) -> Result<Expr> {
        self.advance(); // consume OpenParen
        let mut args = Vec::new();

        // Arguments are enclosed in parens, so a `{` here can no longer be
        // confused with a statement body — allow struct literals again.
        self.with_struct_literal_allowed(|state| {
            if !state.check(&Token::CloseParen) {
                args.push(state.parse_expr(0)?);
                while state.check(&Token::Comma) {
                    state.advance(); // consume comma
                    if state.check(&Token::CloseParen) {
                        break; // trailing comma
                    }
                    args.push(state.parse_expr(0)?);
                }
            }
            Ok(())
        })?;

        let end_span = self.expect(&Token::CloseParen)?;
        let span = Span::new(function.span.start, end_span.end);
        Ok(Expr {
            kind: ExprKind::FunctionCall {
                function: Box::new(function),
                args,
            },
            span,
        })
    }

    /// Parse a dot expression, distinguishing:
    /// - Enum construction: `EnumName.VariantName` or `EnumName.VariantName(args)`
    /// - Method call: `receiver.method(args)`
    /// - Member access: `object.field`
    fn parse_dot_expr(&mut self, lhs: Expr) -> Result<Expr> {
        self.advance(); // consume Dot
        let (name, name_span) = self.expect_ident()?;

        // Check if lhs is an identifier that looks like an enum name
        let is_enum_construction = match &lhs.kind {
            ExprKind::Ident(ident_name) => Self::is_enum_name_like(ident_name),
            _ => false,
        };

        // Check for args following: `EnumName.VariantName(args)` or `receiver.method(args)`
        if self.check(&Token::OpenParen) {
            self.advance(); // consume OpenParen
            let mut args = Vec::new();

            // Arguments are enclosed in parens, so a `{` here can no
            // longer be confused with a statement body — allow struct
            // literals again.
            self.with_struct_literal_allowed(|state| {
                if !state.check(&Token::CloseParen) {
                    args.push(state.parse_expr(0)?);
                    while state.check(&Token::Comma) {
                        state.advance();
                        if state.check(&Token::CloseParen) {
                            break; // trailing comma
                        }
                        args.push(state.parse_expr(0)?);
                    }
                }
                Ok(())
            })?;

            let end_span = self.expect(&Token::CloseParen)?;

            // Distinguish enum construction from method call
            if is_enum_construction {
                let enum_name = match &lhs.kind {
                    ExprKind::Ident(name) => name.clone(),
                    _ => unreachable!(),
                };
                let span = Span::new(lhs.span.start, end_span.end);
                Ok(Expr {
                    kind: ExprKind::EnumConstruction {
                        enum_name,
                        variant_name: name,
                        args,
                    },
                    span,
                })
            } else {
                let span = Span::new(lhs.span.start, end_span.end);
                Ok(Expr {
                    kind: ExprKind::MethodCall {
                        receiver: Box::new(lhs),
                        method: name,
                        args,
                    },
                    span,
                })
            }
        } else {
            // No args: either enum construction or member access
            if is_enum_construction {
                // `EnumName.VariantName` (unit variant)
                let enum_name = match &lhs.kind {
                    ExprKind::Ident(name) => name.clone(),
                    _ => unreachable!(),
                };
                let span = Span::new(lhs.span.start, name_span.end);
                Ok(Expr {
                    kind: ExprKind::EnumConstruction {
                        enum_name,
                        variant_name: name,
                        args: Vec::new(),
                    },
                    span,
                })
            } else {
                // Member access
                let span = Span::new(lhs.span.start, name_span.end);
                Ok(Expr {
                    kind: ExprKind::MemberAccess {
                        object: Box::new(lhs),
                        field: name,
                    },
                    span,
                })
            }
        }
    }

    /// Parse an index access expression: `lhs[index]`
    fn parse_index_expr(&mut self, lhs: Expr) -> Result<Expr> {
        self.advance(); // consume OpenBracket
        // Enclosed in brackets, so a `{` here can no longer be confused
        // with a statement body — allow struct literals again.
        let index = self.with_struct_literal_allowed(|state| state.parse_expr(0))?;
        let end_span = self.expect(&Token::CloseBracket)?;
        let span = Span::new(lhs.span.start, end_span.end);
        Ok(Expr {
            kind: ExprKind::IndexAccess {
                object: Box::new(lhs),
                index: Box::new(index),
            },
            span,
        })
    }

    /// Parse a parenthesized (grouped) expression: `(expr)`
    fn parse_grouped_expr(&mut self) -> Result<Expr> {
        self.advance(); // consume OpenParen
        if self.check(&Token::CloseParen) {
            return Err(self.error_expected("expression"));
        }
        // Enclosed in parens, so a `{` here can no longer be confused with
        // a statement body — allow struct literals again.
        let expr = self.with_struct_literal_allowed(|state| state.parse_expr(0))?;
        self.expect(&Token::CloseParen)?;
        Ok(expr)
    }

    /// Parse a list literal: `[elem1, elem2, ...]`
    fn parse_list_literal(&mut self) -> Result<Expr> {
        let start_span = self.current_span();
        self.advance(); // consume OpenBracket
        let mut elements = Vec::new();

        // Enclosed in brackets, so a `{` here can no longer be confused
        // with a statement body — allow struct literals again.
        self.with_struct_literal_allowed(|state| {
            if !state.check(&Token::CloseBracket) {
                elements.push(state.parse_expr(0)?);
                while state.check(&Token::Comma) {
                    state.advance();
                    if state.check(&Token::CloseBracket) {
                        break; // trailing comma
                    }
                    elements.push(state.parse_expr(0)?);
                }
            }
            Ok(())
        })?;

        let end_span = self.expect(&Token::CloseBracket)?;
        let span = Span::new(start_span.start, end_span.end);
        Ok(Expr {
            kind: ExprKind::ListLiteral(elements),
            span,
        })
    }

    /// Parse a match expression: `match scrutinee { Pattern => { body }, ... }`
    fn parse_match_expr(&mut self) -> Result<Expr> {
        let start_span = self.current_span();
        self.expect(&Token::Match)?;

        // Parse the scrutinee expression
        let scrutinee = self.with_struct_literal_forbidden(|state| state.parse_expr(0))?;

        // Expect `{` to start the match arms
        self.expect(&Token::OpenBrace)?;

        let mut arms = Vec::new();
        while !self.check(&Token::CloseBrace) && !self.at_eof() {
            let arm = self.parse_match_arm()?;
            arms.push(arm);
        }

        let end_span = self.expect(&Token::CloseBrace)?;
        let span = Span::new(start_span.start, end_span.end);
        Ok(Expr {
            kind: ExprKind::Match(MatchExpr {
                scrutinee: Box::new(scrutinee),
                arms,
                span,
            }),
            span,
        })
    }

    /// Parse a single match arm: `Pattern => { body }`
    fn parse_match_arm(&mut self) -> Result<MatchArm> {
        let start_span = self.current_span();

        // Parse the pattern
        let pattern = self.parse_pattern()?;

        // Expect `=>` (fat arrow)
        self.expect(&Token::FatArrow)?;

        // Parse the body - either a block `{ ... }` or a single expression
        let body = if self.check(&Token::OpenBrace) {
            self.advance(); // consume `{`
            let mut stmts = Vec::new();
            while !self.check(&Token::CloseBrace) && !self.at_eof() {
                stmts.push(self.parse_statement()?);
            }
            self.expect(&Token::CloseBrace)?;
            stmts
        } else {
            // Single expression statement
            let expr = self.parse_expr(0)?;
            let span = expr.span;
            vec![Stmt::Expr(ExprStmt { expr, span })]
        };

        let span = self.span_from(start_span);
        Ok(MatchArm {
            pattern,
            body,
            span,
        })
    }

    /// Parse a pattern: `EnumName.VariantName(binding1, binding2)` or `_`
    fn parse_pattern(&mut self) -> Result<Pattern> {
        let start_span = self.current_span();

        // Check for wildcard pattern: `_` is parsed as an identifier
        if let Token::Ident(name) = self.peek() {
            if name == "_" {
                self.advance();
                return Ok(Pattern::Wildcard {
                    span: start_span,
                });
            }
        }

        // Otherwise, parse a variant pattern: EnumName.VariantName(bindings...)
        let (enum_name, _) = self.expect_ident()?;
        self.expect(&Token::Dot)?;
        let (variant_name, _) = self.expect_ident()?;

        // Check for bindings: `(binding1, binding2, ...)`
        let mut bindings = Vec::new();
        if self.check(&Token::OpenParen) {
            self.advance(); // consume `(`

            if !self.check(&Token::CloseParen) {
                let (binding, _) = self.expect_ident()?;
                bindings.push(binding);

                while self.check(&Token::Comma) {
                    self.advance(); // consume `,`
                    if self.check(&Token::CloseParen) {
                        break; // trailing comma
                    }
                    let (binding, _) = self.expect_ident()?;
                    bindings.push(binding);
                }
            }

            self.expect(&Token::CloseParen)?;
        }

        let span = self.span_from(start_span);
        Ok(Pattern::Variant {
            enum_name,
            variant_name,
            bindings,
            span,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::error::Result;
    use crate::lexer::{Span, SpannedToken, Token};

    use super::super::ast::*;
    use super::super::parser_state::ParserState;

    /// Helper to create a SpannedToken with auto-calculated spans
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
                _ => 1, // operators, delimiters, keywords
            };
            result.push(SpannedToken {
                token,
                span: Span::new(pos, pos + len),
            });
            pos += len + 1; // +1 for space
        }
        result
    }

    /// Helper to parse an expression from tokens
    fn parse_expr(tokens: Vec<Token>) -> Result<Expr> {
        let spanned = make_tokens(tokens);
        let mut state = ParserState::new(spanned)?;
        state.parse_expr(0)
    }

    // ===== 1. Literal parsing =====

    #[test]
    fn literal_int() {
        let expr = parse_expr(vec![Token::Int(42), Token::Eof]).unwrap();
        assert_eq!(expr.kind, ExprKind::IntLiteral(42));
    }

    #[test]
    fn literal_float() {
        let expr = parse_expr(vec![Token::Float(3.14), Token::Eof]).unwrap();
        assert_eq!(expr.kind, ExprKind::FloatLiteral(3.14));
    }

    #[test]
    fn literal_string() {
        let expr = parse_expr(vec![Token::String("hello".to_string()), Token::Eof]).unwrap();
        assert_eq!(expr.kind, ExprKind::StringLiteral("hello".to_string()));
    }

    #[test]
    fn literal_bool_true() {
        let expr = parse_expr(vec![Token::True, Token::Eof]).unwrap();
        assert_eq!(expr.kind, ExprKind::BoolLiteral(true));
    }

    #[test]
    fn literal_bool_false() {
        let expr = parse_expr(vec![Token::False, Token::Eof]).unwrap();
        assert_eq!(expr.kind, ExprKind::BoolLiteral(false));
    }

    #[test]
    fn literal_null() {
        let expr = parse_expr(vec![Token::Null, Token::Eof]).unwrap();
        assert_eq!(expr.kind, ExprKind::NullLiteral);
    }

    // ===== 2. Binary operations with correct precedence =====

    #[test]
    fn binary_op_mul_higher_than_add() {
        // a + b * c → BinaryOp(a, Add, BinaryOp(b, Mul, c))
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::Plus,
            Token::Ident("b".to_string()),
            Token::Star,
            Token::Ident("c".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::BinaryOp { left, op, right } => {
                assert_eq!(*op, BinOp::Add);
                assert_eq!(left.kind, ExprKind::Ident("a".to_string()));
                // right should be BinaryOp(b, Mul, c)
                match &right.kind {
                    ExprKind::BinaryOp { left: rl, op: rop, right: rr } => {
                        assert_eq!(*rop, BinOp::Mul);
                        assert_eq!(rl.kind, ExprKind::Ident("b".to_string()));
                        assert_eq!(rr.kind, ExprKind::Ident("c".to_string()));
                    }
                    _ => panic!("Expected BinaryOp(b, Mul, c), got {:?}", right.kind),
                }
            }
            _ => panic!("Expected BinaryOp at top level, got {:?}", expr.kind),
        }
    }

    // ===== 3. Left-associativity =====

    #[test]
    fn binary_op_left_associative() {
        // a - b - c → BinaryOp(BinaryOp(a, Sub, b), Sub, c)
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::Minus,
            Token::Ident("b".to_string()),
            Token::Minus,
            Token::Ident("c".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::BinaryOp { left, op, right } => {
                assert_eq!(*op, BinOp::Sub);
                assert_eq!(right.kind, ExprKind::Ident("c".to_string()));
                // left should be BinaryOp(a, Sub, b)
                match &left.kind {
                    ExprKind::BinaryOp { left: ll, op: lop, right: lr } => {
                        assert_eq!(*lop, BinOp::Sub);
                        assert_eq!(ll.kind, ExprKind::Ident("a".to_string()));
                        assert_eq!(lr.kind, ExprKind::Ident("b".to_string()));
                    }
                    _ => panic!("Expected BinaryOp(a, Sub, b), got {:?}", left.kind),
                }
            }
            _ => panic!("Expected BinaryOp at top level, got {:?}", expr.kind),
        }
    }

    // ===== 4. Unary operators =====

    #[test]
    fn unary_neg() {
        // -x → UnaryOp(Neg, Ident("x"))
        let expr = parse_expr(vec![
            Token::Minus,
            Token::Ident("x".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::UnaryOp { op, operand } => {
                assert_eq!(*op, UnaryOp::Neg);
                assert_eq!(operand.kind, ExprKind::Ident("x".to_string()));
            }
            _ => panic!("Expected UnaryOp, got {:?}", expr.kind),
        }
    }

    #[test]
    fn unary_not_keyword() {
        // not y → UnaryOp(Not, Ident("y"))
        let expr = parse_expr(vec![
            Token::Not,
            Token::Ident("y".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::UnaryOp { op, operand } => {
                assert_eq!(*op, UnaryOp::Not);
                assert_eq!(operand.kind, ExprKind::Ident("y".to_string()));
            }
            _ => panic!("Expected UnaryOp, got {:?}", expr.kind),
        }
    }

    #[test]
    fn unary_bang() {
        // !z → UnaryOp(Not, Ident("z"))
        let expr = parse_expr(vec![
            Token::Bang,
            Token::Ident("z".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::UnaryOp { op, operand } => {
                assert_eq!(*op, UnaryOp::Not);
                assert_eq!(operand.kind, ExprKind::Ident("z".to_string()));
            }
            _ => panic!("Expected UnaryOp, got {:?}", expr.kind),
        }
    }

    // ===== 5. Chained unary (right-to-left nesting) =====

    #[test]
    fn chained_unary_not_not() {
        // not not x → UnaryOp(Not, UnaryOp(Not, Ident("x")))
        let expr = parse_expr(vec![
            Token::Not,
            Token::Not,
            Token::Ident("x".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::UnaryOp { op, operand } => {
                assert_eq!(*op, UnaryOp::Not);
                match &operand.kind {
                    ExprKind::UnaryOp { op: inner_op, operand: inner_operand } => {
                        assert_eq!(*inner_op, UnaryOp::Not);
                        assert_eq!(inner_operand.kind, ExprKind::Ident("x".to_string()));
                    }
                    _ => panic!("Expected inner UnaryOp, got {:?}", operand.kind),
                }
            }
            _ => panic!("Expected UnaryOp, got {:?}", expr.kind),
        }
    }

    // ===== 6. Function calls =====

    #[test]
    fn function_call_zero_args() {
        // f()
        let expr = parse_expr(vec![
            Token::Ident("f".to_string()),
            Token::OpenParen,
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::FunctionCall { function, args } => {
                assert_eq!(function.kind, ExprKind::Ident("f".to_string()));
                assert!(args.is_empty());
            }
            _ => panic!("Expected FunctionCall, got {:?}", expr.kind),
        }
    }

    #[test]
    fn function_call_one_arg() {
        // f(a)
        let expr = parse_expr(vec![
            Token::Ident("f".to_string()),
            Token::OpenParen,
            Token::Ident("a".to_string()),
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::FunctionCall { function, args } => {
                assert_eq!(function.kind, ExprKind::Ident("f".to_string()));
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].kind, ExprKind::Ident("a".to_string()));
            }
            _ => panic!("Expected FunctionCall, got {:?}", expr.kind),
        }
    }

    #[test]
    fn function_call_many_args() {
        // f(a, b)
        let expr = parse_expr(vec![
            Token::Ident("f".to_string()),
            Token::OpenParen,
            Token::Ident("a".to_string()),
            Token::Comma,
            Token::Ident("b".to_string()),
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::FunctionCall { function, args } => {
                assert_eq!(function.kind, ExprKind::Ident("f".to_string()));
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].kind, ExprKind::Ident("a".to_string()));
                assert_eq!(args[1].kind, ExprKind::Ident("b".to_string()));
            }
            _ => panic!("Expected FunctionCall, got {:?}", expr.kind),
        }
    }

    #[test]
    fn function_call_trailing_comma() {
        // f(a, b,)
        let expr = parse_expr(vec![
            Token::Ident("f".to_string()),
            Token::OpenParen,
            Token::Ident("a".to_string()),
            Token::Comma,
            Token::Ident("b".to_string()),
            Token::Comma,
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::FunctionCall { function, args } => {
                assert_eq!(function.kind, ExprKind::Ident("f".to_string()));
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].kind, ExprKind::Ident("a".to_string()));
                assert_eq!(args[1].kind, ExprKind::Ident("b".to_string()));
            }
            _ => panic!("Expected FunctionCall, got {:?}", expr.kind),
        }
    }

    // ===== 7. Method calls and member access =====

    #[test]
    fn member_access() {
        // a.b
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::Dot,
            Token::Ident("b".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::MemberAccess { object, field } => {
                assert_eq!(object.kind, ExprKind::Ident("a".to_string()));
                assert_eq!(field, "b");
            }
            _ => panic!("Expected MemberAccess, got {:?}", expr.kind),
        }
    }

    #[test]
    fn method_call() {
        // a.b()
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::Dot,
            Token::Ident("b".to_string()),
            Token::OpenParen,
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::MethodCall { receiver, method, args } => {
                assert_eq!(receiver.kind, ExprKind::Ident("a".to_string()));
                assert_eq!(method, "b");
                assert!(args.is_empty());
            }
            _ => panic!("Expected MethodCall, got {:?}", expr.kind),
        }
    }

    #[test]
    fn chained_member_access_and_method_call() {
        // a.b.c() → MethodCall(MemberAccess(a, b), c, [])
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::Dot,
            Token::Ident("b".to_string()),
            Token::Dot,
            Token::Ident("c".to_string()),
            Token::OpenParen,
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::MethodCall { receiver, method, args } => {
                assert_eq!(method, "c");
                assert!(args.is_empty());
                // receiver should be MemberAccess(a, b)
                match &receiver.kind {
                    ExprKind::MemberAccess { object, field } => {
                        assert_eq!(object.kind, ExprKind::Ident("a".to_string()));
                        assert_eq!(field, "b");
                    }
                    _ => panic!("Expected MemberAccess as receiver, got {:?}", receiver.kind),
                }
            }
            _ => panic!("Expected MethodCall, got {:?}", expr.kind),
        }
    }

    // ===== 8. Index access =====

    #[test]
    fn index_access() {
        // arr[0]
        let expr = parse_expr(vec![
            Token::Ident("arr".to_string()),
            Token::OpenBracket,
            Token::Int(0),
            Token::CloseBracket,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::IndexAccess { object, index } => {
                assert_eq!(object.kind, ExprKind::Ident("arr".to_string()));
                assert_eq!(index.kind, ExprKind::IntLiteral(0));
            }
            _ => panic!("Expected IndexAccess, got {:?}", expr.kind),
        }
    }

    #[test]
    fn index_access_negative() {
        // prices[-1] → IndexAccess(prices, UnaryOp(Neg, 1))
        let expr = parse_expr(vec![
            Token::Ident("prices".to_string()),
            Token::OpenBracket,
            Token::Minus,
            Token::Int(1),
            Token::CloseBracket,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::IndexAccess { object, index } => {
                assert_eq!(object.kind, ExprKind::Ident("prices".to_string()));
                match &index.kind {
                    ExprKind::UnaryOp { op, operand } => {
                        assert_eq!(*op, UnaryOp::Neg);
                        assert_eq!(operand.kind, ExprKind::IntLiteral(1));
                    }
                    _ => panic!("Expected UnaryOp(Neg, 1), got {:?}", index.kind),
                }
            }
            _ => panic!("Expected IndexAccess, got {:?}", expr.kind),
        }
    }

    // ===== 9. Grouped expressions =====

    #[test]
    fn grouped_expression() {
        // (a + b) * c → BinaryOp(BinaryOp(a, Add, b), Mul, c)
        let expr = parse_expr(vec![
            Token::OpenParen,
            Token::Ident("a".to_string()),
            Token::Plus,
            Token::Ident("b".to_string()),
            Token::CloseParen,
            Token::Star,
            Token::Ident("c".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::BinaryOp { left, op, right } => {
                assert_eq!(*op, BinOp::Mul);
                assert_eq!(right.kind, ExprKind::Ident("c".to_string()));
                // left should be the grouped (a + b)
                match &left.kind {
                    ExprKind::BinaryOp { left: ll, op: lop, right: lr } => {
                        assert_eq!(*lop, BinOp::Add);
                        assert_eq!(ll.kind, ExprKind::Ident("a".to_string()));
                        assert_eq!(lr.kind, ExprKind::Ident("b".to_string()));
                    }
                    _ => panic!("Expected BinaryOp(a, Add, b), got {:?}", left.kind),
                }
            }
            _ => panic!("Expected BinaryOp at top level, got {:?}", expr.kind),
        }
    }

    // ===== 10. List literals =====

    #[test]
    fn list_literal_empty() {
        // []
        let expr = parse_expr(vec![
            Token::OpenBracket,
            Token::CloseBracket,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::ListLiteral(elements) => {
                assert!(elements.is_empty());
            }
            _ => panic!("Expected ListLiteral, got {:?}", expr.kind),
        }
    }

    #[test]
    fn list_literal_multiple_elements() {
        // [1, 2, 3]
        let expr = parse_expr(vec![
            Token::OpenBracket,
            Token::Int(1),
            Token::Comma,
            Token::Int(2),
            Token::Comma,
            Token::Int(3),
            Token::CloseBracket,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::ListLiteral(elements) => {
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0].kind, ExprKind::IntLiteral(1));
                assert_eq!(elements[1].kind, ExprKind::IntLiteral(2));
                assert_eq!(elements[2].kind, ExprKind::IntLiteral(3));
            }
            _ => panic!("Expected ListLiteral, got {:?}", expr.kind),
        }
    }

    #[test]
    fn list_literal_trailing_comma() {
        // [1,]
        let expr = parse_expr(vec![
            Token::OpenBracket,
            Token::Int(1),
            Token::Comma,
            Token::CloseBracket,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::ListLiteral(elements) => {
                assert_eq!(elements.len(), 1);
                assert_eq!(elements[0].kind, ExprKind::IntLiteral(1));
            }
            _ => panic!("Expected ListLiteral, got {:?}", expr.kind),
        }
    }

    // ===== 11. and/&& and or/|| equivalence =====

    #[test]
    fn and_keyword_produces_binop_and() {
        // a and b
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::And,
            Token::Ident("b".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::BinaryOp { op, .. } => assert_eq!(*op, BinOp::And),
            _ => panic!("Expected BinaryOp, got {:?}", expr.kind),
        }
    }

    #[test]
    fn andand_produces_binop_and() {
        // a && b
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::AndAnd,
            Token::Ident("b".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::BinaryOp { op, .. } => assert_eq!(*op, BinOp::And),
            _ => panic!("Expected BinaryOp, got {:?}", expr.kind),
        }
    }

    #[test]
    fn or_keyword_produces_binop_or() {
        // a or b
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::Or,
            Token::Ident("b".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::BinaryOp { op, .. } => assert_eq!(*op, BinOp::Or),
            _ => panic!("Expected BinaryOp, got {:?}", expr.kind),
        }
    }

    #[test]
    fn oror_produces_binop_or() {
        // a || b
        let expr = parse_expr(vec![
            Token::Ident("a".to_string()),
            Token::OrOr,
            Token::Ident("b".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::BinaryOp { op, .. } => assert_eq!(*op, BinOp::Or),
            _ => panic!("Expected BinaryOp, got {:?}", expr.kind),
        }
    }

    // ===== 12. Error cases =====

    #[test]
    fn error_missing_close_paren_in_call() {
        // f(a  — missing )
        let result = parse_expr(vec![
            Token::Ident("f".to_string()),
            Token::OpenParen,
            Token::Ident("a".to_string()),
            Token::Eof,
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn error_missing_close_bracket_in_list() {
        // [1, 2  — missing ]
        let result = parse_expr(vec![
            Token::OpenBracket,
            Token::Int(1),
            Token::Comma,
            Token::Int(2),
            Token::Eof,
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn error_empty_parens_as_expression() {
        // () — empty parens are not a valid expression
        let result = parse_expr(vec![
            Token::OpenParen,
            Token::CloseParen,
            Token::Eof,
        ]);

        assert!(result.is_err());
    }

    // ===== 13. Struct literal parsing =====

    #[test]
    fn struct_literal_empty() {
        // Quote {}
        let expr = parse_expr(vec![
            Token::Ident("Quote".to_string()),
            Token::OpenBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::StructLiteral { struct_name, fields } => {
                assert_eq!(struct_name, "Quote");
                assert!(fields.is_empty());
            }
            _ => panic!("Expected StructLiteral, got {:?}", expr.kind),
        }
    }

    #[test]
    fn struct_literal_single_field() {
        // Tick { price = 100 }
        let expr = parse_expr(vec![
            Token::Ident("Tick".to_string()),
            Token::OpenBrace,
            Token::Ident("price".to_string()),
            Token::Assign,
            Token::Int(100),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::StructLiteral { struct_name, fields } => {
                assert_eq!(struct_name, "Tick");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].0, "price");
                assert_eq!(fields[0].1.kind, ExprKind::IntLiteral(100));
            }
            _ => panic!("Expected StructLiteral, got {:?}", expr.kind),
        }
    }

    #[test]
    fn struct_literal_multi_field() {
        // Quote { bid = 100.0, ask = 101.0, size = 5 }
        let expr = parse_expr(vec![
            Token::Ident("Quote".to_string()),
            Token::OpenBrace,
            Token::Ident("bid".to_string()),
            Token::Assign,
            Token::Float(100.0),
            Token::Comma,
            Token::Ident("ask".to_string()),
            Token::Assign,
            Token::Float(101.0),
            Token::Comma,
            Token::Ident("size".to_string()),
            Token::Assign,
            Token::Int(5),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::StructLiteral { struct_name, fields } => {
                assert_eq!(struct_name, "Quote");
                assert_eq!(fields.len(), 3);
                assert_eq!(fields[0], ("bid".to_string(), Expr {
                    kind: ExprKind::FloatLiteral(100.0),
                    span: fields[0].1.span,
                }));
                assert_eq!(fields[1].0, "ask");
                assert_eq!(fields[1].1.kind, ExprKind::FloatLiteral(101.0));
                assert_eq!(fields[2].0, "size");
                assert_eq!(fields[2].1.kind, ExprKind::IntLiteral(5));
            }
            _ => panic!("Expected StructLiteral, got {:?}", expr.kind),
        }
    }

    #[test]
    fn struct_literal_trailing_comma() {
        // Tick { price = 100, }
        let expr = parse_expr(vec![
            Token::Ident("Tick".to_string()),
            Token::OpenBrace,
            Token::Ident("price".to_string()),
            Token::Assign,
            Token::Int(100),
            Token::Comma,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::StructLiteral { struct_name, fields } => {
                assert_eq!(struct_name, "Tick");
                assert_eq!(fields.len(), 1);
            }
            _ => panic!("Expected StructLiteral, got {:?}", expr.kind),
        }
    }

    #[test]
    fn struct_literal_nested() {
        // Outer { inner = Inner { x = 1 } }
        let expr = parse_expr(vec![
            Token::Ident("Outer".to_string()),
            Token::OpenBrace,
            Token::Ident("inner".to_string()),
            Token::Assign,
            Token::Ident("Inner".to_string()),
            Token::OpenBrace,
            Token::Ident("x".to_string()),
            Token::Assign,
            Token::Int(1),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::StructLiteral { struct_name, fields } => {
                assert_eq!(struct_name, "Outer");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].0, "inner");
                match &fields[0].1.kind {
                    ExprKind::StructLiteral { struct_name, fields } => {
                        assert_eq!(struct_name, "Inner");
                        assert_eq!(fields.len(), 1);
                        assert_eq!(fields[0].0, "x");
                        assert_eq!(fields[0].1.kind, ExprKind::IntLiteral(1));
                    }
                    other => panic!("Expected nested StructLiteral, got {:?}", other),
                }
            }
            _ => panic!("Expected StructLiteral, got {:?}", expr.kind),
        }
    }

    #[test]
    fn struct_literal_field_access_disambiguated_from_bare_ident() {
        // A bare identifier not followed by `{` should still parse as Ident.
        let expr = parse_expr(vec![
            Token::Ident("close".to_string()),
            Token::Eof,
        ])
        .unwrap();

        assert_eq!(expr.kind, ExprKind::Ident("close".to_string()));
    }

    #[test]
    fn error_missing_close_brace_in_struct_literal() {
        // Tick { price = 100  — missing closing brace
        let result = parse_expr(vec![
            Token::Ident("Tick".to_string()),
            Token::OpenBrace,
            Token::Ident("price".to_string()),
            Token::Assign,
            Token::Int(100),
            Token::Eof,
        ]);

        assert!(result.is_err());
    }

    // ===== 14. Enum construction parsing =====

    #[test]
    fn enum_construction_unit_variant() {
        // OrderType.Market
        let expr = parse_expr(vec![
            Token::Ident("OrderType".to_string()),
            Token::Dot,
            Token::Ident("Market".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::EnumConstruction { enum_name, variant_name, args } => {
                assert_eq!(enum_name, "OrderType");
                assert_eq!(variant_name, "Market");
                assert!(args.is_empty());
            }
            _ => panic!("Expected EnumConstruction, got {:?}", expr.kind),
        }
    }

    #[test]
    fn enum_construction_with_args() {
        // OrderType.Limit(price: 100.0)
        let expr = parse_expr(vec![
            Token::Ident("OrderType".to_string()),
            Token::Dot,
            Token::Ident("Limit".to_string()),
            Token::OpenParen,
            Token::Float(100.0),
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::EnumConstruction { enum_name, variant_name, args } => {
                assert_eq!(enum_name, "OrderType");
                assert_eq!(variant_name, "Limit");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].kind, ExprKind::FloatLiteral(100.0));
            }
            _ => panic!("Expected EnumConstruction, got {:?}", expr.kind),
        }
    }

    #[test]
    fn enum_construction_multiple_args() {
        // Result.Success(value, code)
        let expr = parse_expr(vec![
            Token::Ident("Result".to_string()),
            Token::Dot,
            Token::Ident("Success".to_string()),
            Token::OpenParen,
            Token::Ident("value".to_string()),
            Token::Comma,
            Token::Ident("code".to_string()),
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::EnumConstruction { enum_name, variant_name, args } => {
                assert_eq!(enum_name, "Result");
                assert_eq!(variant_name, "Success");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected EnumConstruction, got {:?}", expr.kind),
        }
    }

    #[test]
    fn member_access_lowercase_not_enum() {
        // obj.field should be member access, not enum construction
        let expr = parse_expr(vec![
            Token::Ident("obj".to_string()),
            Token::Dot,
            Token::Ident("field".to_string()),
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::MemberAccess { object, field } => {
                assert_eq!(field, "field");
                match &object.kind {
                    ExprKind::Ident(name) => assert_eq!(name, "obj"),
                    _ => panic!("Expected Ident as object"),
                }
            }
            _ => panic!("Expected MemberAccess, got {:?}", expr.kind),
        }
    }

    #[test]
    fn method_call_lowercase_not_enum() {
        // obj.method() should be method call, not enum construction
        let expr = parse_expr(vec![
            Token::Ident("obj".to_string()),
            Token::Dot,
            Token::Ident("method".to_string()),
            Token::OpenParen,
            Token::CloseParen,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::MethodCall { receiver, method, args } => {
                assert_eq!(method, "method");
                assert!(args.is_empty());
                match &receiver.kind {
                    ExprKind::Ident(name) => assert_eq!(name, "obj"),
                    _ => panic!("Expected Ident as receiver"),
                }
            }
            _ => panic!("Expected MethodCall, got {:?}", expr.kind),
        }
    }

    // ===== 15. Match expression parsing =====

    #[test]
    fn match_expression_single_arm() {
        // match x { OrderType.Market => { a } }
        let expr = parse_expr(vec![
            Token::Match,
            Token::Ident("x".to_string()),
            Token::OpenBrace,
            Token::Ident("OrderType".to_string()),
            Token::Dot,
            Token::Ident("Market".to_string()),
            Token::FatArrow,
            Token::OpenBrace,
            Token::Ident("a".to_string()),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::Match(MatchExpr { scrutinee, arms, span: _ }) => {
                assert_eq!(scrutinee.kind, ExprKind::Ident("x".to_string()));
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern {
                    Pattern::Variant { enum_name, variant_name, bindings, span: _ } => {
                        assert_eq!(enum_name, "OrderType");
                        assert_eq!(variant_name, "Market");
                        assert!(bindings.is_empty());
                    }
                    _ => panic!("Expected Variant pattern"),
                }
            }
            _ => panic!("Expected Match, got {:?}", expr.kind),
        }
    }

    #[test]
    fn match_expression_with_bindings() {
        // match order { OrderType.Limit(p) => { p } }
        let expr = parse_expr(vec![
            Token::Match,
            Token::Ident("order".to_string()),
            Token::OpenBrace,
            Token::Ident("OrderType".to_string()),
            Token::Dot,
            Token::Ident("Limit".to_string()),
            Token::OpenParen,
            Token::Ident("p".to_string()),
            Token::CloseParen,
            Token::FatArrow,
            Token::OpenBrace,
            Token::Ident("p".to_string()),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::Match(MatchExpr { scrutinee, arms, span: _ }) => {
                assert_eq!(scrutinee.kind, ExprKind::Ident("order".to_string()));
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern {
                    Pattern::Variant { enum_name, variant_name, bindings, span: _ } => {
                        assert_eq!(enum_name, "OrderType");
                        assert_eq!(variant_name, "Limit");
                        assert_eq!(bindings, &["p".to_string()]);
                    }
                    _ => panic!("Expected Variant pattern"),
                }
            }
            _ => panic!("Expected Match, got {:?}", expr.kind),
        }
    }

    #[test]
    fn match_expression_wildcard_pattern() {
        // match x { _ => { default } }
        let expr = parse_expr(vec![
            Token::Match,
            Token::Ident("x".to_string()),
            Token::OpenBrace,
            Token::Ident("_".to_string()),
            Token::FatArrow,
            Token::OpenBrace,
            Token::Ident("default".to_string()),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::Match(MatchExpr { scrutinee, arms, span: _ }) => {
                assert_eq!(scrutinee.kind, ExprKind::Ident("x".to_string()));
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern {
                    Pattern::Wildcard { span: _ } => {}
                    _ => panic!("Expected Wildcard pattern"),
                }
            }
            _ => panic!("Expected Match, got {:?}", expr.kind),
        }
    }

    #[test]
    fn match_expression_multiple_arms() {
        // match x { OrderType.Market => { a } OrderType.Limit(p) => { p } }
        let expr = parse_expr(vec![
            Token::Match,
            Token::Ident("x".to_string()),
            Token::OpenBrace,
            // First arm
            Token::Ident("OrderType".to_string()),
            Token::Dot,
            Token::Ident("Market".to_string()),
            Token::FatArrow,
            Token::OpenBrace,
            Token::Ident("a".to_string()),
            Token::CloseBrace,
            // Second arm
            Token::Ident("OrderType".to_string()),
            Token::Dot,
            Token::Ident("Limit".to_string()),
            Token::OpenParen,
            Token::Ident("p".to_string()),
            Token::CloseParen,
            Token::FatArrow,
            Token::OpenBrace,
            Token::Ident("p".to_string()),
            Token::CloseBrace,
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::Match(MatchExpr { arms, .. }) => {
                assert_eq!(arms.len(), 2);
                // First arm
                match &arms[0].pattern {
                    Pattern::Variant { variant_name, .. } => {
                        assert_eq!(variant_name, "Market");
                    }
                    _ => panic!("Expected Variant pattern"),
                }
                // Second arm
                match &arms[1].pattern {
                    Pattern::Variant { variant_name, bindings, .. } => {
                        assert_eq!(variant_name, "Limit");
                        assert_eq!(bindings, &["p".to_string()]);
                    }
                    _ => panic!("Expected Variant pattern"),
                }
            }
            _ => panic!("Expected Match, got {:?}", expr.kind),
        }
    }

    #[test]
    fn match_expression_single_expr_body() {
        // match x { OrderType.Market => 1 }
        let expr = parse_expr(vec![
            Token::Match,
            Token::Ident("x".to_string()),
            Token::OpenBrace,
            Token::Ident("OrderType".to_string()),
            Token::Dot,
            Token::Ident("Market".to_string()),
            Token::FatArrow,
            Token::Int(1),
            Token::CloseBrace,
            Token::Eof,
        ])
        .unwrap();

        match &expr.kind {
            ExprKind::Match(MatchExpr { arms, .. }) => {
                assert_eq!(arms.len(), 1);
                assert_eq!(arms[0].body.len(), 1);
                match &arms[0].body[0] {
                    Stmt::Expr(ExprStmt { expr, span: _ }) => {
                        assert_eq!(expr.kind, ExprKind::IntLiteral(1));
                    }
                    _ => panic!("Expected Expr statement"),
                }
            }
            _ => panic!("Expected Match, got {:?}", expr.kind),
        }
    }
}
