//! Call graph construction and cycle detection for user-defined functions.
//!
//! Builds an adjacency list of function-call edges from function bodies,
//! then detects cycles using iterative DFS with 3-color marking.

use std::collections::{HashMap, HashSet};

use crate::parser::ast::{
    ElifBranch, Expr, ExprKind, FnDef, IfStmt, Stmt,
};

/// Adjacency list for the function call graph.
/// Key: function name, Value: set of user-defined functions it calls.
pub type CallGraph = HashMap<String, HashSet<String>>;

/// Build a call graph from user-defined function definitions.
///
/// For each function, walks its body and extracts calls to other user-defined
/// functions (filtering out built-in function calls).
pub fn build_call_graph(functions: &[FnDef]) -> CallGraph {
    let fn_names: HashSet<String> = functions.iter().map(|f| f.name.clone()).collect();
    let mut graph: CallGraph = HashMap::new();

    for fn_def in functions {
        let mut callees = HashSet::new();
        extract_calls_from_stmts(&fn_def.body, &fn_names, &mut callees);
        graph.insert(fn_def.name.clone(), callees);
    }

    graph
}

/// Detect cycles in the call graph using iterative DFS with 3-color marking.
///
/// Returns `Some(cycle_path)` if a cycle is found, where cycle_path is the
/// list of function names forming the cycle (e.g., `["foo", "bar", "foo"]`
/// for mutual recursion, or `["foo", "foo"]` for direct self-recursion).
///
/// Returns `None` if the graph is acyclic.
pub fn detect_cycles(graph: &CallGraph) -> Option<Vec<String>> {
    // 3-color marking: White = unvisited, Gray = in current path, Black = fully explored
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut color: HashMap<&str, Color> = HashMap::new();
    for name in graph.keys() {
        color.insert(name.as_str(), Color::White);
    }

    // Try DFS from each unvisited node
    for start in graph.keys() {
        if color[start.as_str()] != Color::White {
            continue;
        }

        // Iterative DFS using an explicit stack
        // Each stack frame: (node, iterator index into neighbors)
        let mut stack: Vec<(&str, usize)> = vec![(start.as_str(), 0)];
        *color.get_mut(start.as_str()).unwrap() = Color::Gray;

        while let Some((node, idx)) = stack.last_mut() {
            let neighbors: Vec<&str> = graph
                .get(*node)
                .map(|s| s.iter().map(|n| n.as_str()).collect())
                .unwrap_or_default();

            if *idx >= neighbors.len() {
                // All neighbors explored — mark black and backtrack
                *color.get_mut(*node).unwrap() = Color::Black;
                stack.pop();
                continue;
            }

            let neighbor = neighbors[*idx];
            *idx += 1;

            match color.get(neighbor) {
                Some(Color::Gray) => {
                    // Found a back edge → cycle detected!
                    // Reconstruct the cycle path from the stack
                    let mut cycle = Vec::new();
                    let mut found_start = false;
                    for (frame_node, _) in &stack {
                        if *frame_node == neighbor {
                            found_start = true;
                        }
                        if found_start {
                            cycle.push(frame_node.to_string());
                        }
                    }
                    cycle.push(neighbor.to_string()); // close the cycle
                    return Some(cycle);
                }
                Some(Color::White) => {
                    // Unvisited — push onto stack
                    *color.get_mut(neighbor).unwrap() = Color::Gray;
                    stack.push((neighbor, 0));
                }
                _ => {
                    // Black — already fully explored, skip
                }
            }
        }
    }

    None
}

/// Extract function call names from a list of statements, filtering to only
/// those that are user-defined (present in `fn_names`).
fn extract_calls_from_stmts(stmts: &[Stmt], fn_names: &HashSet<String>, callees: &mut HashSet<String>) {
    for stmt in stmts {
        extract_calls_from_stmt(stmt, fn_names, callees);
    }
}

/// Extract function call names from a single statement.
fn extract_calls_from_stmt(stmt: &Stmt, fn_names: &HashSet<String>, callees: &mut HashSet<String>) {
    match stmt {
        Stmt::Assignment(assign) => {
            extract_calls_from_expr(&assign.target, fn_names, callees);
            extract_calls_from_expr(&assign.value, fn_names, callees);
        }
        Stmt::If(if_stmt) => {
            extract_calls_from_if(if_stmt, fn_names, callees);
        }
        Stmt::For(for_loop) => {
            extract_calls_from_expr(&for_loop.iterable, fn_names, callees);
            extract_calls_from_stmts(&for_loop.body, fn_names, callees);
        }
        Stmt::While(while_loop) => {
            extract_calls_from_expr(&while_loop.condition, fn_names, callees);
            extract_calls_from_stmts(&while_loop.body, fn_names, callees);
        }
        Stmt::Return(ret) => {
            if let Some(ref expr) = ret.value {
                extract_calls_from_expr(expr, fn_names, callees);
            }
        }
        Stmt::Expr(expr_stmt) => {
            extract_calls_from_expr(&expr_stmt.expr, fn_names, callees);
        }
    }
}

/// Extract function call names from an if statement (including elif/else branches).
fn extract_calls_from_if(if_stmt: &IfStmt, fn_names: &HashSet<String>, callees: &mut HashSet<String>) {
    extract_calls_from_expr(&if_stmt.condition, fn_names, callees);
    extract_calls_from_stmts(&if_stmt.body, fn_names, callees);
    for elif in &if_stmt.elif_branches {
        extract_calls_from_elif(elif, fn_names, callees);
    }
    if let Some(ref else_body) = if_stmt.else_body {
        extract_calls_from_stmts(else_body, fn_names, callees);
    }
}

/// Extract function call names from an elif branch.
fn extract_calls_from_elif(elif: &ElifBranch, fn_names: &HashSet<String>, callees: &mut HashSet<String>) {
    extract_calls_from_expr(&elif.condition, fn_names, callees);
    extract_calls_from_stmts(&elif.body, fn_names, callees);
}

/// Extract function call names from an expression, recursing into sub-expressions.
fn extract_calls_from_expr(expr: &Expr, fn_names: &HashSet<String>, callees: &mut HashSet<String>) {
    match &expr.kind {
        ExprKind::FunctionCall { function, args } => {
            // Check if the function being called is a user-defined function
            if let ExprKind::Ident(ref name) = function.kind {
                if fn_names.contains(name) {
                    callees.insert(name.clone());
                }
            }
            // Also recurse into the function expression and arguments
            extract_calls_from_expr(function, fn_names, callees);
            for arg in args {
                extract_calls_from_expr(arg, fn_names, callees);
            }
        }
        ExprKind::BinaryOp { left, right, .. } => {
            extract_calls_from_expr(left, fn_names, callees);
            extract_calls_from_expr(right, fn_names, callees);
        }
        ExprKind::UnaryOp { operand, .. } => {
            extract_calls_from_expr(operand, fn_names, callees);
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            extract_calls_from_expr(receiver, fn_names, callees);
            for arg in args {
                extract_calls_from_expr(arg, fn_names, callees);
            }
        }
        ExprKind::MemberAccess { object, .. } => {
            extract_calls_from_expr(object, fn_names, callees);
        }
        ExprKind::IndexAccess { object, index } => {
            extract_calls_from_expr(object, fn_names, callees);
            extract_calls_from_expr(index, fn_names, callees);
        }
        ExprKind::ListLiteral(elements) => {
            for elem in elements {
                extract_calls_from_expr(elem, fn_names, callees);
            }
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                extract_calls_from_expr(value, fn_names, callees);
            }
        }
        // Terminals — no further calls to extract
        ExprKind::IntLiteral(_)
        | ExprKind::FloatLiteral(_)
        | ExprKind::StringLiteral(_)
        | ExprKind::BoolLiteral(_)
        | ExprKind::NullLiteral
        | ExprKind::Ident(_) => {}
        // EnumConstruction and Match - will be fully implemented in Phase 1B
        ExprKind::EnumConstruction { args, .. } => {
            for arg in args {
                extract_calls_from_expr(arg, fn_names, callees);
            }
        }
        ExprKind::Match(match_expr) => {
            extract_calls_from_expr(&match_expr.scrutinee, fn_names, callees);
            for arm in &match_expr.arms {
                extract_calls_from_stmts(&arm.body, fn_names, callees);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;
    use crate::parser::ast::*;

    fn span() -> Span {
        Span::new(0, 1)
    }

    fn make_call_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::FunctionCall {
                function: Box::new(Expr {
                    kind: ExprKind::Ident(name.to_string()),
                    span: span(),
                }),
                args: vec![],
            },
            span: span(),
        }
    }

    fn make_fn_def(name: &str, body: Vec<Stmt>) -> FnDef {
        FnDef {
            name: name.to_string(),
            params: vec![],
            return_type: None,
            body,
            span: span(),
        }
    }

    fn make_call_stmt(name: &str) -> Stmt {
        Stmt::Expr(ExprStmt {
            expr: make_call_expr(name),
            span: span(),
        })
    }

    #[test]
    fn test_build_call_graph_direct_call() {
        let functions = vec![
            make_fn_def("foo", vec![make_call_stmt("bar")]),
            make_fn_def("bar", vec![]),
        ];

        let graph = build_call_graph(&functions);

        assert_eq!(graph["foo"], HashSet::from(["bar".to_string()]));
        assert_eq!(graph["bar"], HashSet::new());
    }

    #[test]
    fn test_build_call_graph_filters_builtins() {
        // Calls to non-user-defined functions (builtins) should be excluded
        let functions = vec![
            make_fn_def("foo", vec![make_call_stmt("sma"), make_call_stmt("bar")]),
            make_fn_def("bar", vec![]),
        ];

        let graph = build_call_graph(&functions);

        // "sma" should not appear since it's not in fn_names
        assert_eq!(graph["foo"], HashSet::from(["bar".to_string()]));
    }

    #[test]
    fn test_detect_cycles_direct_self_recursion() {
        let mut graph: CallGraph = HashMap::new();
        graph.insert("foo".to_string(), HashSet::from(["foo".to_string()]));

        let cycle = detect_cycles(&graph);
        assert!(cycle.is_some());
        let path = cycle.unwrap();
        assert_eq!(path.first(), path.last());
        assert!(path.contains(&"foo".to_string()));
    }

    #[test]
    fn test_detect_cycles_mutual_recursion() {
        let mut graph: CallGraph = HashMap::new();
        graph.insert("foo".to_string(), HashSet::from(["bar".to_string()]));
        graph.insert("bar".to_string(), HashSet::from(["foo".to_string()]));

        let cycle = detect_cycles(&graph);
        assert!(cycle.is_some());
        let path = cycle.unwrap();
        // The cycle should close (first == last)
        assert_eq!(path.first(), path.last());
    }

    #[test]
    fn test_detect_cycles_three_function_cycle() {
        let mut graph: CallGraph = HashMap::new();
        graph.insert("a".to_string(), HashSet::from(["b".to_string()]));
        graph.insert("b".to_string(), HashSet::from(["c".to_string()]));
        graph.insert("c".to_string(), HashSet::from(["a".to_string()]));

        let cycle = detect_cycles(&graph);
        assert!(cycle.is_some());
        let path = cycle.unwrap();
        assert_eq!(path.first(), path.last());
    }

    #[test]
    fn test_detect_cycles_acyclic_chain() {
        let mut graph: CallGraph = HashMap::new();
        graph.insert("a".to_string(), HashSet::from(["b".to_string()]));
        graph.insert("b".to_string(), HashSet::from(["c".to_string()]));
        graph.insert("c".to_string(), HashSet::new());

        let cycle = detect_cycles(&graph);
        assert!(cycle.is_none());
    }

    #[test]
    fn test_detect_cycles_diamond_no_cycle() {
        let mut graph: CallGraph = HashMap::new();
        graph.insert("a".to_string(), HashSet::from(["b".to_string(), "c".to_string()]));
        graph.insert("b".to_string(), HashSet::from(["d".to_string()]));
        graph.insert("c".to_string(), HashSet::from(["d".to_string()]));
        graph.insert("d".to_string(), HashSet::new());

        let cycle = detect_cycles(&graph);
        assert!(cycle.is_none());
    }

    #[test]
    fn test_detect_cycles_empty_graph() {
        let graph: CallGraph = HashMap::new();
        let cycle = detect_cycles(&graph);
        assert!(cycle.is_none());
    }

    #[test]
    fn test_detect_cycles_isolated_nodes() {
        let mut graph: CallGraph = HashMap::new();
        graph.insert("a".to_string(), HashSet::new());
        graph.insert("b".to_string(), HashSet::new());
        graph.insert("c".to_string(), HashSet::new());

        let cycle = detect_cycles(&graph);
        assert!(cycle.is_none());
    }
}
