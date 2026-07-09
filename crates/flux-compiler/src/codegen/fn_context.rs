//! Context analysis for user-defined functions in code generation.
//!
//! Determines which functions need `ctx: &BarContext` and/or
//! `signals: &mut Vec<Signal>` parameters based on their body content
//! and transitive call dependencies.

use std::collections::{HashMap, HashSet};

use crate::typeck::typed_ast::*;

/// Known market data identifiers available through `ctx`.
const MARKET_DATA: &[&str] = &[
    "close", "open", "high", "low", "volume", "symbol", "in_position",
];

/// Known signal-producing function names.
const SIGNAL_FUNCTIONS: &[&str] = &["OPEN", "CLOSE", "CLOSE_QTY"];

/// Context requirements for a user-defined function in codegen.
#[derive(Debug, Clone, PartialEq)]
pub struct FnContext {
    /// Whether the function needs `ctx: &BarContext` parameter.
    pub needs_bar_context: bool,
    /// Whether the function needs `signals: &mut Vec<Signal>` parameter.
    pub needs_signals: bool,
}

/// Analyze all user-defined functions to determine which need bar context
/// and/or signals parameters.
///
/// This performs two passes:
/// 1. Direct analysis: does the function body reference market data or emit signals?
/// 2. Transitive closure: if function A calls function B that needs ctx, then A also needs ctx.
pub fn analyze_function_context(functions: &[TypedFnDef]) -> HashMap<String, FnContext> {
    let fn_names: HashSet<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    let mut contexts: HashMap<String, FnContext> = HashMap::new();

    // Pass 1: direct analysis
    for fn_def in functions {
        let param_names: HashSet<&str> = fn_def.params.iter().map(|p| p.as_str()).collect();
        let needs_ctx = body_references_market_data(&fn_def.body, &param_names);
        let needs_signals = body_emits_signals(&fn_def.body);
        contexts.insert(
            fn_def.name.clone(),
            FnContext {
                needs_bar_context: needs_ctx,
                needs_signals,
            },
        );
    }

    // Pass 2: transitive closure — propagate through call graph
    let mut changed = true;
    while changed {
        changed = false;
        for fn_def in functions {
            let callees = extract_user_fn_calls(&fn_def.body, &fn_names);
            for callee in &callees {
                let callee_ctx = contexts.get(callee.as_str()).cloned();
                if let Some(callee_ctx) = callee_ctx {
                    let my_ctx = contexts.get_mut(&fn_def.name).unwrap();
                    if callee_ctx.needs_bar_context && !my_ctx.needs_bar_context {
                        my_ctx.needs_bar_context = true;
                        changed = true;
                    }
                    if callee_ctx.needs_signals && !my_ctx.needs_signals {
                        my_ctx.needs_signals = true;
                        changed = true;
                    }
                }
            }
        }
    }

    contexts
}

/// Check if a function body directly references market data variables.
/// Excludes identifiers that match function parameter names (those are local).
fn body_references_market_data(stmts: &[TypedStmt], param_names: &HashSet<&str>) -> bool {
    for stmt in stmts {
        if stmt_references_market_data(stmt, param_names) {
            return true;
        }
    }
    false
}

/// Check if a single statement references market data variables.
fn stmt_references_market_data(stmt: &TypedStmt, param_names: &HashSet<&str>) -> bool {
    match stmt {
        TypedStmt::Assignment(assign) => {
            expr_references_market_data(&assign.value, param_names)
                || expr_references_market_data(&assign.target, param_names)
        }
        TypedStmt::If(if_stmt) => {
            expr_references_market_data(&if_stmt.condition, param_names)
                || body_references_market_data(&if_stmt.body, param_names)
                || if_stmt
                    .elif_branches
                    .iter()
                    .any(|b| {
                        expr_references_market_data(&b.condition, param_names)
                            || body_references_market_data(&b.body, param_names)
                    })
                || if_stmt
                    .else_body
                    .as_ref()
                    .is_some_and(|b| body_references_market_data(b, param_names))
        }
        TypedStmt::For(for_loop) => {
            expr_references_market_data(&for_loop.iterable, param_names)
                || body_references_market_data(&for_loop.body, param_names)
        }
        TypedStmt::While(while_loop) => {
            expr_references_market_data(&while_loop.condition, param_names)
                || body_references_market_data(&while_loop.body, param_names)
        }
        TypedStmt::Return(ret) => ret
            .value
            .as_ref()
            .is_some_and(|e| expr_references_market_data(e, param_names)),
        TypedStmt::Expr(expr_stmt) => {
            expr_references_market_data(&expr_stmt.expr, param_names)
        }
    }
}

/// Check if an expression references market data variables.
fn expr_references_market_data(expr: &TypedExpr, param_names: &HashSet<&str>) -> bool {
    match &expr.kind {
        TypedExprKind::Ident(name) => {
            // Only count as market data if it's NOT a function parameter
            !param_names.contains(name.as_str()) && MARKET_DATA.contains(&name.as_str())
        }
        TypedExprKind::BinaryOp { left, right, .. } => {
            expr_references_market_data(left, param_names)
                || expr_references_market_data(right, param_names)
        }
        TypedExprKind::UnaryOp { operand, .. } => {
            expr_references_market_data(operand, param_names)
        }
        TypedExprKind::FunctionCall { function, args } => {
            expr_references_market_data(function, param_names)
                || args
                    .iter()
                    .any(|a| expr_references_market_data(a, param_names))
        }
        TypedExprKind::MethodCall {
            receiver, args, ..
        } => {
            expr_references_market_data(receiver, param_names)
                || args
                    .iter()
                    .any(|a| expr_references_market_data(a, param_names))
        }
        TypedExprKind::MemberAccess { object, .. } => {
            expr_references_market_data(object, param_names)
        }
        TypedExprKind::IndexAccess { object, index } => {
            expr_references_market_data(object, param_names)
                || expr_references_market_data(index, param_names)
        }
        TypedExprKind::ListLiteral(items) => {
            items.iter().any(|i| expr_references_market_data(i, param_names))
        }
        // Literals don't reference market data
        TypedExprKind::IntLiteral(_)
        | TypedExprKind::FloatLiteral(_)
        | TypedExprKind::StringLiteral(_)
        | TypedExprKind::BoolLiteral(_)
        | TypedExprKind::NullLiteral => false,
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, expr)| expr_references_market_data(expr, param_names))
        }
    }
}

/// Check if a function body emits signals (calls OPEN, CLOSE, or CLOSE_QTY).
fn body_emits_signals(stmts: &[TypedStmt]) -> bool {
    for stmt in stmts {
        if stmt_emits_signals(stmt) {
            return true;
        }
    }
    false
}

/// Check if a single statement emits signals.
fn stmt_emits_signals(stmt: &TypedStmt) -> bool {
    match stmt {
        TypedStmt::Assignment(assign) => {
            expr_emits_signals(&assign.value) || expr_emits_signals(&assign.target)
        }
        TypedStmt::If(if_stmt) => {
            expr_emits_signals(&if_stmt.condition)
                || stmts_emit_signals(&if_stmt.body)
                || if_stmt
                    .elif_branches
                    .iter()
                    .any(|b| {
                        expr_emits_signals(&b.condition)
                            || stmts_emit_signals(&b.body)
                    })
                || if_stmt
                    .else_body
                    .as_ref()
                    .is_some_and(|b| stmts_emit_signals(b))
        }
        TypedStmt::For(for_loop) => {
            expr_emits_signals(&for_loop.iterable)
                || stmts_emit_signals(&for_loop.body)
        }
        TypedStmt::While(while_loop) => {
            expr_emits_signals(&while_loop.condition)
                || stmts_emit_signals(&while_loop.body)
        }
        TypedStmt::Return(ret) => ret
            .value
            .as_ref()
            .is_some_and(|e| expr_emits_signals(e)),
        TypedStmt::Expr(expr_stmt) => expr_emits_signals(&expr_stmt.expr),
    }
}

/// Helper: check if any statement in a list emits signals.
fn stmts_emit_signals(stmts: &[TypedStmt]) -> bool {
    stmts.iter().any(|s| stmt_emits_signals(s))
}

/// Check if an expression emits signals (calls OPEN, CLOSE, or CLOSE_QTY).
fn expr_emits_signals(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::FunctionCall { function, args } => {
            // Check if the function itself is a signal function
            if let TypedExprKind::Ident(name) = &function.kind {
                if SIGNAL_FUNCTIONS.contains(&name.as_str()) {
                    return true;
                }
            }
            // Also check args recursively
            expr_emits_signals(function)
                || args.iter().any(|a| expr_emits_signals(a))
        }
        TypedExprKind::BinaryOp { left, right, .. } => {
            expr_emits_signals(left) || expr_emits_signals(right)
        }
        TypedExprKind::UnaryOp { operand, .. } => expr_emits_signals(operand),
        TypedExprKind::MethodCall {
            receiver, args, ..
        } => {
            expr_emits_signals(receiver)
                || args.iter().any(|a| expr_emits_signals(a))
        }
        TypedExprKind::MemberAccess { object, .. } => expr_emits_signals(object),
        TypedExprKind::IndexAccess { object, index } => {
            expr_emits_signals(object) || expr_emits_signals(index)
        }
        TypedExprKind::ListLiteral(items) => {
            items.iter().any(|i| expr_emits_signals(i))
        }
        // Idents and literals don't emit signals
        TypedExprKind::Ident(_)
        | TypedExprKind::IntLiteral(_)
        | TypedExprKind::FloatLiteral(_)
        | TypedExprKind::StringLiteral(_)
        | TypedExprKind::BoolLiteral(_)
        | TypedExprKind::NullLiteral => false,
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, expr)| expr_emits_signals(expr))
        }
    }
}

/// Extract calls to user-defined functions from a function body.
/// Only returns names that are in the `fn_names` set.
fn extract_user_fn_calls(stmts: &[TypedStmt], fn_names: &HashSet<&str>) -> Vec<String> {
    let mut calls = Vec::new();
    for stmt in stmts {
        extract_calls_from_stmt(stmt, fn_names, &mut calls);
    }
    calls
}

/// Extract user-defined function calls from a statement.
fn extract_calls_from_stmt(
    stmt: &TypedStmt,
    fn_names: &HashSet<&str>,
    calls: &mut Vec<String>,
) {
    match stmt {
        TypedStmt::Assignment(assign) => {
            extract_calls_from_expr(&assign.target, fn_names, calls);
            extract_calls_from_expr(&assign.value, fn_names, calls);
        }
        TypedStmt::If(if_stmt) => {
            extract_calls_from_expr(&if_stmt.condition, fn_names, calls);
            for s in &if_stmt.body {
                extract_calls_from_stmt(s, fn_names, calls);
            }
            for branch in &if_stmt.elif_branches {
                extract_calls_from_expr(&branch.condition, fn_names, calls);
                for s in &branch.body {
                    extract_calls_from_stmt(s, fn_names, calls);
                }
            }
            if let Some(else_body) = &if_stmt.else_body {
                for s in else_body {
                    extract_calls_from_stmt(s, fn_names, calls);
                }
            }
        }
        TypedStmt::For(for_loop) => {
            extract_calls_from_expr(&for_loop.iterable, fn_names, calls);
            for s in &for_loop.body {
                extract_calls_from_stmt(s, fn_names, calls);
            }
        }
        TypedStmt::While(while_loop) => {
            extract_calls_from_expr(&while_loop.condition, fn_names, calls);
            for s in &while_loop.body {
                extract_calls_from_stmt(s, fn_names, calls);
            }
        }
        TypedStmt::Return(ret) => {
            if let Some(expr) = &ret.value {
                extract_calls_from_expr(expr, fn_names, calls);
            }
        }
        TypedStmt::Expr(expr_stmt) => {
            extract_calls_from_expr(&expr_stmt.expr, fn_names, calls);
        }
    }
}

/// Extract user-defined function calls from an expression.
fn extract_calls_from_expr(
    expr: &TypedExpr,
    fn_names: &HashSet<&str>,
    calls: &mut Vec<String>,
) {
    match &expr.kind {
        TypedExprKind::FunctionCall { function, args } => {
            if let TypedExprKind::Ident(name) = &function.kind {
                if fn_names.contains(name.as_str()) {
                    calls.push(name.clone());
                }
            }
            extract_calls_from_expr(function, fn_names, calls);
            for arg in args {
                extract_calls_from_expr(arg, fn_names, calls);
            }
        }
        TypedExprKind::BinaryOp { left, right, .. } => {
            extract_calls_from_expr(left, fn_names, calls);
            extract_calls_from_expr(right, fn_names, calls);
        }
        TypedExprKind::UnaryOp { operand, .. } => {
            extract_calls_from_expr(operand, fn_names, calls);
        }
        TypedExprKind::MethodCall {
            receiver, args, ..
        } => {
            extract_calls_from_expr(receiver, fn_names, calls);
            for arg in args {
                extract_calls_from_expr(arg, fn_names, calls);
            }
        }
        TypedExprKind::MemberAccess { object, .. } => {
            extract_calls_from_expr(object, fn_names, calls);
        }
        TypedExprKind::IndexAccess { object, index } => {
            extract_calls_from_expr(object, fn_names, calls);
            extract_calls_from_expr(index, fn_names, calls);
        }
        TypedExprKind::ListLiteral(items) => {
            for item in items {
                extract_calls_from_expr(item, fn_names, calls);
            }
        }
        // Idents and literals have no nested calls
        TypedExprKind::Ident(_)
        | TypedExprKind::IntLiteral(_)
        | TypedExprKind::FloatLiteral(_)
        | TypedExprKind::StringLiteral(_)
        | TypedExprKind::BoolLiteral(_)
        | TypedExprKind::NullLiteral => {}
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, field_expr) in fields {
                extract_calls_from_expr(field_expr, fn_names, calls);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;
    use crate::typeck::types::FluxType;

    /// Helper to create a typed expression at a dummy span.
    fn texpr(kind: TypedExprKind) -> TypedExpr {
        TypedExpr {
            kind,
            resolved_type: FluxType::Float,
            span: Span::new(0, 1),
        }
    }

    /// Helper to create a function call expression.
    fn call_expr(name: &str, args: Vec<TypedExpr>) -> TypedExpr {
        texpr(TypedExprKind::FunctionCall {
            function: Box::new(texpr(TypedExprKind::Ident(name.to_string()))),
            args,
        })
    }

    /// Helper to create a simple TypedFnDef.
    fn make_fn(name: &str, params: &[&str], body: Vec<TypedStmt>) -> TypedFnDef {
        TypedFnDef {
            name: name.to_string(),
            params: params.iter().map(|s| s.to_string()).collect(),
            param_types: params.iter().map(|_| FluxType::Float).collect(),
            body,
            return_type: FluxType::Float,
            span: Span::new(0, 1),
        }
    }

    #[test]
    fn no_context_needed_for_pure_function() {
        // fn add(x, y) { return x + y }
        let body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(texpr(TypedExprKind::BinaryOp {
                left: Box::new(texpr(TypedExprKind::Ident("x".to_string()))),
                op: crate::parser::ast::BinOp::Add,
                right: Box::new(texpr(TypedExprKind::Ident("y".to_string()))),
            })),
            span: Span::new(0, 1),
        })];
        let functions = vec![make_fn("add", &["x", "y"], body)];
        let result = analyze_function_context(&functions);

        let ctx = result.get("add").unwrap();
        assert!(!ctx.needs_bar_context);
        assert!(!ctx.needs_signals);
    }

    #[test]
    fn needs_bar_context_when_referencing_close() {
        // fn get_price() { return close }
        let body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(texpr(TypedExprKind::Ident("close".to_string()))),
            span: Span::new(0, 1),
        })];
        let functions = vec![make_fn("get_price", &[], body)];
        let result = analyze_function_context(&functions);

        let ctx = result.get("get_price").unwrap();
        assert!(ctx.needs_bar_context);
        assert!(!ctx.needs_signals);
    }

    #[test]
    fn needs_signals_when_calling_open() {
        // fn emit_signal() { OPEN(symbol, 100.0) }
        let body = vec![TypedStmt::Expr(TypedExprStmt {
            expr: call_expr("OPEN", vec![
                texpr(TypedExprKind::Ident("symbol".to_string())),
                texpr(TypedExprKind::FloatLiteral(100.0)),
            ]),
            span: Span::new(0, 1),
        })];
        let functions = vec![make_fn("emit_signal", &[], body)];
        let result = analyze_function_context(&functions);

        let ctx = result.get("emit_signal").unwrap();
        assert!(ctx.needs_bar_context); // references symbol
        assert!(ctx.needs_signals);     // calls OPEN
    }

    #[test]
    fn transitive_bar_context_propagation() {
        // fn inner() { return close }
        // fn outer() { return inner() }
        // outer doesn't directly reference close, but calls inner which does
        let inner_body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(texpr(TypedExprKind::Ident("close".to_string()))),
            span: Span::new(0, 1),
        })];
        let outer_body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(call_expr("inner", vec![])),
            span: Span::new(0, 1),
        })];
        let functions = vec![
            make_fn("inner", &[], inner_body),
            make_fn("outer", &[], outer_body),
        ];
        let result = analyze_function_context(&functions);

        let inner_ctx = result.get("inner").unwrap();
        assert!(inner_ctx.needs_bar_context);
        assert!(!inner_ctx.needs_signals);

        let outer_ctx = result.get("outer").unwrap();
        assert!(outer_ctx.needs_bar_context); // transitive
        assert!(!outer_ctx.needs_signals);
    }

    #[test]
    fn transitive_signals_propagation() {
        // fn do_open() { OPEN(symbol, 100.0) }
        // fn wrapper() { do_open() }
        let open_body = vec![TypedStmt::Expr(TypedExprStmt {
            expr: call_expr("OPEN", vec![
                texpr(TypedExprKind::Ident("symbol".to_string())),
                texpr(TypedExprKind::FloatLiteral(100.0)),
            ]),
            span: Span::new(0, 1),
        })];
        let wrapper_body = vec![TypedStmt::Expr(TypedExprStmt {
            expr: call_expr("do_open", vec![]),
            span: Span::new(0, 1),
        })];
        let functions = vec![
            make_fn("do_open", &[], open_body),
            make_fn("wrapper", &[], wrapper_body),
        ];
        let result = analyze_function_context(&functions);

        let wrapper_ctx = result.get("wrapper").unwrap();
        assert!(wrapper_ctx.needs_bar_context); // transitive from do_open using symbol
        assert!(wrapper_ctx.needs_signals);     // transitive from do_open
    }

    #[test]
    fn param_named_close_does_not_trigger_bar_context() {
        // fn calc(close) { return close * 2.0 }
        // "close" is a param, not market data
        let body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(texpr(TypedExprKind::BinaryOp {
                left: Box::new(texpr(TypedExprKind::Ident("close".to_string()))),
                op: crate::parser::ast::BinOp::Mul,
                right: Box::new(texpr(TypedExprKind::FloatLiteral(2.0))),
            })),
            span: Span::new(0, 1),
        })];
        let functions = vec![make_fn("calc", &["close"], body)];
        let result = analyze_function_context(&functions);

        let ctx = result.get("calc").unwrap();
        assert!(!ctx.needs_bar_context); // "close" is a parameter
        assert!(!ctx.needs_signals);
    }

    #[test]
    fn multiple_market_data_vars() {
        // fn spread() { return high - low }
        let body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(texpr(TypedExprKind::BinaryOp {
                left: Box::new(texpr(TypedExprKind::Ident("high".to_string()))),
                op: crate::parser::ast::BinOp::Sub,
                right: Box::new(texpr(TypedExprKind::Ident("low".to_string()))),
            })),
            span: Span::new(0, 1),
        })];
        let functions = vec![make_fn("spread", &[], body)];
        let result = analyze_function_context(&functions);

        let ctx = result.get("spread").unwrap();
        assert!(ctx.needs_bar_context);
        assert!(!ctx.needs_signals);
    }

    #[test]
    fn deep_transitive_chain() {
        // fn a() { return close }
        // fn b() { return a() }
        // fn c() { return b() }
        // All should need bar context
        let a_body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(texpr(TypedExprKind::Ident("close".to_string()))),
            span: Span::new(0, 1),
        })];
        let b_body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(call_expr("a", vec![])),
            span: Span::new(0, 1),
        })];
        let c_body = vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(call_expr("b", vec![])),
            span: Span::new(0, 1),
        })];
        let functions = vec![
            make_fn("a", &[], a_body),
            make_fn("b", &[], b_body),
            make_fn("c", &[], c_body),
        ];
        let result = analyze_function_context(&functions);

        assert!(result.get("a").unwrap().needs_bar_context);
        assert!(result.get("b").unwrap().needs_bar_context);
        assert!(result.get("c").unwrap().needs_bar_context);
    }
}
