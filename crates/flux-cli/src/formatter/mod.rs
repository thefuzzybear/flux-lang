//! Formatter module — formats Flux AST back into consistently-styled source text.
//!
//! The formatter operates in a parse-then-pretty-print model:
//! parse the source into an AST, walk the AST to emit formatted text,
//! and optionally apply ANSI colorization for terminal display.

use flux_compiler::parser::ast::{
    Assignment, BinOp, DataBlock, DecoratorArg, EventHandler, Expr, ExprKind, ExprStmt, ForLoop,
    IfStmt, Import, MatchExpr, Param, ParamsBlock, Pattern, Program, Property, ReturnStmt, StateBlock, StateVar,
    Strategy, StrategyItem, Stmt, StructDef, TypeAnnotation, UnaryOp, WhileLoop,
};
use flux_compiler::{extract_comments, Comment};

/// Configuration for the formatter.
pub struct FormatterConfig {
    /// Number of spaces per indentation level (always 4).
    pub indent_width: usize,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self { indent_width: 4 }
    }
}

/// The formatter engine that walks an AST and emits formatted source text.
pub struct Formatter {
    config: FormatterConfig,
    output: String,
    indent_level: usize,
    comments: Vec<Comment>,
    comment_index: usize,
}

impl Formatter {
    /// Create a new formatter with the given configuration.
    pub fn new(config: FormatterConfig) -> Self {
        Self {
            config,
            output: String::new(),
            indent_level: 0,
            comments: Vec::new(),
            comment_index: 0,
        }
    }

    /// Format a parsed AST back into source text.
    ///
    /// This is the main entry point. It extracts comments from the original source,
    /// walks the AST to produce formatted output, and post-processes to clean up
    /// blank lines and trailing whitespace.
    pub fn format(program: &Program, source: &str) -> String {
        let comments = extract_comments(source);
        let mut formatter = Formatter {
            config: FormatterConfig::default(),
            output: String::new(),
            indent_level: 0,
            comments,
            comment_index: 0,
        };

        formatter.format_program(program);
        formatter.emit_remaining_comments();
        post_process(&formatter.output)
    }

    // --- Indentation helpers ---

    fn indent(&self) -> String {
        " ".repeat(self.config.indent_width * self.indent_level)
    }

    fn push_indent(&mut self) {
        self.output.push_str(&self.indent());
    }

    // --- Comment helpers ---

    /// Emit any above-line comments whose byte offset is before `node_start`.
    fn emit_comments_before(&mut self, node_start: usize) {
        while self.comment_index < self.comments.len() {
            let start = self.comments[self.comment_index].start;
            let is_trailing = self.comments[self.comment_index].is_trailing;
            if start < node_start && !is_trailing {
                let text = self.comments[self.comment_index].text.clone();
                self.push_indent();
                self.output.push_str(&text);
                self.output.push('\n');
                self.comment_index += 1;
            } else {
                break;
            }
        }
    }

    /// Check if the next unconsumed comment is a trailing comment on the same
    /// source line as the code we just emitted. If so, append it.
    fn emit_trailing_comment(&mut self, node_end: usize) {
        if self.comment_index < self.comments.len() {
            let is_trailing = self.comments[self.comment_index].is_trailing;
            let start = self.comments[self.comment_index].start;
            if is_trailing && start >= node_end {
                let text = self.comments[self.comment_index].text.clone();
                self.output.push(' ');
                self.output.push_str(&text);
                self.comment_index += 1;
            }
        }
    }

    /// Emit all remaining comments at end of file.
    fn emit_remaining_comments(&mut self) {
        while self.comment_index < self.comments.len() {
            let text = self.comments[self.comment_index].text.clone();
            self.push_indent();
            self.output.push_str(&text);
            self.output.push('\n');
            self.comment_index += 1;
        }
    }

    // --- AST walking ---

    fn format_program(&mut self, program: &Program) {
        // Format imports
        for import in &program.imports {
            self.emit_comments_before(import.span.start);
            self.format_import(import);
        }

        // Blank line between imports and data block or strategy (if there were imports)
        if !program.imports.is_empty() {
            self.output.push('\n');
        }

        // Format data block (if present)
        if let Some(ref data_block) = program.data_block {
            self.emit_comments_before(data_block.span.start);
            self.format_data_block(data_block);
            // Blank line between data block and next section
            self.output.push('\n');
        }

        // Format struct definitions
        for (i, struct_def) in program.structs.iter().enumerate() {
            self.emit_comments_before(struct_def.span.start);
            self.format_struct_def(struct_def);
            // Blank line between struct definitions
            if i < program.structs.len() - 1 {
                self.output.push('\n');
            }
        }

        // Blank line between structs and strategy (if there were structs)
        if !program.structs.is_empty() {
            self.output.push('\n');
        }

        // Format strategy
        self.emit_comments_before(program.strategy.span.start);
        self.format_strategy(&program.strategy);
    }

    fn format_import(&mut self, import: &Import) {
        self.push_indent();
        self.output.push_str("from ");
        self.output.push_str(&import.module_path);
        self.output.push_str(" import {");
        for (i, name) in import.names.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            self.output.push_str(name);
        }
        self.output.push('}');
        self.emit_trailing_comment(import.span.end);
        self.output.push('\n');
    }

    fn format_struct_def(&mut self, struct_def: &StructDef) {
        // Emit decorators, one per line above the struct keyword
        for decorator in &struct_def.decorators {
            self.push_indent();
            self.output.push('@');
            self.output.push_str(&decorator.name);
            if let Some(ref arg) = decorator.arg {
                match arg {
                    DecoratorArg::Int(n) => {
                        self.output.push('(');
                        self.output.push_str(&n.to_string());
                        self.output.push(')');
                    }
                }
            }
            self.output.push('\n');
        }

        // Emit struct keyword and name
        self.push_indent();
        self.output.push_str("struct ");
        self.output.push_str(&struct_def.name);
        self.output.push_str(" {\n");

        self.indent_level += 1;

        // Emit fields, one per line
        for (i, field) in struct_def.fields.iter().enumerate() {
            // Emit field-level decorators (e.g. @hot, @cold)
            for dec in &field.field_decorators {
                self.push_indent();
                self.output.push('@');
                self.output.push_str(&dec.name);
                if let Some(ref arg) = dec.arg {
                    match arg {
                        DecoratorArg::Int(n) => {
                            self.output.push('(');
                            self.output.push_str(&n.to_string());
                            self.output.push(')');
                        }
                    }
                }
                self.output.push('\n');
            }

            self.push_indent();
            self.output.push_str(&field.name);
            self.output.push_str(": ");
            self.format_type_annotation(&field.field_type);
            // Comma after each field except the last
            if i < struct_def.fields.len() - 1 {
                self.output.push(',');
            }
            self.output.push('\n');
        }

        self.indent_level -= 1;
        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_type_annotation(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::F64 => self.output.push_str("f64"),
            TypeAnnotation::Int => self.output.push_str("int"),
            TypeAnnotation::Bool => self.output.push_str("bool"),
            TypeAnnotation::Str => self.output.push_str("str"),
            TypeAnnotation::Named(name) => self.output.push_str(name),
            TypeAnnotation::FixedArray(elem_type, size) => {
                self.output.push('[');
                self.format_type_annotation(elem_type);
                self.output.push_str("; ");
                self.output.push_str(&size.to_string());
                self.output.push(']');
            }
            TypeAnnotation::BitInt(n) => {
                self.output.push_str("int(");
                self.output.push_str(&n.to_string());
                self.output.push(')');
            }
            TypeAnnotation::Generic(name, type_args) => {
                self.output.push_str(name);
                self.output.push('[');
                for (i, arg) in type_args.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.format_type_annotation(arg);
                }
                self.output.push(']');
            }
        }
    }

    fn format_data_block(&mut self, block: &DataBlock) {
        self.push_indent();
        self.output.push_str("data {\n");
        self.indent_level += 1;

        if let Some(ref symbols) = block.symbols {
            self.push_indent();
            let list = symbols
                .value
                .iter()
                .map(|s| format!("\"{}\"", s))
                .collect::<Vec<_>>()
                .join(", ");
            self.output.push_str(&format!("symbols = [{}]\n", list));
        }
        if let Some(ref period) = block.period {
            self.push_indent();
            self.output.push_str(&format!("period = \"{}\"\n", period.value));
        }
        if let Some(ref interval) = block.interval {
            self.push_indent();
            self.output.push_str(&format!("interval = \"{}\"\n", interval.value));
        }
        if let Some(ref source) = block.source {
            self.push_indent();
            self.output.push_str(&format!("source = \"{}\"\n", source.value));
        }

        self.indent_level -= 1;
        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_strategy(&mut self, strategy: &Strategy) {
        self.push_indent();
        self.output.push_str("strategy ");
        self.output.push_str(&strategy.name);
        self.output.push_str(" {\n");

        self.indent_level += 1;

        let mut first_item = true;
        for item in &strategy.body {
            // Single blank line between top-level blocks
            if !first_item {
                self.output.push('\n');
            }
            first_item = false;

            self.format_strategy_item(item);
        }

        self.indent_level -= 1;
        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_strategy_item(&mut self, item: &StrategyItem) {
        match item {
            StrategyItem::Property(prop) => {
                self.emit_comments_before(prop.span.start);
                self.format_property(prop);
            }
            StrategyItem::ParamsBlock(block) => {
                self.emit_comments_before(block.span.start);
                self.format_params_block(block);
            }
            StrategyItem::StateBlock(block) => {
                self.emit_comments_before(block.span.start);
                self.format_state_block(block);
            }
            StrategyItem::EventHandler(handler) => {
                self.emit_comments_before(handler.span.start);
                self.format_event_handler(handler);
            }
        }
    }

    fn format_property(&mut self, prop: &Property) {
        self.push_indent();
        self.output.push_str(&prop.name);
        self.output.push_str(" = ");
        self.format_expr(&prop.value);
        self.emit_trailing_comment(prop.span.end);
        self.output.push('\n');
    }

    fn format_params_block(&mut self, block: &ParamsBlock) {
        self.push_indent();
        self.output.push_str("params {\n");
        self.indent_level += 1;

        for param in &block.params {
            self.emit_comments_before(param.span.start);
            self.format_param(param);
        }

        self.indent_level -= 1;
        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_param(&mut self, param: &Param) {
        self.push_indent();
        self.output.push_str(&param.name);
        self.output.push_str(" = ");
        self.format_expr(&param.default_value);
        self.emit_trailing_comment(param.span.end);
        self.output.push('\n');
    }

    fn format_state_block(&mut self, block: &StateBlock) {
        self.push_indent();
        self.output.push_str("state {\n");
        self.indent_level += 1;

        for var in &block.variables {
            self.emit_comments_before(var.span.start);
            self.format_state_var(var);
        }

        self.indent_level -= 1;
        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_state_var(&mut self, var: &StateVar) {
        self.push_indent();
        self.output.push_str(&var.name);
        self.output.push_str(" = ");
        self.format_expr(&var.initial_value);
        self.emit_trailing_comment(var.span.end);
        self.output.push('\n');
    }

    fn format_event_handler(&mut self, handler: &EventHandler) {
        self.push_indent();
        self.output.push_str("on ");
        self.output.push_str(&handler.event_name);
        self.output.push_str(" {\n");
        self.indent_level += 1;

        self.format_stmts(&handler.body);

        self.indent_level -= 1;
        self.push_indent();
        self.output.push_str("}\n");
    }

    // --- Statements ---

    fn format_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.emit_comments_before(stmt_span_start(stmt));
            self.format_stmt(stmt);
        }
    }

    fn format_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assignment(a) => self.format_assignment(a),
            Stmt::If(if_stmt) => self.format_if(if_stmt),
            Stmt::For(for_loop) => self.format_for(for_loop),
            Stmt::While(while_loop) => self.format_while(while_loop),
            Stmt::Return(ret) => self.format_return(ret),
            Stmt::Expr(expr_stmt) => self.format_expr_stmt(expr_stmt),
        }
    }

    fn format_assignment(&mut self, assign: &Assignment) {
        self.push_indent();
        self.format_expr(&assign.target);
        self.output.push_str(" = ");
        self.format_expr(&assign.value);
        self.emit_trailing_comment(assign.span.end);
        self.output.push('\n');
    }

    fn format_if(&mut self, if_stmt: &IfStmt) {
        self.push_indent();
        self.output.push_str("if ");
        self.format_expr(&if_stmt.condition);
        self.output.push_str(" {\n");

        self.indent_level += 1;
        self.format_stmts(&if_stmt.body);
        self.indent_level -= 1;

        // elif branches
        for elif in &if_stmt.elif_branches {
            self.push_indent();
            self.output.push_str("} elif ");
            self.format_expr(&elif.condition);
            self.output.push_str(" {\n");

            self.indent_level += 1;
            self.format_stmts(&elif.body);
            self.indent_level -= 1;
        }

        // else
        if let Some(else_body) = &if_stmt.else_body {
            self.push_indent();
            self.output.push_str("} else {\n");

            self.indent_level += 1;
            self.format_stmts(else_body);
            self.indent_level -= 1;
        }

        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_for(&mut self, for_loop: &ForLoop) {
        self.push_indent();
        self.output.push_str("for ");
        self.output.push_str(&for_loop.variable);
        self.output.push_str(" in ");
        self.format_expr(&for_loop.iterable);
        self.output.push_str(" {\n");

        self.indent_level += 1;
        self.format_stmts(&for_loop.body);
        self.indent_level -= 1;

        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_while(&mut self, while_loop: &WhileLoop) {
        self.push_indent();
        self.output.push_str("while ");
        self.format_expr(&while_loop.condition);
        self.output.push_str(" {\n");

        self.indent_level += 1;
        self.format_stmts(&while_loop.body);
        self.indent_level -= 1;

        self.push_indent();
        self.output.push_str("}\n");
    }

    fn format_return(&mut self, ret: &ReturnStmt) {
        self.push_indent();
        self.output.push_str("return");
        if let Some(value) = &ret.value {
            self.output.push(' ');
            self.format_expr(value);
        }
        self.emit_trailing_comment(ret.span.end);
        self.output.push('\n');
    }

    fn format_expr_stmt(&mut self, expr_stmt: &ExprStmt) {
        self.push_indent();
        self.format_expr(&expr_stmt.expr);
        self.emit_trailing_comment(expr_stmt.span.end);
        self.output.push('\n');
    }

    // --- Expressions ---

    fn format_expr(&mut self, expr: &Expr) {
        self.format_expr_with_prec(expr, 0, false);
    }

    /// Format an expression, adding parentheses if its precedence is lower than
    /// the parent context requires.
    fn format_expr_with_prec(&mut self, expr: &Expr, parent_prec: u8, is_right: bool) {
        let my_prec = expr_precedence(&expr.kind);
        let needs_parens = if my_prec == 0 {
            false // atoms never need parens
        } else if is_right {
            my_prec <= parent_prec // right operand: parens at same or lower
        } else {
            my_prec < parent_prec // left operand: parens only at lower
        };

        if needs_parens {
            self.output.push('(');
        }
        self.format_expr_inner(expr);
        if needs_parens {
            self.output.push(')');
        }
    }

    /// Format the inner expression content (no outer parens decision).
    fn format_expr_inner(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::IntLiteral(val) => {
                self.output.push_str(&val.to_string());
            }
            ExprKind::FloatLiteral(val) => {
                let s = val.to_string();
                if s.contains('.') {
                    self.output.push_str(&s);
                } else {
                    // Ensure float formatting always has a decimal point
                    self.output.push_str(&s);
                    self.output.push_str(".0");
                }
            }
            ExprKind::StringLiteral(s) => {
                self.output.push('"');
                // Escape special characters
                for ch in s.chars() {
                    match ch {
                        '\\' => self.output.push_str("\\\\"),
                        '"' => self.output.push_str("\\\""),
                        '\n' => self.output.push_str("\\n"),
                        '\t' => self.output.push_str("\\t"),
                        other => self.output.push(other),
                    }
                }
                self.output.push('"');
            }
            ExprKind::BoolLiteral(val) => {
                self.output.push_str(if *val { "true" } else { "false" });
            }
            ExprKind::NullLiteral => {
                self.output.push_str("null");
            }
            ExprKind::ListLiteral(elements) => {
                self.output.push('[');
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.format_expr(elem);
                }
                self.output.push(']');
            }
            ExprKind::Ident(name) => {
                self.output.push_str(name);
            }
            ExprKind::BinaryOp { left, op, right } => {
                let prec = binop_prec(*op);
                self.format_expr_with_prec(left, prec, false);
                self.output.push(' ');
                self.output.push_str(binop_str(*op));
                self.output.push(' ');
                self.format_expr_with_prec(right, prec, true);
            }
            ExprKind::UnaryOp { op, operand } => {
                match op {
                    UnaryOp::Neg => {
                        self.output.push('-');
                        self.format_expr_with_prec(operand, 7, false);
                    }
                    UnaryOp::Not => {
                        self.output.push_str("not ");
                        self.format_expr_with_prec(operand, 7, false);
                    }
                }
            }
            ExprKind::FunctionCall { function, args } => {
                self.format_expr_with_prec(function, 0, false);
                self.output.push('(');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.format_expr(arg);
                }
                self.output.push(')');
            }
            ExprKind::MethodCall { receiver, method, args } => {
                self.format_expr_with_prec(receiver, 0, false);
                self.output.push('.');
                self.output.push_str(method);
                self.output.push('(');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.format_expr(arg);
                }
                self.output.push(')');
            }
            ExprKind::MemberAccess { object, field } => {
                self.format_expr_with_prec(object, 0, false);
                self.output.push('.');
                self.output.push_str(field);
            }
            ExprKind::IndexAccess { object, index } => {
                self.format_expr_with_prec(object, 0, false);
                self.output.push('[');
                self.format_expr(index);
                self.output.push(']');
            }
            ExprKind::StructLiteral { struct_name, fields } => {
                self.format_struct_literal(struct_name, fields);
            }
            ExprKind::EnumConstruction { enum_name, variant_name, args } => {
                self.output.push_str(enum_name);
                self.output.push('.');
                self.output.push_str(variant_name);
                if !args.is_empty() {
                    self.output.push('(');
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.output.push_str(", ");
                        }
                        self.format_expr(arg);
                    }
                    self.output.push(')');
                }
            }
            ExprKind::Match(match_expr) => {
                self.format_match_expr(match_expr);
            }
        }
    }

    /// Format a match expression.
    fn format_match_expr(&mut self, match_expr: &MatchExpr) {
        self.output.push_str("match ");
        self.format_expr(&match_expr.scrutinee);
        self.output.push_str(" {\n");
        self.indent_level += 1;
        for arm in &match_expr.arms {
            self.push_indent();
            self.format_pattern(&arm.pattern);
            self.output.push_str(" => ");
            if arm.body.len() == 1 {
                // Single statement: put on same line
                self.format_stmt(&arm.body[0]);
            } else {
                // Multiple statements: use block
                self.output.push_str("{\n");
                self.indent_level += 1;
                for stmt in &arm.body {
                    self.push_indent();
                    self.format_stmt(stmt);
                    self.output.push('\n');
                }
                self.indent_level -= 1;
                self.push_indent();
                self.output.push_str("}");
            }
            self.output.push('\n');
        }
        self.indent_level -= 1;
        self.push_indent();
        self.output.push('}');
    }

    /// Format a pattern in a match arm.
    fn format_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Variant { enum_name, variant_name, bindings, .. } => {
                self.output.push_str(enum_name);
                self.output.push('.');
                self.output.push_str(variant_name);
                if !bindings.is_empty() {
                    self.output.push('(');
                    for (i, binding) in bindings.iter().enumerate() {
                        if i > 0 {
                            self.output.push_str(", ");
                        }
                        self.output.push_str(binding);
                    }
                    self.output.push(')');
                }
            }
            Pattern::Wildcard { .. } => {
                self.output.push('_');
            }
        }
    }

    /// Format a struct literal expression.
    /// Uses single-line format for ≤3 fields and multi-line format for >3 fields.
    fn format_struct_literal(&mut self, struct_name: &str, fields: &[(String, Expr)]) {
        self.output.push_str(struct_name);
        if fields.len() <= 3 {
            // Single-line: Point { x = 1.0, y = 2.0 }
            self.output.push_str(" { ");
            for (i, (name, value)) in fields.iter().enumerate() {
                if i > 0 {
                    self.output.push_str(", ");
                }
                self.output.push_str(name);
                self.output.push_str(" = ");
                self.format_expr(value);
            }
            self.output.push_str(" }");
        } else {
            // Multi-line: one field per line with increased indentation
            self.output.push_str(" {\n");
            self.indent_level += 1;
            for (i, (name, value)) in fields.iter().enumerate() {
                self.push_indent();
                self.output.push_str(name);
                self.output.push_str(" = ");
                self.format_expr(value);
                if i < fields.len() - 1 {
                    self.output.push(',');
                }
                self.output.push('\n');
            }
            self.indent_level -= 1;
            self.push_indent();
            self.output.push('}');
        }
    }
}

// --- Helpers ---

/// Get the precedence level of an expression kind.
/// 0 = atoms (never need parens), 7 = unary, 1-6 = binary by precedence level.
fn expr_precedence(kind: &ExprKind) -> u8 {
    match kind {
        ExprKind::BinaryOp { op, .. } => binop_prec(*op),
        ExprKind::UnaryOp { .. } => 7,
        _ => 0, // atoms, calls, etc. — never need outer parens
    }
}

/// Get the precedence level of a binary operator.
fn binop_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne => 3,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

/// Get the byte start position of a statement.
fn stmt_span_start(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Assignment(a) => a.span.start,
        Stmt::If(s) => s.span.start,
        Stmt::For(s) => s.span.start,
        Stmt::While(s) => s.span.start,
        Stmt::Return(s) => s.span.start,
        Stmt::Expr(s) => s.span.start,
    }
}

/// Convert a binary operator to its string representation.
fn binop_str(op: BinOp) -> &'static str {
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

/// Post-process the formatted output:
/// 1. Strip trailing whitespace from each line
/// 2. Collapse consecutive blank lines to at most one
/// 3. Ensure exactly one trailing newline
fn post_process(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let mut result_lines: Vec<String> = Vec::new();
    let mut prev_blank = false;

    for line in &lines {
        let trimmed = line.trim_end();
        let is_blank = trimmed.is_empty();

        if is_blank {
            if !prev_blank {
                result_lines.push(String::new());
            }
            prev_blank = true;
        } else {
            result_lines.push(trimmed.to_string());
            prev_blank = false;
        }
    }

    // Remove trailing blank lines before adding exactly one newline
    while result_lines.last().map_or(false, |l| l.is_empty()) {
        result_lines.pop();
    }

    let mut output = result_lines.join("\n");
    output.push('\n');
    output
}

pub mod ansi;

#[cfg(test)]
mod tests {
    use super::*;
    use flux_compiler::lexer;
    use flux_compiler::parser;

    /// Helper: parse source and format it
    fn format_source(source: &str) -> String {
        let tokens = lexer::lex_with_spans(source).unwrap();
        let ast = parser::parse(tokens).unwrap();
        Formatter::format(&ast, source)
    }

    #[test]
    fn format_simple_strategy() {
        let source = r#"strategy Simple {
    params {
        period = 20
        threshold = 2.5
    }

    state {
        count = 0
    }

    on bar {
        if close > open {
            OPEN(symbol, 100.0)
        }
    }
}
"#;
        let result = format_source(source);
        assert_eq!(result, source, "Already-formatted source should be idempotent");
    }

    #[test]
    fn format_idempotent() {
        let source = r#"strategy Test {
    on bar {
        x = 1 + 2
        y = sma(close, 20)
    }
}
"#;
        let first = format_source(source);
        let second = format_source(&first);
        assert_eq!(first, second, "Formatting should be idempotent");
    }

    #[test]
    fn format_binary_ops_spacing() {
        let source = "strategy S {\n    on bar {\n        x = a + b\n        y = c * d\n        z = e == f\n        w = g and h\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("a + b"), "Binary add should have spaces");
        assert!(result.contains("c * d"), "Binary mul should have spaces");
        assert!(result.contains("e == f"), "Binary eq should have spaces");
        assert!(result.contains("g and h"), "Binary and should have spaces");
    }

    #[test]
    fn format_function_call_no_space_before_paren() {
        let source = "strategy S {\n    on bar {\n        x = sma(close, 20)\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("sma(close, 20)"), "No space before ( in function call");
    }

    #[test]
    fn format_if_elif_else() {
        let source = r#"strategy S {
    on bar {
        if x > 0 {
            y = 1
        } elif x == 0 {
            y = 0
        } else {
            y = -1
        }
    }
}
"#;
        let result = format_source(source);
        assert!(result.contains("} elif"), "elif should follow }} on same line");
        assert!(result.contains("} else {"), "else should follow }} on same line");
    }

    #[test]
    fn format_for_loop() {
        let source = "strategy S {\n    on bar {\n        for i in items {\n            x = i\n        }\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("for i in items {"), "For loop syntax");
    }

    #[test]
    fn format_while_loop() {
        let source = "strategy S {\n    on bar {\n        while x > 0 {\n            x = x - 1\n        }\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("while x > 0 {"), "While loop syntax");
    }

    #[test]
    fn format_return_statement() {
        let source = "strategy S {\n    on bar {\n        return 42\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("        return 42\n"), "Return with value");
    }

    #[test]
    fn format_list_literal() {
        let source = "strategy S {\n    state {\n        items = [1, 2, 3]\n    }\n\n    on bar {\n        x = 1\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("[1, 2, 3]"), "List literal formatting");
    }

    #[test]
    fn format_float_preserves_decimal() {
        let source = "strategy S {\n    params {\n        x = 2.0\n    }\n\n    on bar {\n        y = 1\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("2.0"), "Float 2.0 must keep decimal point");
    }

    #[test]
    fn format_string_literal_with_escapes() {
        let source = r#"strategy S {
    on bar {
        x = "hello\nworld"
    }
}
"#;
        let result = format_source(source);
        assert!(result.contains(r#""hello\nworld""#), "String preserves escape sequences");
    }

    #[test]
    fn format_unary_operators() {
        let source = "strategy S {\n    on bar {\n        x = -42\n        y = not z\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("-42"), "Unary neg: no space");
        assert!(result.contains("not z"), "Unary not: space after");
    }

    #[test]
    fn format_method_call() {
        let source = "strategy S {\n    on bar {\n        x = obj.method(1, 2)\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("obj.method(1, 2)"), "Method call with no space around dot");
    }

    #[test]
    fn format_member_access() {
        let source = "strategy S {\n    on bar {\n        x = obj.field\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("obj.field"), "Member access with no space around dot");
    }

    #[test]
    fn format_index_access() {
        let source = "strategy S {\n    on bar {\n        x = arr[0]\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("arr[0]"), "Index access");
    }

    #[test]
    fn format_trailing_whitespace_stripped() {
        // The formatter output should never have trailing whitespace
        let source = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
        let result = format_source(source);
        for line in result.lines() {
            assert_eq!(line, line.trim_end(), "No trailing whitespace on any line");
        }
    }

    #[test]
    fn format_ends_with_single_newline() {
        let source = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
        let result = format_source(source);
        assert!(result.ends_with('\n'), "Should end with newline");
        assert!(!result.ends_with("\n\n"), "Should not end with double newline");
    }

    #[test]
    fn format_collapses_multiple_blank_lines() {
        // This tests post_process directly
        let raw = "line1\n\n\n\nline2\n";
        let result = post_process(raw);
        assert_eq!(result, "line1\n\nline2\n");
    }

    #[test]
    fn format_imports() {
        let source = "from indicators import {sma, ema}\n\nstrategy S {\n    on bar {\n        x = sma(close, 20)\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("from indicators import {sma, ema}"), "Import formatting");
    }

    #[test]
    fn format_above_comment_preserved() {
        let source = "strategy S {\n    on bar {\n        # compute signal\n        x = 1\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("        # compute signal\n        x = 1"), 
            "Above comment should be indented at same level as following code");
    }

    #[test]
    fn format_trailing_comment_preserved() {
        let source = "strategy S {\n    on bar {\n        x = 1 # the value\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("x = 1 # the value"), 
            "Trailing comment should be preserved on same line");
    }

    #[test]
    fn format_top_level_block_separation() {
        let source = "strategy S {\n    params {\n        x = 1\n    }\n\n    state {\n        y = 0\n    }\n\n    on bar {\n        z = 1\n    }\n}\n";
        let result = format_source(source);
        // Check there's exactly one blank line between params and state
        assert!(result.contains("    }\n\n    state"), "One blank line between params and state");
        assert!(result.contains("    }\n\n    on bar"), "One blank line between state and on bar");
    }

    #[test]
    fn format_bool_literals() {
        let source = "strategy S {\n    on bar {\n        x = true\n        y = false\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("x = true"));
        assert!(result.contains("y = false"));
    }

    #[test]
    fn format_null_literal() {
        let source = "strategy S {\n    state {\n        x = null\n    }\n\n    on bar {\n        y = 1\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("x = null"));
    }

    #[test]
    fn format_data_block_all_fields() {
        let source = r#"data {
    symbols = ["AAPL", "MSFT"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy S {
    on bar {
        x = 1
    }
}
"#;
        let result = format_source(source);
        assert_eq!(result, source, "Data block with all fields should be idempotent");
    }

    #[test]
    fn format_data_block_symbols_only() {
        let source = "data {\n    symbols = [\"AAPL\"]\n}\n\nstrategy S {\n    on bar {\n        x = 1\n    }\n}\n";
        let result = format_source(source);
        assert!(result.contains("data {\n    symbols = [\"AAPL\"]\n}"), "Data block with symbols only");
    }

    #[test]
    fn format_data_block_with_imports() {
        let source = "from indicators import {sma}\n\ndata {\n    symbols = [\"AAPL\"]\n    period = \"1y\"\n}\n\nstrategy S {\n    on bar {\n        x = sma(close, 20)\n    }\n}\n";
        let result = format_source(source);
        // Verify ordering: imports → blank → data → blank → strategy
        assert!(result.contains("import {sma}\n\ndata {"), "Blank line between imports and data block");
        assert!(result.contains("}\n\nstrategy S"), "Blank line between data block and strategy");
    }

    #[test]
    fn format_data_block_idempotent() {
        let source = "data {\n    symbols = [\"AAPL\", \"MSFT\"]\n    period = \"6mo\"\n    interval = \"1h\"\n    source = \"yahoo\"\n}\n\nstrategy S {\n    on bar {\n        x = 1\n    }\n}\n";
        let first = format_source(source);
        let second = format_source(&first);
        assert_eq!(first, second, "Data block formatting should be idempotent");
    }
}
