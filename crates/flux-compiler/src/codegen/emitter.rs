//! Code emitter: walks the TypedProgram and produces Rust source code.

use std::collections::HashSet;

use crate::error::{CompileError, Result};
use crate::parser::ast::{BinOp, UnaryOp};
use crate::typeck::typed_ast::*;
use crate::typeck::types::FluxType;

use super::type_map::map_type;

/// Known market data identifiers available through `ctx`.
const MARKET_DATA: &[&str] = &[
    "close", "open", "high", "low", "volume", "symbol", "in_position",
];

/// Known signal-producing function names.
const SIGNAL_FUNCTIONS: &[&str] = &["OPEN", "CLOSE"];

/// The code emitter accumulates Rust source code by walking the typed AST.
pub(crate) struct CodeEmitter<'a> {
    /// Reference to the input typed program.
    program: &'a TypedProgram,
    /// Accumulated output string buffer.
    output: String,
    /// Current indentation level (0 = top-level).
    indent_level: usize,
    /// Parameter names (accessed via `self.`).
    params: HashSet<String>,
    /// State variable names (accessed via `self.`).
    state_vars: HashSet<String>,
    /// Property names (accessed via `self.`).
    properties: HashSet<String>,
    /// Local variables declared in the current handler scope.
    local_vars: HashSet<String>,
    /// Imported function names (accessed as bare names).
    imported_functions: HashSet<String>,
}

impl<'a> CodeEmitter<'a> {
    /// Create a new CodeEmitter, collecting context from the TypedProgram.
    pub fn new(program: &'a TypedProgram) -> Self {
        let mut params = HashSet::new();
        let mut state_vars = HashSet::new();
        let mut properties = HashSet::new();
        let mut imported_functions = HashSet::new();

        // Collect imported function names
        for import in &program.imports {
            for name in &import.names {
                imported_functions.insert(name.clone());
            }
        }

        // Walk strategy body to collect params, state vars, and properties
        for item in &program.strategy.body {
            match item {
                TypedStrategyItem::ParamsBlock(pb) => {
                    for p in &pb.params {
                        params.insert(p.name.clone());
                    }
                }
                TypedStrategyItem::StateBlock(sb) => {
                    for v in &sb.variables {
                        state_vars.insert(v.name.clone());
                    }
                }
                TypedStrategyItem::Property(prop) => {
                    properties.insert(prop.name.clone());
                }
                TypedStrategyItem::EventHandler(_) => {}
            }
        }

        Self {
            program,
            output: String::new(),
            indent_level: 0,
            params,
            state_vars,
            properties,
            local_vars: HashSet::new(),
            imported_functions,
        }
    }

    // ========================================================================
    // Public entry point
    // ========================================================================

    /// Main entry point: emit the full Rust source file.
    pub fn emit(&mut self) -> Result<String> {
        self.local_vars.clear();
        self.output.clear();
        self.emit_preamble();
        self.emit_struct()?;
        self.output.push('\n');
        self.emit_default_impl()?;
        self.output.push('\n');
        self.emit_strategy_impl()?;
        Ok(self.output.clone())
    }

    // ========================================================================
    // Top-level structure emission
    // ========================================================================

    /// Emit the preamble: `use flux_runtime::*;\n\n`
    fn emit_preamble(&mut self) {
        self.output.push_str("use flux_runtime::*;\n\n");
    }

    /// Emit the struct definition with property, param, and state fields.
    fn emit_struct(&mut self) -> Result<()> {
        let strategy_name = &self.program.strategy.name.clone();
        self.output
            .push_str(&format!("pub struct {} {{\n", strategy_name));

        // Collect fields in order: properties, params, state
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::Property(prop) = item {
                let type_str = map_type(&prop.value.resolved_type, prop.span.start)?;
                self.output
                    .push_str(&format!("    pub {}: {},\n", prop.name, type_str));
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::ParamsBlock(pb) = item {
                for param in &pb.params {
                    let type_str = map_type(&param.resolved_type, param.span.start)?;
                    self.output
                        .push_str(&format!("    pub {}: {},\n", param.name, type_str));
                }
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::StateBlock(sb) = item {
                for var in &sb.variables {
                    let type_str = map_type(&var.resolved_type, var.span.start)?;
                    self.output
                        .push_str(&format!("    {}: {},\n", var.name, type_str));
                }
            }
        }

        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit the `impl Default for N { fn default() -> Self { Self { ... } } }` block.
    fn emit_default_impl(&mut self) -> Result<()> {
        let strategy_name = &self.program.strategy.name.clone();
        self.output
            .push_str(&format!("impl Default for {} {{\n", strategy_name));
        self.output.push_str("    fn default() -> Self {\n");
        self.output.push_str("        Self {\n");

        // Emit default values in order: properties, params, state
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::Property(prop) = item {
                self.output
                    .push_str(&format!("            {}: ", prop.name));
                self.emit_expr(&prop.value.clone())?;
                self.output.push_str(",\n");
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::ParamsBlock(pb) = item {
                for param in &pb.params {
                    self.output
                        .push_str(&format!("            {}: ", param.name));
                    self.emit_expr(&param.default_value.clone())?;
                    self.output.push_str(",\n");
                }
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::StateBlock(sb) = item {
                for var in &sb.variables {
                    self.output
                        .push_str(&format!("            {}: ", var.name));
                    self.emit_expr(&var.initial_value.clone())?;
                    self.output.push_str(",\n");
                }
            }
        }

        self.output.push_str("        }\n");
        self.output.push_str("    }\n");
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit the `impl Strategy for N { ... }` block with event handler methods.
    fn emit_strategy_impl(&mut self) -> Result<()> {
        let strategy_name = &self.program.strategy.name.clone();
        self.output
            .push_str(&format!("impl Strategy for {} {{\n", strategy_name));

        let handlers: Vec<TypedEventHandler> = self
            .program
            .strategy
            .body
            .iter()
            .filter_map(|item| {
                if let TypedStrategyItem::EventHandler(handler) = item {
                    Some(handler.clone())
                } else {
                    None
                }
            })
            .collect();

        for handler in &handlers {
            self.emit_event_handler(handler)?;
        }

        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a single event handler method.
    fn emit_event_handler(&mut self, handler: &TypedEventHandler) -> Result<()> {
        // Clear local vars for fresh handler scope
        self.local_vars.clear();

        // Method signature
        self.output.push_str(&format!(
            "    fn on_{}(&mut self, ctx: &BarContext) -> Vec<Signal> {{\n",
            handler.event_name
        ));

        // Signal declaration
        self.output
            .push_str("        let mut signals: Vec<Signal> = Vec::new();\n");

        // Body statements
        self.indent_level = 2;
        for stmt in &handler.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level = 0;

        // Return signals
        self.output.push_str("        signals\n");

        // Close method
        self.output.push_str("    }\n");
        Ok(())
    }

    // ========================================================================
    // Statement emission (task 3.1)
    // ========================================================================

    /// Dispatch statement emission based on the `TypedStmt` variant.
    fn emit_stmt(&mut self, stmt: &TypedStmt) -> Result<()> {
        match stmt {
            TypedStmt::Assignment(assign) => self.emit_assignment(assign),
            TypedStmt::If(if_stmt) => self.emit_if_stmt(if_stmt),
            TypedStmt::For(for_loop) => self.emit_for_loop(for_loop),
            TypedStmt::While(while_loop) => self.emit_while_loop(while_loop),
            TypedStmt::Return(ret) => self.emit_return(ret),
            TypedStmt::Expr(expr_stmt) => self.emit_expr_stmt(expr_stmt),
        }
    }

    /// Emit an assignment statement.
    ///
    /// Handles four cases:
    /// - Index target: `object[index] = value;`
    /// - State variable: `self.name = value;`
    /// - New local variable: `let mut name = value;`
    /// - Existing local variable: `name = value;`
    fn emit_assignment(&mut self, assign: &TypedAssignment) -> Result<()> {
        self.write_indent();
        match &assign.target.kind {
            TypedExprKind::IndexAccess { object, index } => {
                self.emit_expr(object)?;
                self.output.push('[');
                self.emit_expr(index)?;
                self.output.push_str("] = ");
                self.emit_expr(&assign.value)?;
                self.output.push_str(";\n");
            }
            TypedExprKind::Ident(name) => {
                if self.state_vars.contains(name) {
                    self.output.push_str(&format!("self.{} = ", name));
                    self.emit_expr(&assign.value)?;
                    self.output.push_str(";\n");
                } else if self.is_new_local(name) {
                    self.output.push_str(&format!("let mut {} = ", name));
                    self.emit_expr(&assign.value)?;
                    self.output.push_str(";\n");
                    self.local_vars.insert(name.clone());
                } else {
                    self.output.push_str(&format!("{} = ", name));
                    self.emit_expr(&assign.value)?;
                    self.output.push_str(";\n");
                }
            }
            _ => {
                // Fallback: emit target expression directly
                self.emit_expr(&assign.target)?;
                self.output.push_str(" = ");
                self.emit_expr(&assign.value)?;
                self.output.push_str(";\n");
            }
        }
        Ok(())
    }

    /// Emit an if/elif/else statement with proper indentation.
    fn emit_if_stmt(&mut self, if_stmt: &TypedIfStmt) -> Result<()> {
        // if condition {
        self.write_indent();
        self.output.push_str("if ");
        self.emit_expr(&if_stmt.condition)?;
        self.output.push_str(" {\n");

        // body
        self.indent_level += 1;
        for stmt in &if_stmt.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level -= 1;

        // elif branches
        for elif in &if_stmt.elif_branches {
            self.write_indent();
            self.output.push_str("} else if ");
            self.emit_expr(&elif.condition)?;
            self.output.push_str(" {\n");

            self.indent_level += 1;
            for stmt in &elif.body {
                self.emit_stmt(stmt)?;
            }
            self.indent_level -= 1;
        }

        // else branch
        if let Some(else_body) = &if_stmt.else_body {
            self.write_indent();
            self.output.push_str("} else {\n");

            self.indent_level += 1;
            for stmt in else_body {
                self.emit_stmt(stmt)?;
            }
            self.indent_level -= 1;
        }

        // closing brace
        self.write_indent();
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a for loop: `for variable in iterable { body }`.
    ///
    /// Registers the loop variable as a local before emitting the body.
    fn emit_for_loop(&mut self, for_loop: &TypedForLoop) -> Result<()> {
        self.write_indent();
        self.output.push_str(&format!("for {} in ", for_loop.variable));
        self.emit_expr(&for_loop.iterable)?;
        self.output.push_str(" {\n");

        // Register loop variable as local
        self.local_vars.insert(for_loop.variable.clone());

        self.indent_level += 1;
        for stmt in &for_loop.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level -= 1;

        self.write_indent();
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a while loop: `while condition { body }`.
    fn emit_while_loop(&mut self, while_loop: &TypedWhileLoop) -> Result<()> {
        self.write_indent();
        self.output.push_str("while ");
        self.emit_expr(&while_loop.condition)?;
        self.output.push_str(" {\n");

        self.indent_level += 1;
        for stmt in &while_loop.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level -= 1;

        self.write_indent();
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a return statement: `return value;` or `return;`.
    fn emit_return(&mut self, ret: &TypedReturnStmt) -> Result<()> {
        self.write_indent();
        if let Some(value) = &ret.value {
            self.output.push_str("return ");
            self.emit_expr(value)?;
            self.output.push_str(";\n");
        } else {
            self.output.push_str("return;\n");
        }
        Ok(())
    }

    /// Emit an expression statement.
    ///
    /// Signal-producing expressions are wrapped in `signals.push(...)`.
    /// Other expressions are emitted as standalone statements.
    fn emit_expr_stmt(&mut self, expr_stmt: &TypedExprStmt) -> Result<()> {
        self.write_indent();
        if self.is_signal_expr(&expr_stmt.expr) {
            self.output.push_str("signals.push(");
            self.emit_expr(&expr_stmt.expr)?;
            self.output.push_str(");\n");
        } else {
            self.emit_expr(&expr_stmt.expr)?;
            self.output.push_str(";\n");
        }
        Ok(())
    }

    // ========================================================================
    // Expression emission
    // ========================================================================

    /// Emit a typed expression to the output buffer.
    pub(crate) fn emit_expr(&mut self, expr: &TypedExpr) -> Result<()> {
        match &expr.kind {
            TypedExprKind::IntLiteral(v) => {
                self.output.push_str(&v.to_string());
            }
            TypedExprKind::FloatLiteral(v) => {
                let s = v.to_string();
                self.output.push_str(&s);
                // Ensure the float literal always contains a decimal point
                if !s.contains('.') {
                    self.output.push_str(".0");
                }
            }
            TypedExprKind::StringLiteral(s) => {
                let escaped = s
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t");
                self.output.push_str(&format!("String::from(\"{}\")", escaped));
            }
            TypedExprKind::BoolLiteral(b) => {
                self.output.push_str(if *b { "true" } else { "false" });
            }
            TypedExprKind::NullLiteral => {
                self.output.push_str("()");
            }
            TypedExprKind::ListLiteral(elems) => {
                self.output.push_str("vec![");
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_expr(elem)?;
                }
                self.output.push(']');
            }
            TypedExprKind::Ident(name) => {
                let resolved = self.resolve_ident(name);
                self.output.push_str(&resolved);
            }
            TypedExprKind::BinaryOp { left, op, right } => {
                self.emit_binary_op(left, *op, right)?;
            }
            TypedExprKind::UnaryOp { op, operand } => {
                self.emit_unary_op(*op, operand)?;
            }
            TypedExprKind::FunctionCall { function, args } => {
                self.emit_function_call(function, args)?;
            }
            TypedExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                self.emit_method_call(receiver, method, args)?;
            }
            TypedExprKind::MemberAccess { object, field } => {
                self.emit_member_access(object, field)?;
            }
            TypedExprKind::IndexAccess { object, index } => {
                self.emit_index_access(object, index)?;
            }
        }
        Ok(())
    }

    /// Emit a binary operation expression.
    fn emit_binary_op(&mut self, left: &TypedExpr, op: BinOp, right: &TypedExpr) -> Result<()> {
        // Special case: String + String → format!("{}{}", left, right)
        if op == BinOp::Add
            && left.resolved_type == FluxType::String
            && right.resolved_type == FluxType::String
        {
            self.output.push_str("format!(\"{}{}\", ");
            self.emit_expr(left)?;
            self.output.push_str(", ");
            self.emit_expr(right)?;
            self.output.push(')');
            return Ok(());
        }

        let op_str = match op {
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
            BinOp::And => "&&",
            BinOp::Or => "||",
        };

        self.output.push('(');

        // Left operand: cast to f64 if needed
        if self.needs_cast_to_f64(left, &right.resolved_type) {
            self.output.push('(');
            self.emit_expr(left)?;
            self.output.push_str(" as f64)");
        } else {
            self.emit_expr(left)?;
        }

        self.output.push(' ');
        self.output.push_str(op_str);
        self.output.push(' ');

        // Right operand: cast to f64 if needed
        if self.needs_cast_to_f64(right, &left.resolved_type) {
            self.output.push('(');
            self.emit_expr(right)?;
            self.output.push_str(" as f64)");
        } else {
            self.emit_expr(right)?;
        }

        self.output.push(')');
        Ok(())
    }

    /// Emit a unary operation expression.
    fn emit_unary_op(&mut self, op: UnaryOp, operand: &TypedExpr) -> Result<()> {
        match op {
            UnaryOp::Neg => {
                self.output.push_str("(-");
                self.emit_expr(operand)?;
                self.output.push(')');
            }
            UnaryOp::Not => {
                self.output.push_str("(!");
                self.emit_expr(operand)?;
                self.output.push(')');
            }
        }
        Ok(())
    }

    /// Emit a function call expression.
    ///
    /// Handles signal functions (OPEN, CLOSE) specially, mapping them to
    /// the runtime Signal API. Other functions are emitted as direct calls.
    fn emit_function_call(&mut self, function: &TypedExpr, args: &[TypedExpr]) -> Result<()> {
        // Check if this is a signal function call
        if let TypedExprKind::Ident(ref name) = function.kind {
            match name.as_str() {
                "OPEN" => {
                    self.output.push_str("Signal::open(");
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.output.push_str(", ");
                        }
                        self.emit_expr(arg)?;
                    }
                    self.output.push(')');
                    return Ok(());
                }
                "CLOSE" => {
                    if args.len() == 1 {
                        self.output.push_str("Signal::close(");
                        self.emit_expr(&args[0])?;
                        self.output.push(')');
                    } else {
                        // 2 args → Signal::close_qty
                        self.output.push_str("Signal::close_qty(");
                        self.emit_expr(&args[0])?;
                        self.output.push_str(", ");
                        self.emit_expr(&args[1])?;
                        self.output.push(')');
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        // Regular function call: name(args...)
        self.emit_expr(function)?;
        self.output.push('(');
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            self.emit_expr(arg)?;
        }
        self.output.push(')');
        Ok(())
    }

    /// Emit a method call: `receiver.method(args...)`.
    fn emit_method_call(
        &mut self,
        receiver: &TypedExpr,
        method: &str,
        args: &[TypedExpr],
    ) -> Result<()> {
        self.emit_expr(receiver)?;
        self.output.push('.');
        self.output.push_str(method);
        self.output.push('(');
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            self.emit_expr(arg)?;
        }
        self.output.push(')');
        Ok(())
    }

    /// Emit a member access: `object.field`.
    fn emit_member_access(&mut self, object: &TypedExpr, field: &str) -> Result<()> {
        self.emit_expr(object)?;
        self.output.push('.');
        self.output.push_str(field);
        Ok(())
    }

    /// Emit an index access: `object[index]`.
    fn emit_index_access(&mut self, object: &TypedExpr, index: &TypedExpr) -> Result<()> {
        self.emit_expr(object)?;
        self.output.push('[');
        self.emit_expr(index)?;
        self.output.push(']');
        Ok(())
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Resolve an identifier to its correct Rust expression form.
    ///
    /// Priority: MARKET_DATA → local_vars → params/state/properties → bare name
    fn resolve_ident(&self, name: &str) -> String {
        if MARKET_DATA.contains(&name) {
            format!("ctx.{}", name)
        } else if self.local_vars.contains(name) {
            name.to_string()
        } else if self.params.contains(name)
            || self.state_vars.contains(name)
            || self.properties.contains(name)
        {
            format!("self.{}", name)
        } else {
            // Imported function or unknown — emit bare name
            name.to_string()
        }
    }

    /// Write indentation (4 spaces per level) to the output buffer.
    #[allow(dead_code)]
    fn write_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.output.push_str("    ");
        }
    }

    /// Check if an expression needs to be cast to f64.
    ///
    /// Returns true if `expr` has type Int and the `other_type` is Float,
    /// meaning this operand needs an `as f64` cast for the binary op.
    fn needs_cast_to_f64(&self, expr: &TypedExpr, other_type: &FluxType) -> bool {
        expr.resolved_type == FluxType::Int && *other_type == FluxType::Float
    }

    /// Check if an expression produces a Signal value.
    #[allow(dead_code)]
    pub(crate) fn is_signal_expr(&self, expr: &TypedExpr) -> bool {
        expr.resolved_type == FluxType::Signal
    }

    /// Check if a name is a new local variable (not yet declared).
    ///
    /// Returns true if the name is NOT in params, state_vars, properties,
    /// or local_vars — i.e., it's being assigned for the first time.
    #[allow(dead_code)]
    pub(crate) fn is_new_local(&self, name: &str) -> bool {
        !self.params.contains(name)
            && !self.state_vars.contains(name)
            && !self.properties.contains(name)
            && !self.local_vars.contains(name)
    }

    /// Construct a codegen error with byte offset.
    #[allow(dead_code)]
    pub(crate) fn error(&self, message: &str, span_start: usize) -> CompileError {
        CompileError::Codegen(format!("at byte {}: {}", span_start, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;
    use crate::parser::ast::Import;

    /// Helper: build a minimal TypedProgram with no params, state, or handlers.
    fn minimal_program() -> TypedProgram {
        TypedProgram {
            imports: vec![],
            strategy: TypedStrategy {
                name: "Test".to_string(),
                body: vec![],
                span: Span::new(0, 10),
            },
            span: Span::new(0, 10),
        }
    }

    /// Helper: build a TypedProgram with params, state, properties, and imports.
    fn full_context_program() -> TypedProgram {
        TypedProgram {
            imports: vec![Import {
                module_path: "indicators".to_string(),
                names: vec!["sma".to_string(), "ema".to_string()],
                span: Span::new(0, 30),
            }],
            strategy: TypedStrategy {
                name: "MyStrategy".to_string(),
                body: vec![
                    TypedStrategyItem::Property(TypedProperty {
                        name: "book_side".to_string(),
                        value: TypedExpr {
                            kind: TypedExprKind::IntLiteral(1),
                            resolved_type: FluxType::Int,
                            span: Span::new(40, 41),
                        },
                        span: Span::new(35, 45),
                    }),
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "period".to_string(),
                            default_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(20),
                                resolved_type: FluxType::Int,
                                span: Span::new(60, 62),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(50, 62),
                        }],
                        span: Span::new(48, 65),
                    }),
                    TypedStrategyItem::StateBlock(TypedStateBlock {
                        variables: vec![TypedStateVar {
                            name: "count".to_string(),
                            initial_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(0),
                                resolved_type: FluxType::Int,
                                span: Span::new(80, 81),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(70, 81),
                        }],
                        span: Span::new(68, 85),
                    }),
                ],
                span: Span::new(32, 100),
            },
            span: Span::new(0, 100),
        }
    }

    /// Helper: create a typed expression with a given kind and type.
    fn typed_expr(kind: TypedExprKind, resolved_type: FluxType) -> TypedExpr {
        TypedExpr {
            kind,
            resolved_type,
            span: Span::new(0, 1),
        }
    }

    // ===== Constructor tests =====

    #[test]
    fn new_collects_params() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.params.contains("period"));
    }

    #[test]
    fn new_collects_state_vars() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.state_vars.contains("count"));
    }

    #[test]
    fn new_collects_properties() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.properties.contains("book_side"));
    }

    #[test]
    fn new_collects_imported_functions() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.imported_functions.contains("sma"));
        assert!(emitter.imported_functions.contains("ema"));
    }

    // ===== resolve_ident tests =====

    #[test]
    fn resolve_ident_market_data() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("close"), "ctx.close");
        assert_eq!(emitter.resolve_ident("open"), "ctx.open");
        assert_eq!(emitter.resolve_ident("high"), "ctx.high");
        assert_eq!(emitter.resolve_ident("low"), "ctx.low");
        assert_eq!(emitter.resolve_ident("volume"), "ctx.volume");
        assert_eq!(emitter.resolve_ident("symbol"), "ctx.symbol");
        assert_eq!(emitter.resolve_ident("in_position"), "ctx.in_position");
    }

    #[test]
    fn resolve_ident_local_var() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("zscore".to_string());
        assert_eq!(emitter.resolve_ident("zscore"), "zscore");
    }

    #[test]
    fn resolve_ident_param() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("period"), "self.period");
    }

    #[test]
    fn resolve_ident_state_var() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("count"), "self.count");
    }

    #[test]
    fn resolve_ident_property() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("book_side"), "self.book_side");
    }

    #[test]
    fn resolve_ident_imported_function() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("sma"), "sma");
    }

    #[test]
    fn resolve_ident_unknown() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("unknown_thing"), "unknown_thing");
    }

    #[test]
    fn resolve_ident_local_takes_priority_over_param() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        // If a local shadows a param, local wins
        emitter.local_vars.insert("period".to_string());
        assert_eq!(emitter.resolve_ident("period"), "period");
    }

    // ===== emit_expr tests: Literals =====

    #[test]
    fn emit_int_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "42");
    }

    #[test]
    fn emit_int_literal_negative() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(-7), FluxType::Int);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "-7");
    }

    #[test]
    fn emit_float_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::FloatLiteral(3.14), FluxType::Float);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "3.14");
    }

    #[test]
    fn emit_float_literal_whole_number() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::FloatLiteral(2.0), FluxType::Float);
        emitter.emit_expr(&expr).unwrap();
        // Must contain a decimal point
        assert!(emitter.output.contains('.'), "Float must contain decimal point");
        assert_eq!(emitter.output, "2.0");
    }

    #[test]
    fn emit_string_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::StringLiteral("hello".to_string()),
            FluxType::String,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "String::from(\"hello\")");
    }

    #[test]
    fn emit_string_literal_with_escaping() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::StringLiteral("say \"hi\"\nnewline".to_string()),
            FluxType::String,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "String::from(\"say \\\"hi\\\"\\nnewline\")"
        );
    }

    #[test]
    fn emit_bool_true() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "true");
    }

    #[test]
    fn emit_bool_false() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "false");
    }

    #[test]
    fn emit_null_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::NullLiteral, FluxType::Null);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "()");
    }

    #[test]
    fn emit_list_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::ListLiteral(vec![
                typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            ]),
            FluxType::List(Box::new(FluxType::Int)),
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "vec![1, 2, 3]");
    }

    #[test]
    fn emit_list_literal_empty() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::ListLiteral(vec![]),
            FluxType::List(Box::new(FluxType::Null)),
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "vec![]");
    }

    // ===== emit_expr tests: Identifiers =====

    #[test]
    fn emit_ident_market_data() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("close".to_string()),
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "ctx.close");
    }

    #[test]
    fn emit_ident_param() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("period".to_string()),
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "self.period");
    }

    #[test]
    fn emit_ident_state_var() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("count".to_string()),
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "self.count");
    }

    #[test]
    fn emit_ident_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("zscore".to_string());
        let expr = typed_expr(
            TypedExprKind::Ident("zscore".to_string()),
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "zscore");
    }

    #[test]
    fn emit_ident_imported_function() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("sma".to_string()),
            FluxType::Fn {
                params: crate::typeck::types::FnParams::VariadicNumeric,
                ret: Box::new(FluxType::Float),
            },
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "sma");
    }

    // ===== emit_expr tests: Binary Operations =====

    #[test]
    fn emit_binary_add() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Add,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 + 2)");
    }

    #[test]
    fn emit_binary_sub() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int)),
                op: BinOp::Sub,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(5 - 3)");
    }

    #[test]
    fn emit_binary_mul() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(4), FluxType::Int)),
                op: BinOp::Mul,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(6), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(4 * 6)");
    }

    #[test]
    fn emit_binary_div() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int)),
                op: BinOp::Div,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(10 / 2)");
    }

    #[test]
    fn emit_binary_mod() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int)),
                op: BinOp::Mod,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(10 % 3)");
    }

    #[test]
    fn emit_binary_eq() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Eq,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 == 1)");
    }

    #[test]
    fn emit_binary_ne() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Ne,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 != 2)");
    }

    #[test]
    fn emit_binary_lt() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Lt,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 < 2)");
    }

    #[test]
    fn emit_binary_le() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Le,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 <= 2)");
    }

    #[test]
    fn emit_binary_gt() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
                op: BinOp::Gt,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(2 > 1)");
    }

    #[test]
    fn emit_binary_ge() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
                op: BinOp::Ge,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(2 >= 1)");
    }

    #[test]
    fn emit_binary_and() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool)),
                op: BinOp::And,
                right: Box::new(typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(true && false)");
    }

    #[test]
    fn emit_binary_or() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool)),
                op: BinOp::Or,
                right: Box::new(typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(true || false)");
    }

    // ===== Numeric coercion =====

    #[test]
    fn emit_binary_int_plus_float_casts_left() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Add,
                right: Box::new(typed_expr(TypedExprKind::FloatLiteral(2.5), FluxType::Float)),
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "((1 as f64) + 2.5)");
    }

    #[test]
    fn emit_binary_float_plus_int_casts_right() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::FloatLiteral(2.5), FluxType::Float)),
                op: BinOp::Add,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(2.5 + (1 as f64))");
    }

    #[test]
    fn emit_binary_same_type_no_cast() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::FloatLiteral(1.0), FluxType::Float)),
                op: BinOp::Mul,
                right: Box::new(typed_expr(TypedExprKind::FloatLiteral(2.0), FluxType::Float)),
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1.0 * 2.0)");
        assert!(!emitter.output.contains("as f64"));
    }

    // ===== String concatenation =====

    #[test]
    fn emit_string_concat_uses_format() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(
                    TypedExprKind::StringLiteral("hello".to_string()),
                    FluxType::String,
                )),
                op: BinOp::Add,
                right: Box::new(typed_expr(
                    TypedExprKind::StringLiteral(" world".to_string()),
                    FluxType::String,
                )),
            },
            FluxType::String,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "format!(\"{}{}\", String::from(\"hello\"), String::from(\" world\"))"
        );
        assert!(!emitter.output.contains(" + "));
    }

    // ===== Unary operations =====

    #[test]
    fn emit_unary_neg() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::UnaryOp {
                op: UnaryOp::Neg,
                operand: Box::new(typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(-5)");
    }

    #[test]
    fn emit_unary_not() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(!true)");
    }

    // ===== Function calls =====

    #[test]
    fn emit_function_call_indicator() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("sma".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::VariadicNumeric,
                        ret: Box::new(FluxType::Float),
                    },
                )),
                args: vec![
                    typed_expr(TypedExprKind::Ident("close".to_string()), FluxType::Float),
                    typed_expr(TypedExprKind::IntLiteral(20), FluxType::Int),
                ],
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "sma(ctx.close, 20)");
    }

    #[test]
    fn emit_function_call_open() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("OPEN".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::Fixed(vec![
                            FluxType::String,
                            FluxType::Int,
                        ]),
                        ret: Box::new(FluxType::Signal),
                    },
                )),
                args: vec![
                    typed_expr(TypedExprKind::Ident("symbol".to_string()), FluxType::String),
                    typed_expr(TypedExprKind::IntLiteral(100), FluxType::Int),
                ],
            },
            FluxType::Signal,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "Signal::open(ctx.symbol, 100)");
    }

    #[test]
    fn emit_function_call_close_one_arg() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("CLOSE".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::Fixed(vec![FluxType::String]),
                        ret: Box::new(FluxType::Signal),
                    },
                )),
                args: vec![typed_expr(
                    TypedExprKind::Ident("symbol".to_string()),
                    FluxType::String,
                )],
            },
            FluxType::Signal,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "Signal::close(ctx.symbol)");
    }

    #[test]
    fn emit_function_call_close_two_args() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("CLOSE".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::Fixed(vec![
                            FluxType::String,
                            FluxType::Int,
                        ]),
                        ret: Box::new(FluxType::Signal),
                    },
                )),
                args: vec![
                    typed_expr(TypedExprKind::Ident("symbol".to_string()), FluxType::String),
                    typed_expr(TypedExprKind::IntLiteral(50), FluxType::Int),
                ],
            },
            FluxType::Signal,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "Signal::close_qty(ctx.symbol, 50)");
    }

    // ===== Method calls, member access, index access =====

    #[test]
    fn emit_method_call() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("count".to_string()),
                    FluxType::Int,
                )),
                method: "abs".to_string(),
                args: vec![],
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "self.count.abs()");
    }

    #[test]
    fn emit_method_call_with_args() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("prices".to_string());
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("prices".to_string()),
                    FluxType::List(Box::new(FluxType::Float)),
                )),
                method: "append".to_string(),
                args: vec![typed_expr(TypedExprKind::FloatLiteral(1.5), FluxType::Float)],
            },
            FluxType::Void,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "prices.append(1.5)");
    }

    #[test]
    fn emit_member_access() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("obj".to_string());
        let expr = typed_expr(
            TypedExprKind::MemberAccess {
                object: Box::new(typed_expr(
                    TypedExprKind::Ident("obj".to_string()),
                    FluxType::Int, // type doesn't matter here
                )),
                field: "x".to_string(),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "obj.x");
    }

    #[test]
    fn emit_index_access() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("arr".to_string());
        let expr = typed_expr(
            TypedExprKind::IndexAccess {
                object: Box::new(typed_expr(
                    TypedExprKind::Ident("arr".to_string()),
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                index: Box::new(typed_expr(TypedExprKind::IntLiteral(0), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "arr[0]");
    }

    // ===== Helper method tests =====

    #[test]
    fn is_new_local_true_for_unknown() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.is_new_local("brand_new"));
    }

    #[test]
    fn is_new_local_false_for_param() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(!emitter.is_new_local("period"));
    }

    #[test]
    fn is_new_local_false_for_state() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(!emitter.is_new_local("count"));
    }

    #[test]
    fn is_new_local_false_for_existing_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("x".to_string());
        assert!(!emitter.is_new_local("x"));
    }

    #[test]
    fn is_signal_expr_true() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::NullLiteral, FluxType::Signal);
        assert!(emitter.is_signal_expr(&expr));
    }

    #[test]
    fn is_signal_expr_false() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int);
        assert!(!emitter.is_signal_expr(&expr));
    }

    #[test]
    fn error_helper_format() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let err = emitter.error("something went wrong", 42);
        match err {
            CompileError::Codegen(msg) => {
                assert_eq!(msg, "at byte 42: something went wrong");
            }
            _ => panic!("Expected Codegen error"),
        }
    }

    #[test]
    fn needs_cast_to_f64_int_vs_float() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int);
        assert!(emitter.needs_cast_to_f64(&expr, &FluxType::Float));
    }

    #[test]
    fn needs_cast_to_f64_float_vs_float() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::FloatLiteral(1.0), FluxType::Float);
        assert!(!emitter.needs_cast_to_f64(&expr, &FluxType::Float));
    }

    #[test]
    fn needs_cast_to_f64_int_vs_int() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int);
        assert!(!emitter.needs_cast_to_f64(&expr, &FluxType::Int));
    }

    // ===== Statement emission tests (task 3.1) =====

    #[test]
    fn emit_stmt_dispatches_assignment() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let stmt = TypedStmt::Assignment(TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int),
            span: Span::new(0, 10),
        });
        emitter.emit_stmt(&stmt).unwrap();
        assert_eq!(emitter.output, "let mut x = 42;\n");
    }

    #[test]
    fn emit_assignment_new_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("zscore".to_string()), FluxType::Float),
            value: typed_expr(TypedExprKind::FloatLiteral(1.5), FluxType::Float),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "let mut zscore = 1.5;\n");
        assert!(emitter.local_vars.contains("zscore"));
    }

    #[test]
    fn emit_assignment_existing_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("x".to_string());
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "x = 10;\n");
    }

    #[test]
    fn emit_assignment_state_var() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("count".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "self.count = 5;\n");
    }

    #[test]
    fn emit_assignment_index_target() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("arr".to_string());
        let assign = TypedAssignment {
            target: typed_expr(
                TypedExprKind::IndexAccess {
                    object: Box::new(typed_expr(
                        TypedExprKind::Ident("arr".to_string()),
                        FluxType::List(Box::new(FluxType::Int)),
                    )),
                    index: Box::new(typed_expr(TypedExprKind::IntLiteral(0), FluxType::Int)),
                },
                FluxType::Int,
            ),
            value: typed_expr(TypedExprKind::IntLiteral(99), FluxType::Int),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "arr[0] = 99;\n");
    }

    #[test]
    fn emit_if_stmt_simple() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let if_stmt = TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 20),
        };
        emitter.emit_if_stmt(&if_stmt).unwrap();
        assert_eq!(emitter.output, "if true {\n    1;\n}\n");
    }

    #[test]
    fn emit_if_stmt_with_else() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let if_stmt = TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            elif_branches: vec![],
            else_body: Some(vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                span: Span::new(0, 1),
            })]),
            span: Span::new(0, 30),
        };
        emitter.emit_if_stmt(&if_stmt).unwrap();
        assert_eq!(
            emitter.output,
            "if true {\n    1;\n} else {\n    2;\n}\n"
        );
    }

    #[test]
    fn emit_if_stmt_with_elif() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let if_stmt = TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            elif_branches: vec![TypedElifBranch {
                condition: typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
                body: vec![TypedStmt::Expr(TypedExprStmt {
                    expr: typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                    span: Span::new(0, 1),
                })],
                span: Span::new(0, 10),
            }],
            else_body: Some(vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
                span: Span::new(0, 1),
            })]),
            span: Span::new(0, 40),
        };
        emitter.emit_if_stmt(&if_stmt).unwrap();
        assert_eq!(
            emitter.output,
            "if true {\n    1;\n} else if false {\n    2;\n} else {\n    3;\n}\n"
        );
    }

    #[test]
    fn emit_for_loop_basic() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("items".to_string());
        let for_loop = TypedForLoop {
            variable: "item".to_string(),
            variable_type: FluxType::Int,
            iterable: typed_expr(
                TypedExprKind::Ident("items".to_string()),
                FluxType::List(Box::new(FluxType::Int)),
            ),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::Ident("item".to_string()), FluxType::Int),
                span: Span::new(0, 1),
            })],
            span: Span::new(0, 30),
        };
        emitter.emit_for_loop(&for_loop).unwrap();
        assert_eq!(emitter.output, "for item in items {\n    item;\n}\n");
        assert!(emitter.local_vars.contains("item"));
    }

    #[test]
    fn emit_while_loop_basic() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let while_loop = TypedWhileLoop {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            span: Span::new(0, 20),
        };
        emitter.emit_while_loop(&while_loop).unwrap();
        assert_eq!(emitter.output, "while true {\n    1;\n}\n");
    }

    #[test]
    fn emit_return_with_value() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let ret = TypedReturnStmt {
            value: Some(typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int)),
            span: Span::new(0, 10),
        };
        emitter.emit_return(&ret).unwrap();
        assert_eq!(emitter.output, "return 42;\n");
    }

    #[test]
    fn emit_return_without_value() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let ret = TypedReturnStmt {
            value: None,
            span: Span::new(0, 7),
        };
        emitter.emit_return(&ret).unwrap();
        assert_eq!(emitter.output, "return;\n");
    }

    #[test]
    fn emit_expr_stmt_non_signal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr_stmt = TypedExprStmt {
            expr: typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int),
            span: Span::new(0, 2),
        };
        emitter.emit_expr_stmt(&expr_stmt).unwrap();
        assert_eq!(emitter.output, "42;\n");
    }

    #[test]
    fn emit_expr_stmt_signal_wrapped() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr_stmt = TypedExprStmt {
            expr: typed_expr(
                TypedExprKind::FunctionCall {
                    function: Box::new(typed_expr(
                        TypedExprKind::Ident("OPEN".to_string()),
                        FluxType::Fn {
                            params: crate::typeck::types::FnParams::Fixed(vec![
                                FluxType::String,
                                FluxType::Int,
                            ]),
                            ret: Box::new(FluxType::Signal),
                        },
                    )),
                    args: vec![
                        typed_expr(
                            TypedExprKind::Ident("symbol".to_string()),
                            FluxType::String,
                        ),
                        typed_expr(TypedExprKind::IntLiteral(100), FluxType::Int),
                    ],
                },
                FluxType::Signal,
            ),
            span: Span::new(0, 20),
        };
        emitter.emit_expr_stmt(&expr_stmt).unwrap();
        assert_eq!(emitter.output, "signals.push(Signal::open(ctx.symbol, 100));\n");
    }

    #[test]
    fn emit_assignment_with_indentation() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.indent_level = 2;
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
            span: Span::new(0, 5),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "        let mut x = 1;\n");
    }

    #[test]
    fn emit_nested_if_in_for_indentation() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("items".to_string());
        let for_loop = TypedForLoop {
            variable: "item".to_string(),
            variable_type: FluxType::Int,
            iterable: typed_expr(
                TypedExprKind::Ident("items".to_string()),
                FluxType::List(Box::new(FluxType::Int)),
            ),
            body: vec![TypedStmt::If(TypedIfStmt {
                condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
                body: vec![TypedStmt::Expr(TypedExprStmt {
                    expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                    span: Span::new(0, 1),
                })],
                elif_branches: vec![],
                else_body: None,
                span: Span::new(0, 20),
            })],
            span: Span::new(0, 40),
        };
        emitter.emit_for_loop(&for_loop).unwrap();
        let expected = "for item in items {\n    if true {\n        1;\n    }\n}\n";
        assert_eq!(emitter.output, expected);
    }

    // ===== Top-level emission tests (task 4.2) =====

    #[test]
    fn emit_preamble_content() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();
        assert!(
            output.starts_with("use flux_runtime::*;\n\n"),
            "Output must start with preamble, got: {:?}",
            &output[..output.len().min(40)]
        );
    }

    #[test]
    fn emit_full_with_params_and_state() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Struct fields
        assert!(
            output.contains("pub period: i64,"),
            "Struct should have pub param field"
        );
        assert!(
            output.contains("    count: i64,"),
            "Struct should have non-pub state field"
        );
        // state fields should NOT have pub
        assert!(
            !output.contains("pub count:"),
            "State field should not be pub"
        );

        // Default impl values
        assert!(
            output.contains("period: 20,"),
            "Default impl should have period: 20"
        );
        assert!(
            output.contains("count: 0,"),
            "Default impl should have count: 0"
        );

        // Strategy impl
        assert!(
            output.contains("impl Strategy for MyStrategy {"),
            "Should have Strategy impl"
        );
    }

    #[test]
    fn emit_empty_strategy_struct() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Empty struct
        assert!(
            output.contains("pub struct Test {\n}\n"),
            "Empty struct should have empty braces, got: {:?}",
            output
        );
    }

    #[test]
    fn emit_empty_strategy_default_impl() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("impl Default for Test {"),
            "Should have Default impl"
        );
        assert!(
            output.contains("Self {\n        }\n"),
            "Empty Default should have empty Self block"
        );
    }

    #[test]
    fn emit_empty_strategy_impl_block() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("impl Strategy for Test {\n}\n"),
            "Empty Strategy impl should have no methods"
        );
    }

    #[test]
    fn emit_event_handler_with_signal() {
        // Build a program with an on_bar handler containing OPEN(symbol, 100)
        let prog = TypedProgram {
            imports: vec![],
            strategy: TypedStrategy {
                name: "Sig".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![TypedStmt::Expr(TypedExprStmt {
                        expr: TypedExpr {
                            kind: TypedExprKind::FunctionCall {
                                function: Box::new(typed_expr(
                                    TypedExprKind::Ident("OPEN".to_string()),
                                    FluxType::Fn {
                                        params: crate::typeck::types::FnParams::Fixed(vec![
                                            FluxType::String,
                                            FluxType::Int,
                                        ]),
                                        ret: Box::new(FluxType::Signal),
                                    },
                                )),
                                args: vec![
                                    typed_expr(
                                        TypedExprKind::Ident("symbol".to_string()),
                                        FluxType::String,
                                    ),
                                    typed_expr(TypedExprKind::IntLiteral(100), FluxType::Int),
                                ],
                            },
                            resolved_type: FluxType::Signal,
                            span: Span::new(0, 20),
                        },
                        span: Span::new(0, 20),
                    })],
                    span: Span::new(0, 50),
                })],
                span: Span::new(0, 60),
            },
            span: Span::new(0, 60),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Signal declaration
        assert!(
            output.contains("let mut signals: Vec<Signal> = Vec::new();"),
            "Handler should declare signals vector"
        );
        // Signal push
        assert!(
            output.contains("signals.push(Signal::open(ctx.symbol, 100));"),
            "Handler should push signal"
        );
        // Return signals
        assert!(
            output.contains("        signals\n"),
            "Handler should return signals"
        );
    }

    #[test]
    fn emit_event_handler_empty_body() {
        // Handler with no statements should only have signal decl and return
        let prog = TypedProgram {
            imports: vec![],
            strategy: TypedStrategy {
                name: "Empty".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![],
                    span: Span::new(0, 30),
                })],
                span: Span::new(0, 40),
            },
            span: Span::new(0, 40),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Should contain the method with just signal decl and return
        let expected_handler = concat!(
            "    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {\n",
            "        let mut signals: Vec<Signal> = Vec::new();\n",
            "        signals\n",
            "    }\n",
        );
        assert!(
            output.contains(expected_handler),
            "Empty handler should only have signal decl and return, got:\n{}",
            output
        );
    }

    #[test]
    fn emit_properties_before_params_in_struct() {
        // Build program with both properties and params, verify ordering
        let prog = TypedProgram {
            imports: vec![],
            strategy: TypedStrategy {
                name: "Order".to_string(),
                body: vec![
                    TypedStrategyItem::Property(TypedProperty {
                        name: "version".to_string(),
                        value: TypedExpr {
                            kind: TypedExprKind::IntLiteral(1),
                            resolved_type: FluxType::Int,
                            span: Span::new(10, 11),
                        },
                        span: Span::new(5, 15),
                    }),
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "period".to_string(),
                            default_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(20),
                                resolved_type: FluxType::Int,
                                span: Span::new(30, 32),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(25, 35),
                        }],
                        span: Span::new(20, 40),
                    }),
                ],
                span: Span::new(0, 50),
            },
            span: Span::new(0, 50),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Properties appear before params in struct
        let version_pos = output.find("pub version: i64,").unwrap();
        let period_pos = output.find("pub period: i64,").unwrap();
        assert!(
            version_pos < period_pos,
            "Properties should appear before params in struct"
        );
    }

    #[test]
    fn emit_blank_lines_between_sections() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // There should be a blank line between struct and Default impl
        assert!(
            output.contains("}\n\nimpl Default for"),
            "Should have blank line between struct and Default impl"
        );
        // There should be a blank line between Default impl and Strategy impl
        assert!(
            output.contains("}\n\nimpl Strategy for"),
            "Should have blank line between Default impl and Strategy impl"
        );
    }

    #[test]
    fn emit_four_space_indentation() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Struct fields use 4-space indent
        assert!(
            output.contains("    pub period: i64,"),
            "Struct fields should use 4-space indentation"
        );
        // Default impl fn uses 4-space indent
        assert!(
            output.contains("    fn default() -> Self {"),
            "Default fn should use 4-space indentation"
        );
        // Inner Self block uses 8-space indent
        assert!(
            output.contains("        Self {"),
            "Self block should use 8-space (2-level) indentation"
        );
        // Field defaults use 12-space indent
        assert!(
            output.contains("            period: 20,"),
            "Default field values should use 12-space (3-level) indentation"
        );
    }

    #[test]
    fn emit_knr_braces() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // K&R style: opening brace on same line
        assert!(
            output.contains("pub struct MyStrategy {\n"),
            "Struct opening brace should be K&R style"
        );
        assert!(
            output.contains("impl Default for MyStrategy {\n"),
            "Default impl opening brace should be K&R style"
        );
        assert!(
            output.contains("impl Strategy for MyStrategy {\n"),
            "Strategy impl opening brace should be K&R style"
        );
    }

    #[test]
    fn emit_full_end_to_end() {
        // Build a realistic TypedProgram similar to the design doc example
        let prog = TypedProgram {
            imports: vec![Import {
                module_path: "indicators".to_string(),
                names: vec!["sma".to_string()],
                span: Span::new(0, 30),
            }],
            strategy: TypedStrategy {
                name: "MomentumStrategy".to_string(),
                body: vec![
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![
                            TypedParam {
                                name: "period".to_string(),
                                default_value: TypedExpr {
                                    kind: TypedExprKind::IntLiteral(20),
                                    resolved_type: FluxType::Int,
                                    span: Span::new(50, 52),
                                },
                                resolved_type: FluxType::Int,
                                span: Span::new(40, 55),
                            },
                            TypedParam {
                                name: "threshold".to_string(),
                                default_value: TypedExpr {
                                    kind: TypedExprKind::FloatLiteral(2.0),
                                    resolved_type: FluxType::Float,
                                    span: Span::new(65, 68),
                                },
                                resolved_type: FluxType::Float,
                                span: Span::new(58, 70),
                            },
                        ],
                        span: Span::new(35, 75),
                    }),
                    TypedStrategyItem::StateBlock(TypedStateBlock {
                        variables: vec![TypedStateVar {
                            name: "count".to_string(),
                            initial_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(0),
                                resolved_type: FluxType::Int,
                                span: Span::new(90, 91),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(80, 95),
                        }],
                        span: Span::new(78, 100),
                    }),
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![
                            // count = count + 1
                            TypedStmt::Assignment(TypedAssignment {
                                target: typed_expr(
                                    TypedExprKind::Ident("count".to_string()),
                                    FluxType::Int,
                                ),
                                value: typed_expr(
                                    TypedExprKind::BinaryOp {
                                        left: Box::new(typed_expr(
                                            TypedExprKind::Ident("count".to_string()),
                                            FluxType::Int,
                                        )),
                                        op: BinOp::Add,
                                        right: Box::new(typed_expr(
                                            TypedExprKind::IntLiteral(1),
                                            FluxType::Int,
                                        )),
                                    },
                                    FluxType::Int,
                                ),
                                span: Span::new(110, 125),
                            }),
                            // if close > sma(close, period) { OPEN(symbol, 100) }
                            TypedStmt::If(TypedIfStmt {
                                condition: typed_expr(
                                    TypedExprKind::BinaryOp {
                                        left: Box::new(typed_expr(
                                            TypedExprKind::Ident("close".to_string()),
                                            FluxType::Float,
                                        )),
                                        op: BinOp::Gt,
                                        right: Box::new(typed_expr(
                                            TypedExprKind::FunctionCall {
                                                function: Box::new(typed_expr(
                                                    TypedExprKind::Ident("sma".to_string()),
                                                    FluxType::Fn {
                                                        params:
                                                            crate::typeck::types::FnParams::VariadicNumeric,
                                                        ret: Box::new(FluxType::Float),
                                                    },
                                                )),
                                                args: vec![
                                                    typed_expr(
                                                        TypedExprKind::Ident("close".to_string()),
                                                        FluxType::Float,
                                                    ),
                                                    typed_expr(
                                                        TypedExprKind::Ident("period".to_string()),
                                                        FluxType::Int,
                                                    ),
                                                ],
                                            },
                                            FluxType::Float,
                                        )),
                                    },
                                    FluxType::Bool,
                                ),
                                body: vec![TypedStmt::Expr(TypedExprStmt {
                                    expr: TypedExpr {
                                        kind: TypedExprKind::FunctionCall {
                                            function: Box::new(typed_expr(
                                                TypedExprKind::Ident("OPEN".to_string()),
                                                FluxType::Fn {
                                                    params:
                                                        crate::typeck::types::FnParams::Fixed(vec![
                                                            FluxType::String,
                                                            FluxType::Int,
                                                        ]),
                                                    ret: Box::new(FluxType::Signal),
                                                },
                                            )),
                                            args: vec![
                                                typed_expr(
                                                    TypedExprKind::Ident("symbol".to_string()),
                                                    FluxType::String,
                                                ),
                                                typed_expr(
                                                    TypedExprKind::IntLiteral(100),
                                                    FluxType::Int,
                                                ),
                                            ],
                                        },
                                        resolved_type: FluxType::Signal,
                                        span: Span::new(140, 160),
                                    },
                                    span: Span::new(140, 160),
                                })],
                                elif_branches: vec![],
                                else_body: None,
                                span: Span::new(130, 170),
                            }),
                        ],
                        span: Span::new(105, 175),
                    }),
                ],
                span: Span::new(32, 180),
            },
            span: Span::new(0, 180),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Verify complete output structure
        let expected = concat!(
            "use flux_runtime::*;\n",
            "\n",
            "pub struct MomentumStrategy {\n",
            "    pub period: i64,\n",
            "    pub threshold: f64,\n",
            "    count: i64,\n",
            "}\n",
            "\n",
            "impl Default for MomentumStrategy {\n",
            "    fn default() -> Self {\n",
            "        Self {\n",
            "            period: 20,\n",
            "            threshold: 2.0,\n",
            "            count: 0,\n",
            "        }\n",
            "    }\n",
            "}\n",
            "\n",
            "impl Strategy for MomentumStrategy {\n",
            "    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {\n",
            "        let mut signals: Vec<Signal> = Vec::new();\n",
            "        self.count = (self.count + 1);\n",
            "        if (ctx.close > sma(ctx.close, self.period)) {\n",
            "            signals.push(Signal::open(ctx.symbol, 100));\n",
            "        }\n",
            "        signals\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(output, expected);
    }
}
