//! Pretty-printer for the Flux AST.
//!
//! Converts a `Program` AST back into well-formatted Flux source text.
//! Uses minimal parenthesization based on operator precedence.

use super::ast::*;

/// Format a complete program back to Flux source text.
pub fn format_program(program: &Program) -> String {
    let mut output = String::new();

    // Format imports
    for import in &program.imports {
        format_import(&mut output, import);
        output.push('\n');
    }
    if !program.imports.is_empty() {
        output.push('\n');
    }

    // Format strategy
    format_strategy(&mut output, &program.strategy);
    output
}

fn format_import(output: &mut String, import: &Import) {
    output.push_str("from ");
    output.push_str(&import.module_path);
    output.push_str(" import {");
    for (i, name) in import.names.iter().enumerate() {
        if i > 0 {
            output.push_str(", ");
        }
        output.push_str(name);
    }
    output.push('}');
}

fn format_strategy(output: &mut String, strategy: &Strategy) {
    output.push_str("strategy ");
    output.push_str(&strategy.name);
    output.push_str(" {\n");

    for item in &strategy.body {
        format_strategy_item(output, item, 1);
    }

    output.push_str("}\n");
}

fn format_strategy_item(output: &mut String, item: &StrategyItem, indent: usize) {
    match item {
        StrategyItem::Property(prop) => format_property(output, prop, indent),
        StrategyItem::ParamsBlock(block) => format_params_block(output, block, indent),
        StrategyItem::StateBlock(block) => format_state_block(output, block, indent),
        StrategyItem::EventHandler(handler) => format_event_handler(output, handler, indent),
    }
}

fn format_property(output: &mut String, prop: &Property, indent: usize) {
    write_indent(output, indent);
    output.push_str(&prop.name);
    output.push_str(" = ");
    format_expr(output, &prop.value);
    output.push('\n');
}

fn format_params_block(output: &mut String, block: &ParamsBlock, indent: usize) {
    write_indent(output, indent);
    output.push_str("params {\n");

    for param in &block.params {
        write_indent(output, indent + 1);
        output.push_str(&param.name);
        output.push_str(" = ");
        format_expr(output, &param.default_value);
        output.push('\n');
    }

    write_indent(output, indent);
    output.push_str("}\n");
}

fn format_state_block(output: &mut String, block: &StateBlock, indent: usize) {
    write_indent(output, indent);
    output.push_str("state {\n");

    for var in &block.variables {
        write_indent(output, indent + 1);
        output.push_str(&var.name);
        output.push_str(" = ");
        format_expr(output, &var.initial_value);
        output.push('\n');
    }

    write_indent(output, indent);
    output.push_str("}\n");
}

fn format_event_handler(output: &mut String, handler: &EventHandler, indent: usize) {
    write_indent(output, indent);
    output.push_str("on_");
    output.push_str(&handler.event_name);
    output.push_str(" {\n");

    for stmt in &handler.body {
        format_stmt(output, stmt, indent + 1);
    }

    write_indent(output, indent);
    output.push_str("}\n");
}

// --- Statement formatting ---

fn format_stmt(output: &mut String, stmt: &Stmt, indent: usize) {
    match stmt {
        Stmt::Assignment(assign) => {
            write_indent(output, indent);
            format_expr(output, &assign.target);
            output.push_str(" = ");
            format_expr(output, &assign.value);
            output.push('\n');
        }
        Stmt::If(if_stmt) => format_if_stmt(output, if_stmt, indent),
        Stmt::For(for_loop) => format_for_loop(output, for_loop, indent),
        Stmt::While(while_loop) => format_while_loop(output, while_loop, indent),
        Stmt::Return(ret) => {
            write_indent(output, indent);
            output.push_str("return");
            if let Some(value) = &ret.value {
                output.push(' ');
                format_expr(output, value);
            }
            output.push('\n');
        }
        Stmt::Expr(expr_stmt) => {
            write_indent(output, indent);
            format_expr(output, &expr_stmt.expr);
            output.push('\n');
        }
    }
}

fn format_if_stmt(output: &mut String, if_stmt: &IfStmt, indent: usize) {
    write_indent(output, indent);
    output.push_str("if ");
    format_expr(output, &if_stmt.condition);
    output.push_str(" {\n");

    for stmt in &if_stmt.body {
        format_stmt(output, stmt, indent + 1);
    }

    for elif in &if_stmt.elif_branches {
        write_indent(output, indent);
        output.push_str("} elif ");
        format_expr(output, &elif.condition);
        output.push_str(" {\n");

        for stmt in &elif.body {
            format_stmt(output, stmt, indent + 1);
        }
    }

    if let Some(else_body) = &if_stmt.else_body {
        write_indent(output, indent);
        output.push_str("} else {\n");

        for stmt in else_body {
            format_stmt(output, stmt, indent + 1);
        }
    }

    write_indent(output, indent);
    output.push_str("}\n");
}

fn format_for_loop(output: &mut String, for_loop: &ForLoop, indent: usize) {
    write_indent(output, indent);
    output.push_str("for ");
    output.push_str(&for_loop.variable);
    output.push_str(" in ");
    format_expr(output, &for_loop.iterable);
    output.push_str(" {\n");

    for stmt in &for_loop.body {
        format_stmt(output, stmt, indent + 1);
    }

    write_indent(output, indent);
    output.push_str("}\n");
}

fn format_while_loop(output: &mut String, while_loop: &WhileLoop, indent: usize) {
    write_indent(output, indent);
    output.push_str("while ");
    format_expr(output, &while_loop.condition);
    output.push_str(" {\n");

    for stmt in &while_loop.body {
        format_stmt(output, stmt, indent + 1);
    }

    write_indent(output, indent);
    output.push_str("}\n");
}

// --- Expression formatting ---

fn format_expr(output: &mut String, expr: &Expr) {
    format_expr_with_prec(output, expr, 0, false);
}

/// Format an expression, parenthesizing if its precedence is lower than `parent_prec`.
/// `is_right` indicates if this is the right operand (needs parens at same precedence
/// for left-associative operators).
fn format_expr_with_prec(output: &mut String, expr: &Expr, parent_prec: u8, is_right: bool) {
    let my_prec = expr_precedence(&expr.kind);
    let needs_parens = if my_prec == 0 {
        false // atoms never need parens
    } else if is_right {
        my_prec <= parent_prec // right operand: parens at same or lower
    } else {
        my_prec < parent_prec // left operand: parens only at lower
    };

    if needs_parens {
        output.push('(');
    }
    format_expr_inner(output, expr);
    if needs_parens {
        output.push(')');
    }
}

fn format_expr_inner(output: &mut String, expr: &Expr) {
    match &expr.kind {
        ExprKind::IntLiteral(n) => {
            output.push_str(&n.to_string());
        }
        ExprKind::FloatLiteral(f) => {
            let s = f.to_string();
            output.push_str(&s);
            // Ensure there's a decimal point so it parses back as float
            if !s.contains('.') {
                output.push_str(".0");
            }
        }
        ExprKind::StringLiteral(s) => {
            output.push('"');
            for ch in s.chars() {
                match ch {
                    '\n' => output.push_str("\\n"),
                    '\t' => output.push_str("\\t"),
                    '"' => output.push_str("\\\""),
                    '\\' => output.push_str("\\\\"),
                    c => output.push(c),
                }
            }
            output.push('"');
        }
        ExprKind::BoolLiteral(b) => {
            output.push_str(if *b { "true" } else { "false" });
        }
        ExprKind::NullLiteral => {
            output.push_str("null");
        }
        ExprKind::ListLiteral(elements) => {
            output.push('[');
            for (i, elem) in elements.iter().enumerate() {
                if i > 0 {
                    output.push_str(", ");
                }
                format_expr(output, elem);
            }
            output.push(']');
        }
        ExprKind::Ident(name) => {
            output.push_str(name);
        }
        ExprKind::BinaryOp { left, op, right } => {
            let prec = binop_precedence(op);
            format_expr_with_prec(output, left, prec, false);
            output.push(' ');
            output.push_str(binop_str(op));
            output.push(' ');
            format_expr_with_prec(output, right, prec, true);
        }
        ExprKind::UnaryOp { op, operand } => match op {
            UnaryOp::Neg => {
                output.push('-');
                format_expr_with_prec(output, operand, 7, false);
            }
            UnaryOp::Not => {
                output.push_str("not ");
                format_expr_with_prec(output, operand, 7, false);
            }
        },
        ExprKind::FunctionCall { function, args } => {
            format_expr_with_prec(output, function, 0, false);
            output.push('(');
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    output.push_str(", ");
                }
                format_expr(output, arg);
            }
            output.push(')');
        }
        ExprKind::MethodCall { receiver, method, args } => {
            format_expr_with_prec(output, receiver, 0, false);
            output.push('.');
            output.push_str(method);
            output.push('(');
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    output.push_str(", ");
                }
                format_expr(output, arg);
            }
            output.push(')');
        }
        ExprKind::MemberAccess { object, field } => {
            format_expr_with_prec(output, object, 0, false);
            output.push('.');
            output.push_str(field);
        }
        ExprKind::IndexAccess { object, index } => {
            format_expr_with_prec(output, object, 0, false);
            output.push('[');
            format_expr(output, index);
            output.push(']');
        }
    }
}

// --- Precedence helpers ---

fn expr_precedence(kind: &ExprKind) -> u8 {
    match kind {
        ExprKind::BinaryOp { op, .. } => binop_precedence(op),
        ExprKind::UnaryOp { .. } => 7,
        _ => 0, // atoms, calls, etc. — never need outer parens
    }
}

fn binop_precedence(op: &BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne => 3,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

fn binop_str(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "and",
        BinOp::Or => "or",
    }
}

// --- Indentation helper ---

fn write_indent(output: &mut String, level: usize) {
    for _ in 0..level {
        output.push_str("    ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;

    fn dummy_span() -> Span {
        Span::new(0, 0)
    }

    fn make_expr(kind: ExprKind) -> Expr {
        Expr { kind, span: dummy_span() }
    }

    fn format_single_expr(expr: &Expr) -> String {
        let mut output = String::new();
        format_expr(&mut output, expr);
        output
    }

    // 1. Integer literal formatting
    #[test]
    fn format_int_literal() {
        let expr = make_expr(ExprKind::IntLiteral(42));
        assert_eq!(format_single_expr(&expr), "42");
    }

    // 2. Float literal formatting
    #[test]
    fn format_float_literal() {
        let expr = make_expr(ExprKind::FloatLiteral(3.14));
        assert_eq!(format_single_expr(&expr), "3.14");
    }

    // 3. String literal formatting with escapes
    #[test]
    fn format_string_literal_with_escapes() {
        let expr = make_expr(ExprKind::StringLiteral("hello\nworld".to_string()));
        assert_eq!(format_single_expr(&expr), r#""hello\nworld""#);
    }

    // 4. Bool and null formatting
    #[test]
    fn format_bool_true() {
        let expr = make_expr(ExprKind::BoolLiteral(true));
        assert_eq!(format_single_expr(&expr), "true");
    }

    #[test]
    fn format_bool_false() {
        let expr = make_expr(ExprKind::BoolLiteral(false));
        assert_eq!(format_single_expr(&expr), "false");
    }

    #[test]
    fn format_null_literal() {
        let expr = make_expr(ExprKind::NullLiteral);
        assert_eq!(format_single_expr(&expr), "null");
    }

    // 5. Binary op formatting
    #[test]
    fn format_binary_op_add() {
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::Ident("a".to_string()))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::Ident("b".to_string()))),
        });
        assert_eq!(format_single_expr(&expr), "a + b");
    }

    // 6. Precedence parenthesization: a + b * c doesn't need parens;
    //    (a + b) * c does need parens around a + b
    #[test]
    fn format_precedence_no_extra_parens() {
        // a + b * c → "a + b * c" (mul is higher prec, no parens needed)
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::Ident("a".to_string()))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::BinaryOp {
                left: Box::new(make_expr(ExprKind::Ident("b".to_string()))),
                op: BinOp::Mul,
                right: Box::new(make_expr(ExprKind::Ident("c".to_string()))),
            })),
        });
        assert_eq!(format_single_expr(&expr), "a + b * c");
    }

    #[test]
    fn format_precedence_needs_parens() {
        // (a + b) * c → "(a + b) * c"
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::BinaryOp {
                left: Box::new(make_expr(ExprKind::Ident("a".to_string()))),
                op: BinOp::Add,
                right: Box::new(make_expr(ExprKind::Ident("b".to_string()))),
            })),
            op: BinOp::Mul,
            right: Box::new(make_expr(ExprKind::Ident("c".to_string()))),
        });
        assert_eq!(format_single_expr(&expr), "(a + b) * c");
    }

    // 7. Left-associativity preservation: a + (b + c) needs parens on right
    #[test]
    fn format_left_assoc_right_needs_parens() {
        // a + (b + c) → "a + (b + c)" (right operand at same prec needs parens)
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::Ident("a".to_string()))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::BinaryOp {
                left: Box::new(make_expr(ExprKind::Ident("b".to_string()))),
                op: BinOp::Add,
                right: Box::new(make_expr(ExprKind::Ident("c".to_string()))),
            })),
        });
        assert_eq!(format_single_expr(&expr), "a + (b + c)");
    }

    // 8. Left-associativity no extra parens: (a + b) + c doesn't need parens
    #[test]
    fn format_left_assoc_left_no_extra_parens() {
        // (a + b) + c → "a + b + c" (left operand at same prec doesn't need parens)
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::BinaryOp {
                left: Box::new(make_expr(ExprKind::Ident("a".to_string()))),
                op: BinOp::Add,
                right: Box::new(make_expr(ExprKind::Ident("b".to_string()))),
            })),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::Ident("c".to_string()))),
        });
        assert_eq!(format_single_expr(&expr), "a + b + c");
    }

    // 9. Unary negation formatting
    #[test]
    fn format_unary_neg() {
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Neg,
            operand: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
        });
        assert_eq!(format_single_expr(&expr), "-x");
    }

    // 10. Unary not formatting
    #[test]
    fn format_unary_not() {
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
        });
        assert_eq!(format_single_expr(&expr), "not x");
    }

    // 11. Function call formatting
    #[test]
    fn format_function_call() {
        let expr = make_expr(ExprKind::FunctionCall {
            function: Box::new(make_expr(ExprKind::Ident("f".to_string()))),
            args: vec![
                make_expr(ExprKind::Ident("a".to_string())),
                make_expr(ExprKind::Ident("b".to_string())),
            ],
        });
        assert_eq!(format_single_expr(&expr), "f(a, b)");
    }

    // 12. Method call formatting
    #[test]
    fn format_method_call() {
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("obj".to_string()))),
            method: "method".to_string(),
            args: vec![make_expr(ExprKind::Ident("a".to_string()))],
        });
        assert_eq!(format_single_expr(&expr), "obj.method(a)");
    }

    // 13. Member access formatting
    #[test]
    fn format_member_access() {
        let expr = make_expr(ExprKind::MemberAccess {
            object: Box::new(make_expr(ExprKind::Ident("obj".to_string()))),
            field: "field".to_string(),
        });
        assert_eq!(format_single_expr(&expr), "obj.field");
    }

    // 14. Index access formatting
    #[test]
    fn format_index_access() {
        let expr = make_expr(ExprKind::IndexAccess {
            object: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            index: Box::new(make_expr(ExprKind::IntLiteral(0))),
        });
        assert_eq!(format_single_expr(&expr), "arr[0]");
    }

    // 15. List literal formatting
    #[test]
    fn format_list_literal() {
        let expr = make_expr(ExprKind::ListLiteral(vec![
            make_expr(ExprKind::IntLiteral(1)),
            make_expr(ExprKind::IntLiteral(2)),
            make_expr(ExprKind::IntLiteral(3)),
        ]));
        assert_eq!(format_single_expr(&expr), "[1, 2, 3]");
    }

    // 16. Basic round-trip: format → lex → parse → compare ASTs
    #[test]
    fn format_round_trip_minimal_program() {
        use crate::lexer::lex_with_spans;
        use super::super::{parse, pretty_print_program};

        // Build a minimal program AST
        let program = Program {
            imports: vec![],
            data_block: None,
            strategy: Strategy {
                name: "Test".to_string(),
                body: vec![
                    StrategyItem::Property(Property {
                        name: "version".to_string(),
                        value: make_expr(ExprKind::IntLiteral(1)),
                        span: dummy_span(),
                    }),
                ],
                span: dummy_span(),
            },
            span: dummy_span(),
        };

        // Format to source
        let source = pretty_print_program(&program);

        // Parse back
        let tokens = lex_with_spans(&source).expect("round-trip lex failed");
        let parsed = parse(tokens).expect("round-trip parse failed");

        // Compare (ignoring spans by checking structural equality of key fields)
        assert_eq!(parsed.strategy.name, program.strategy.name);
        assert_eq!(parsed.strategy.body.len(), program.strategy.body.len());
        match (&parsed.strategy.body[0], &program.strategy.body[0]) {
            (StrategyItem::Property(p1), StrategyItem::Property(p2)) => {
                assert_eq!(p1.name, p2.name);
                assert_eq!(p1.value.kind, p2.value.kind);
            }
            _ => panic!("Expected Property strategy item"),
        }
    }

    // 17. Indentation of nested blocks: strategy with event handler containing if statement
    #[test]
    fn format_indentation_nested_blocks() {
        let program = Program {
            imports: vec![],
            data_block: None,
            strategy: Strategy {
                name: "MyStrategy".to_string(),
                body: vec![
                    StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![
                            Stmt::If(IfStmt {
                                condition: make_expr(ExprKind::BinaryOp {
                                    left: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
                                    op: BinOp::Gt,
                                    right: Box::new(make_expr(ExprKind::IntLiteral(0))),
                                }),
                                body: vec![
                                    Stmt::Expr(ExprStmt {
                                        expr: make_expr(ExprKind::FunctionCall {
                                            function: Box::new(make_expr(ExprKind::Ident("buy".to_string()))),
                                            args: vec![],
                                        }),
                                        span: dummy_span(),
                                    }),
                                ],
                                elif_branches: vec![],
                                else_body: None,
                                span: dummy_span(),
                            }),
                        ],
                        span: dummy_span(),
                    }),
                ],
                span: dummy_span(),
            },
            span: dummy_span(),
        };

        let output = format_program(&program);
        let expected = "\
strategy MyStrategy {
    on_bar {
        if x > 0 {
            buy()
        }
    }
}
";
        assert_eq!(output, expected);
    }
}
