#![allow(dead_code)]
//! Main type-checking logic for the Flux language.
//!
//! The `TypeChecker` struct walks the untyped AST and produces a `TypedProgram`
//! with resolved types on every expression node. It maintains a scoped
//! environment for identifier resolution and enforces all type rules.

use crate::error::{CompileError, Result};
use crate::lexer::Span;
use crate::parser::ast::*;

use super::builtins;
use super::env::TypeEnvironment;
use super::typed_ast::*;
use super::types::{FluxType, FnParams};

/// The core type checker. Walks the untyped AST, resolves identifiers,
/// validates type compatibility, and produces an annotated typed AST.
pub(crate) struct TypeChecker {
    env: TypeEnvironment,
    in_event_handler: bool,
}

impl TypeChecker {
    /// Create a new TypeChecker with an empty global scope.
    pub fn new() -> Self {
        Self {
            env: TypeEnvironment::new(),
            in_event_handler: false,
        }
    }

    /// Type-check an entire program, producing a TypedProgram.
    pub fn check_program(&mut self, program: Program) -> Result<TypedProgram> {
        // Register imports into global scope
        self.register_imports(&program.imports)?;

        // Check strategy
        let typed_strategy = self.check_strategy(program.strategy)?;

        Ok(TypedProgram {
            imports: program.imports,
            strategy: typed_strategy,
            span: program.span,
        })
    }

    fn register_imports(&mut self, imports: &[Import]) -> Result<()> {
        for import in imports {
            for name in &import.names {
                if self.env.resolve(name).is_some() {
                    return Err(self.type_error(
                        import.span,
                        format!("duplicate import: '{}'", name),
                    ));
                }
                self.env.insert(
                    name.clone(),
                    FluxType::Fn {
                        params: FnParams::VariadicNumeric,
                        ret: Box::new(FluxType::Float),
                    },
                );
            }
        }
        Ok(())
    }

    fn check_strategy(&mut self, strategy: Strategy) -> Result<TypedStrategy> {
        self.env.push_scope(); // strategy scope

        // First pass: register params and state so they are visible to each other
        for item in &strategy.body {
            match item {
                StrategyItem::ParamsBlock(pb) => self.register_params(pb)?,
                StrategyItem::StateBlock(sb) => self.register_state(sb)?,
                _ => {}
            }
        }

        // Second pass: type-check all items
        let mut typed_body = Vec::new();
        for item in strategy.body {
            typed_body.push(self.check_strategy_item(item)?);
        }

        self.env.pop_scope(); // leave strategy scope

        Ok(TypedStrategy {
            name: strategy.name,
            body: typed_body,
            span: strategy.span,
        })
    }

    fn register_params(&mut self, params_block: &ParamsBlock) -> Result<()> {
        for param in &params_block.params {
            let ty = self.infer_literal_type(&param.default_value)?;
            self.env.insert(param.name.clone(), ty);
        }
        Ok(())
    }

    fn register_state(&mut self, state_block: &StateBlock) -> Result<()> {
        for var in &state_block.variables {
            let ty = self.infer_state_init_type(&var.initial_value)?;
            self.env.insert(var.name.clone(), ty);
        }
        Ok(())
    }

    fn infer_literal_type(&self, expr: &Expr) -> Result<FluxType> {
        match &expr.kind {
            ExprKind::IntLiteral(_) => Ok(FluxType::Int),
            ExprKind::FloatLiteral(_) => Ok(FluxType::Float),
            ExprKind::StringLiteral(_) => Ok(FluxType::String),
            ExprKind::BoolLiteral(_) => Ok(FluxType::Bool),
            ExprKind::NullLiteral => Ok(FluxType::Null),
            _ => Err(self.type_error(
                expr.span,
                "parameter default must be a literal value".to_string(),
            )),
        }
    }

    fn infer_state_init_type(&self, expr: &Expr) -> Result<FluxType> {
        match &expr.kind {
            ExprKind::IntLiteral(_) => Ok(FluxType::Int),
            ExprKind::FloatLiteral(_) => Ok(FluxType::Float),
            ExprKind::StringLiteral(_) => Ok(FluxType::String),
            ExprKind::BoolLiteral(_) => Ok(FluxType::Bool),
            ExprKind::NullLiteral => Ok(FluxType::Null),
            ExprKind::ListLiteral(elements) => {
                if elements.is_empty() {
                    Ok(FluxType::List(Box::new(FluxType::Null)))
                } else {
                    // Infer element type from first element (must be literal)
                    let first_ty = self.infer_literal_type(&elements[0])?;
                    for elem in elements.iter().skip(1) {
                        let elem_ty = self.infer_literal_type(elem)?;
                        if elem_ty != first_ty {
                            // Check numeric coercion
                            if first_ty.is_numeric() && elem_ty.is_numeric() {
                                continue; // will coerce to Float
                            }
                            return Err(self.type_error(
                                expr.span,
                                format!(
                                    "list elements have incompatible types: {} and {}",
                                    first_ty, elem_ty
                                ),
                            ));
                        }
                    }
                    // If mixed numeric, result is Float
                    let has_float = elements.iter().any(|e| {
                        matches!(e.kind, ExprKind::FloatLiteral(_))
                    });
                    if first_ty.is_numeric() && has_float {
                        Ok(FluxType::List(Box::new(FluxType::Float)))
                    } else {
                        Ok(FluxType::List(Box::new(first_ty)))
                    }
                }
            }
            ExprKind::Ident(name) => {
                if let Some(ty) = self.env.resolve(name) {
                    Ok(ty.clone())
                } else {
                    Err(self.type_error(
                        expr.span,
                        format!("undefined identifier '{}'", name),
                    ))
                }
            }
            _ => Err(self.type_error(
                expr.span,
                "state variable initializer must be a literal or list literal".to_string(),
            )),
        }
    }

    fn check_strategy_item(&mut self, item: StrategyItem) -> Result<TypedStrategyItem> {
        match item {
            StrategyItem::Property(prop) => {
                let typed_value = self.check_expr(prop.value)?;
                Ok(TypedStrategyItem::Property(TypedProperty {
                    name: prop.name,
                    value: typed_value,
                    span: prop.span,
                }))
            }
            StrategyItem::ParamsBlock(pb) => {
                let typed_params = self.check_params_block(pb)?;
                Ok(TypedStrategyItem::ParamsBlock(typed_params))
            }
            StrategyItem::StateBlock(sb) => {
                let typed_state = self.check_state_block(sb)?;
                Ok(TypedStrategyItem::StateBlock(typed_state))
            }
            StrategyItem::EventHandler(eh) => {
                let typed_handler = self.check_event_handler(eh)?;
                Ok(TypedStrategyItem::EventHandler(typed_handler))
            }
        }
    }

    fn check_params_block(&mut self, pb: ParamsBlock) -> Result<TypedParamsBlock> {
        let mut typed_params = Vec::new();
        for param in pb.params {
            let resolved_type = self.infer_literal_type(&param.default_value)?;
            let typed_default = self.check_expr(param.default_value)?;
            typed_params.push(TypedParam {
                name: param.name,
                default_value: typed_default,
                resolved_type,
                span: param.span,
            });
        }
        Ok(TypedParamsBlock {
            params: typed_params,
            span: pb.span,
        })
    }

    fn check_state_block(&mut self, sb: StateBlock) -> Result<TypedStateBlock> {
        let mut typed_vars = Vec::new();
        for var in sb.variables {
            let typed_init = self.check_expr(var.initial_value)?;
            let resolved_type = typed_init.resolved_type.clone();
            typed_vars.push(TypedStateVar {
                name: var.name,
                initial_value: typed_init,
                resolved_type,
                span: var.span,
            });
        }
        Ok(TypedStateBlock {
            variables: typed_vars,
            span: sb.span,
        })
    }

    fn check_event_handler(&mut self, handler: EventHandler) -> Result<TypedEventHandler> {
        // Validate event name
        if !builtins::valid_event_names().contains(&handler.event_name.as_str()) {
            return Err(self.type_error(
                handler.span,
                format!("unrecognized event handler '{}'", handler.event_name),
            ));
        }

        self.env.push_scope(); // handler scope
        self.in_event_handler = true;

        // Inject market data bindings
        for (name, ty) in builtins::market_data_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Inject signal function bindings
        for (name, ty) in builtins::signal_function_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Inject math/stats/portfolio function bindings
        for (name, ty) in builtins::math_function_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Check handler body statements
        let mut typed_body = Vec::new();
        for stmt in handler.body {
            typed_body.push(self.check_stmt(stmt)?);
        }

        self.in_event_handler = false;
        self.env.pop_scope(); // leave handler scope

        Ok(TypedEventHandler {
            event_name: handler.event_name,
            body: typed_body,
            span: handler.span,
        })
    }

    fn check_stmt(&mut self, stmt: Stmt) -> Result<TypedStmt> {
        match stmt {
            Stmt::Assignment(assign) => self.check_assignment(assign),
            Stmt::If(if_stmt) => self.check_if(if_stmt),
            Stmt::For(for_loop) => self.check_for(for_loop),
            Stmt::While(while_loop) => self.check_while(while_loop),
            Stmt::Return(ret) => self.check_return(ret),
            Stmt::Expr(expr_stmt) => self.check_expr_stmt(expr_stmt),
        }
    }

    fn check_assignment(&mut self, assign: Assignment) -> Result<TypedStmt> {
        let typed_value = self.check_expr(assign.value)?;
        let value_type = typed_value.resolved_type.clone();

        // Handle different assignment targets
        match &assign.target.kind {
            ExprKind::Ident(name) => {
                if let Some(existing_ty) = self.env.resolve(name).cloned() {
                    // Reassignment: check type compatibility
                    if !value_type.is_assignable_to(&existing_ty) {
                        return Err(self.type_error(
                            assign.span,
                            format!(
                                "cannot assign {} to variable of type {}",
                                value_type, existing_ty
                            ),
                        ));
                    }
                } else {
                    // New variable: add to current scope
                    self.env.insert(name.clone(), value_type.clone());
                }
                let typed_target = TypedExpr {
                    kind: TypedExprKind::Ident(name.clone()),
                    resolved_type: value_type,
                    span: assign.target.span,
                };
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
            ExprKind::IndexAccess { .. } => {
                let typed_target = self.check_expr(assign.target)?;
                // Verify value type matches element type
                if !value_type.is_assignable_to(&typed_target.resolved_type) {
                    return Err(self.type_error(
                        assign.span,
                        format!(
                            "cannot assign {} to element of type {}",
                            value_type, typed_target.resolved_type
                        ),
                    ));
                }
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
            ExprKind::MemberAccess { .. } => {
                let typed_target = self.check_expr(assign.target)?;
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
            _ => {
                // Type-check the target expression normally
                let typed_target = self.check_expr(assign.target)?;
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
        }
    }

    fn check_if(&mut self, if_stmt: IfStmt) -> Result<TypedStmt> {
        let typed_condition = self.check_expr(if_stmt.condition)?;
        if typed_condition.resolved_type != FluxType::Bool {
            return Err(self.type_error(
                typed_condition.span,
                format!(
                    "if condition must be Bool, found {}",
                    typed_condition.resolved_type
                ),
            ));
        }

        // Check if body in new scope
        self.env.push_scope();
        let mut typed_body = Vec::new();
        for stmt in if_stmt.body {
            typed_body.push(self.check_stmt(stmt)?);
        }
        self.env.pop_scope();

        // Check elif branches
        let mut typed_elifs = Vec::new();
        for elif in if_stmt.elif_branches {
            let typed_elif_cond = self.check_expr(elif.condition)?;
            if typed_elif_cond.resolved_type != FluxType::Bool {
                return Err(self.type_error(
                    typed_elif_cond.span,
                    format!(
                        "elif condition must be Bool, found {}",
                        typed_elif_cond.resolved_type
                    ),
                ));
            }
            self.env.push_scope();
            let mut typed_elif_body = Vec::new();
            for stmt in elif.body {
                typed_elif_body.push(self.check_stmt(stmt)?);
            }
            self.env.pop_scope();
            typed_elifs.push(TypedElifBranch {
                condition: typed_elif_cond,
                body: typed_elif_body,
                span: elif.span,
            });
        }

        // Check else body
        let typed_else = if let Some(else_body) = if_stmt.else_body {
            self.env.push_scope();
            let mut typed_else_body = Vec::new();
            for stmt in else_body {
                typed_else_body.push(self.check_stmt(stmt)?);
            }
            self.env.pop_scope();
            Some(typed_else_body)
        } else {
            None
        };

        Ok(TypedStmt::If(TypedIfStmt {
            condition: typed_condition,
            body: typed_body,
            elif_branches: typed_elifs,
            else_body: typed_else,
            span: if_stmt.span,
        }))
    }

    fn check_for(&mut self, for_loop: ForLoop) -> Result<TypedStmt> {
        let typed_iterable = self.check_expr(for_loop.iterable)?;

        // Iterable must be a List type
        let elem_type = match &typed_iterable.resolved_type {
            FluxType::List(t) => t.as_ref().clone(),
            other => {
                return Err(self.type_error(
                    typed_iterable.span,
                    format!("for-loop requires List type, found {}", other),
                ));
            }
        };

        // Push loop body scope with the loop variable bound
        self.env.push_scope();
        self.env.insert(for_loop.variable.clone(), elem_type.clone());

        let mut typed_body = Vec::new();
        for stmt in for_loop.body {
            typed_body.push(self.check_stmt(stmt)?);
        }

        self.env.pop_scope();

        Ok(TypedStmt::For(TypedForLoop {
            variable: for_loop.variable,
            variable_type: elem_type,
            iterable: typed_iterable,
            body: typed_body,
            span: for_loop.span,
        }))
    }

    fn check_while(&mut self, while_loop: WhileLoop) -> Result<TypedStmt> {
        let typed_condition = self.check_expr(while_loop.condition)?;
        if typed_condition.resolved_type != FluxType::Bool {
            return Err(self.type_error(
                typed_condition.span,
                format!(
                    "while condition must be Bool, found {}",
                    typed_condition.resolved_type
                ),
            ));
        }

        self.env.push_scope();
        let mut typed_body = Vec::new();
        for stmt in while_loop.body {
            typed_body.push(self.check_stmt(stmt)?);
        }
        self.env.pop_scope();

        Ok(TypedStmt::While(TypedWhileLoop {
            condition: typed_condition,
            body: typed_body,
            span: while_loop.span,
        }))
    }

    fn check_return(&mut self, ret: ReturnStmt) -> Result<TypedStmt> {
        let typed_value = if let Some(val) = ret.value {
            Some(self.check_expr(val)?)
        } else {
            None
        };
        Ok(TypedStmt::Return(TypedReturnStmt {
            value: typed_value,
            span: ret.span,
        }))
    }

    fn check_expr_stmt(&mut self, expr_stmt: ExprStmt) -> Result<TypedStmt> {
        let typed_expr = self.check_expr(expr_stmt.expr)?;
        Ok(TypedStmt::Expr(TypedExprStmt {
            expr: typed_expr,
            span: expr_stmt.span,
        }))
    }

    // -----------------------------------------------------------------------
    // Expression checking
    // -----------------------------------------------------------------------

    /// Type-check an expression, returning a TypedExpr with a resolved type.
    pub fn check_expr(&mut self, expr: Expr) -> Result<TypedExpr> {
        let span = expr.span;
        match expr.kind {
            ExprKind::IntLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::IntLiteral(v),
                resolved_type: FluxType::Int,
                span,
            }),
            ExprKind::FloatLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::FloatLiteral(v),
                resolved_type: FluxType::Float,
                span,
            }),
            ExprKind::StringLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::StringLiteral(v),
                resolved_type: FluxType::String,
                span,
            }),
            ExprKind::BoolLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::BoolLiteral(v),
                resolved_type: FluxType::Bool,
                span,
            }),
            ExprKind::NullLiteral => Ok(TypedExpr {
                kind: TypedExprKind::NullLiteral,
                resolved_type: FluxType::Null,
                span,
            }),
            ExprKind::Ident(name) => self.check_ident(&name, span),
            ExprKind::BinaryOp { left, op, right } => {
                self.check_binary_op(*left, op, *right, span)
            }
            ExprKind::UnaryOp { op, operand } => self.check_unary_op(op, *operand, span),
            ExprKind::FunctionCall { function, args } => {
                self.check_function_call(*function, args, span)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => self.check_method_call(*receiver, &method, args, span),
            ExprKind::IndexAccess { object, index } => {
                self.check_index_access(*object, *index, span)
            }
            ExprKind::ListLiteral(elements) => self.check_list_literal(elements, span),
            ExprKind::MemberAccess { object, field } => {
                self.check_member_access(*object, &field, span)
            }
        }
    }

    fn check_ident(&mut self, name: &str, span: Span) -> Result<TypedExpr> {
        if let Some(ty) = self.env.resolve(name) {
            let resolved = ty.clone();
            Ok(TypedExpr {
                kind: TypedExprKind::Ident(name.to_string()),
                resolved_type: resolved,
                span,
            })
        } else {
            // Check if it's a market data identifier used outside an event handler
            let market_data_names: Vec<&str> = builtins::market_data_bindings()
                .iter()
                .map(|(n, _)| *n)
                .collect();
            if market_data_names.contains(&name) && !self.in_event_handler {
                Err(self.type_error(
                    span,
                    format!("'{}' is only available inside event handlers", name),
                ))
            } else {
                Err(self.type_error(
                    span,
                    format!("undefined identifier '{}'", name),
                ))
            }
        }
    }

    fn check_binary_op(
        &mut self,
        left: Expr,
        op: BinOp,
        right: Expr,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_left = self.check_expr(left)?;
        let typed_right = self.check_expr(right)?;
        let left_ty = &typed_left.resolved_type;
        let right_ty = &typed_right.resolved_type;

        let result_type = match op {
            // Arithmetic operators
            BinOp::Add => {
                // String concatenation
                if left_ty == &FluxType::String && right_ty == &FluxType::String {
                    FluxType::String
                } else if let Some(ty) = FluxType::arithmetic_result(left_ty, right_ty) {
                    ty
                } else {
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '+' requires numeric operands, found {} and {}",
                            left_ty, right_ty
                        ),
                    ));
                }
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                if let Some(ty) = FluxType::arithmetic_result(left_ty, right_ty) {
                    ty
                } else {
                    let op_str = match op {
                        BinOp::Sub => "-",
                        BinOp::Mul => "*",
                        BinOp::Div => "/",
                        BinOp::Mod => "%",
                        _ => unreachable!(),
                    };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires numeric operands, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
            }
            // Comparison operators (ordering)
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if left_ty.is_numeric() && right_ty.is_numeric() {
                    FluxType::Bool
                } else {
                    let op_str = match op {
                        BinOp::Lt => "<",
                        BinOp::Le => "<=",
                        BinOp::Gt => ">",
                        BinOp::Ge => ">=",
                        _ => unreachable!(),
                    };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires numeric operands, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
            }
            // Equality operators
            BinOp::Eq | BinOp::Ne => {
                if left_ty == right_ty {
                    FluxType::Bool
                } else if left_ty.is_numeric() && right_ty.is_numeric() {
                    FluxType::Bool
                } else {
                    let op_str = if op == BinOp::Eq { "==" } else { "!=" };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires matching types, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
            }
            // Logical operators
            BinOp::And | BinOp::Or => {
                if left_ty != &FluxType::Bool || right_ty != &FluxType::Bool {
                    let op_str = if op == BinOp::And { "and" } else { "or" };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires boolean operands, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
                FluxType::Bool
            }
        };

        Ok(TypedExpr {
            kind: TypedExprKind::BinaryOp {
                left: Box::new(typed_left),
                op,
                right: Box::new(typed_right),
            },
            resolved_type: result_type,
            span,
        })
    }

    fn check_unary_op(&mut self, op: UnaryOp, operand: Expr, span: Span) -> Result<TypedExpr> {
        let typed_operand = self.check_expr(operand)?;
        let operand_ty = &typed_operand.resolved_type;

        let result_type = match op {
            UnaryOp::Neg => {
                if !operand_ty.is_numeric() {
                    return Err(self.type_error(
                        span,
                        format!("negation requires numeric operand, found {}", operand_ty),
                    ));
                }
                operand_ty.clone()
            }
            UnaryOp::Not => {
                if operand_ty != &FluxType::Bool {
                    return Err(self.type_error(
                        span,
                        format!(
                            "logical negation requires boolean operand, found {}",
                            operand_ty
                        ),
                    ));
                }
                FluxType::Bool
            }
        };

        Ok(TypedExpr {
            kind: TypedExprKind::UnaryOp {
                op,
                operand: Box::new(typed_operand),
            },
            resolved_type: result_type,
            span,
        })
    }

    fn check_function_call(
        &mut self,
        function: Expr,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_function = self.check_expr(function)?;

        // Check the function expression is callable
        let fn_type = typed_function.resolved_type.clone();
        match &fn_type {
            FluxType::Fn { params, ret } => {
                let typed_args = self.check_call_args(
                    &typed_function,
                    params,
                    args,
                    span,
                )?;
                let ret_type = ret.as_ref().clone();
                Ok(TypedExpr {
                    kind: TypedExprKind::FunctionCall {
                        function: Box::new(typed_function),
                        args: typed_args,
                    },
                    resolved_type: ret_type,
                    span,
                })
            }
            _ => {
                // Get the function name for a better error message
                let name = match &typed_function.kind {
                    TypedExprKind::Ident(n) => n.clone(),
                    _ => "expression".to_string(),
                };
                Err(self.type_error(
                    span,
                    format!("'{}' is not a function (type: {})", name, fn_type),
                ))
            }
        }
    }

    fn check_call_args(
        &mut self,
        function: &TypedExpr,
        params: &FnParams,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<Vec<TypedExpr>> {
        let fn_name = match &function.kind {
            TypedExprKind::Ident(n) => n.clone(),
            _ => "function".to_string(),
        };

        match params {
            FnParams::Fixed(param_types) => {
                if args.len() != param_types.len() {
                    return Err(self.type_error(
                        span,
                        format!(
                            "'{}' expects {} arguments, found {}",
                            fn_name,
                            param_types.len(),
                            args.len()
                        ),
                    ));
                }
                let mut typed_args = Vec::new();
                for (i, (arg, expected_ty)) in
                    args.into_iter().zip(param_types.iter()).enumerate()
                {
                    let typed_arg = self.check_expr(arg)?;
                    if !typed_arg.resolved_type.is_assignable_to(expected_ty) {
                        return Err(self.type_error(
                            typed_arg.span,
                            format!(
                                "'{}' argument {} must be {}, found {}",
                                fn_name,
                                i + 1,
                                expected_ty,
                                typed_arg.resolved_type
                            ),
                        ));
                    }
                    typed_args.push(typed_arg);
                }
                Ok(typed_args)
            }
            FnParams::VariadicNumeric => {
                let mut typed_args = Vec::new();
                for (i, arg) in args.into_iter().enumerate() {
                    let typed_arg = self.check_expr(arg)?;
                    if !typed_arg.resolved_type.is_numeric() {
                        return Err(self.type_error(
                            typed_arg.span,
                            format!(
                                "'{}' argument {} must be numeric, found {}",
                                fn_name,
                                i + 1,
                                typed_arg.resolved_type
                            ),
                        ));
                    }
                    typed_args.push(typed_arg);
                }
                Ok(typed_args)
            }
            FnParams::Overloaded(signatures) => {
                // Type-check all args first
                let mut typed_args = Vec::new();
                for arg in args {
                    typed_args.push(self.check_expr(arg)?);
                }

                // Try each signature
                for sig in signatures {
                    if typed_args.len() != sig.len() {
                        continue;
                    }
                    let mut matches = true;
                    for (typed_arg, expected_ty) in typed_args.iter().zip(sig.iter()) {
                        if !typed_arg.resolved_type.is_assignable_to(expected_ty) {
                            matches = false;
                            break;
                        }
                    }
                    if matches {
                        return Ok(typed_args);
                    }
                }

                // No signature matched — generate helpful error
                let arg_count = typed_args.len();
                let expected_counts: Vec<usize> =
                    signatures.iter().map(|s| s.len()).collect();
                if !expected_counts.contains(&arg_count) {
                    Err(self.type_error(
                        span,
                        format!(
                            "'{}' expects {} arguments, found {}",
                            fn_name,
                            expected_counts
                                .iter()
                                .map(|c| c.to_string())
                                .collect::<Vec<_>>()
                                .join(" or "),
                            arg_count
                        ),
                    ))
                } else {
                    // Arg count matched at least one sig but types were wrong
                    let arg_types: Vec<String> = typed_args
                        .iter()
                        .map(|a| a.resolved_type.to_string())
                        .collect();
                    Err(self.type_error(
                        span,
                        format!(
                            "'{}' called with incompatible argument types: ({})",
                            fn_name,
                            arg_types.join(", ")
                        ),
                    ))
                }
            }
        }
    }

    fn check_method_call(
        &mut self,
        receiver: Expr,
        method: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_receiver = self.check_expr(receiver)?;
        let receiver_ty = typed_receiver.resolved_type.clone();

        match &receiver_ty {
            FluxType::List(elem_ty) => {
                let elem_type = elem_ty.as_ref().clone();
                match method {
                    "append" => {
                        if args.len() != 1 {
                            return Err(self.type_error(
                                span,
                                format!("'append' expects 1 argument, found {}", args.len()),
                            ));
                        }
                        let typed_arg = self.check_expr(args.into_iter().next().unwrap())?;
                        if !typed_arg.resolved_type.is_assignable_to(&elem_type) {
                            return Err(self.type_error(
                                typed_arg.span,
                                format!(
                                    "'append' argument must be {}, found {}",
                                    elem_type, typed_arg.resolved_type
                                ),
                            ));
                        }
                        Ok(TypedExpr {
                            kind: TypedExprKind::MethodCall {
                                receiver: Box::new(typed_receiver),
                                method: method.to_string(),
                                args: vec![typed_arg],
                            },
                            resolved_type: FluxType::Void,
                            span,
                        })
                    }
                    "len" => {
                        if !args.is_empty() {
                            return Err(self.type_error(
                                span,
                                format!("'len' expects 0 arguments, found {}", args.len()),
                            ));
                        }
                        Ok(TypedExpr {
                            kind: TypedExprKind::MethodCall {
                                receiver: Box::new(typed_receiver),
                                method: method.to_string(),
                                args: vec![],
                            },
                            resolved_type: FluxType::Int,
                            span,
                        })
                    }
                    "pop" => {
                        if !args.is_empty() {
                            return Err(self.type_error(
                                span,
                                format!("'pop' expects 0 arguments, found {}", args.len()),
                            ));
                        }
                        Ok(TypedExpr {
                            kind: TypedExprKind::MethodCall {
                                receiver: Box::new(typed_receiver),
                                method: method.to_string(),
                                args: vec![],
                            },
                            resolved_type: elem_type,
                            span,
                        })
                    }
                    _ => Err(self.type_error(
                        span,
                        format!(
                            "type {} does not have method '{}'",
                            receiver_ty, method
                        ),
                    )),
                }
            }
            _ => Err(self.type_error(
                span,
                format!("type {} does not have method '{}'", receiver_ty, method),
            )),
        }
    }

    fn check_index_access(
        &mut self,
        object: Expr,
        index: Expr,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_object = self.check_expr(object)?;
        let typed_index = self.check_expr(index)?;

        // Receiver must be List(T)
        let elem_type = match &typed_object.resolved_type {
            FluxType::List(t) => t.as_ref().clone(),
            other => {
                return Err(self.type_error(
                    span,
                    format!("type {} does not support indexing", other),
                ));
            }
        };

        // Index must be Int
        if typed_index.resolved_type != FluxType::Int {
            return Err(self.type_error(
                typed_index.span,
                format!(
                    "index must be Int, found {}",
                    typed_index.resolved_type
                ),
            ));
        }

        Ok(TypedExpr {
            kind: TypedExprKind::IndexAccess {
                object: Box::new(typed_object),
                index: Box::new(typed_index),
            },
            resolved_type: elem_type,
            span,
        })
    }

    fn check_list_literal(&mut self, elements: Vec<Expr>, span: Span) -> Result<TypedExpr> {
        if elements.is_empty() {
            return Ok(TypedExpr {
                kind: TypedExprKind::ListLiteral(vec![]),
                resolved_type: FluxType::List(Box::new(FluxType::Null)),
                span,
            });
        }

        let mut typed_elements = Vec::new();
        for elem in elements {
            typed_elements.push(self.check_expr(elem)?);
        }

        // Infer element type
        let first_ty = typed_elements[0].resolved_type.clone();
        let mut all_same = true;
        let mut all_numeric = first_ty.is_numeric();

        for elem in typed_elements.iter().skip(1) {
            if elem.resolved_type != first_ty {
                all_same = false;
            }
            if !elem.resolved_type.is_numeric() {
                all_numeric = false;
            }
        }

        let elem_type = if all_same {
            // Homogeneous list
            first_ty
        } else if all_numeric {
            // Mixed Int/Float → List(Float)
            FluxType::Float
        } else {
            // Incompatible types
            // Find the first type that differs from the first element
            let other_ty = typed_elements
                .iter()
                .skip(1)
                .find(|e| e.resolved_type != first_ty)
                .map(|e| &e.resolved_type)
                .unwrap();
            return Err(self.type_error(
                span,
                format!(
                    "list elements have incompatible types: {} and {}",
                    first_ty, other_ty
                ),
            ));
        };

        Ok(TypedExpr {
            kind: TypedExprKind::ListLiteral(typed_elements),
            resolved_type: FluxType::List(Box::new(elem_type)),
            span,
        })
    }

    fn check_member_access(
        &mut self,
        object: Expr,
        field: &str,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_object = self.check_expr(object)?;
        // Currently no types in Flux support member access (no structs/objects)
        Err(self.type_error(
            span,
            format!(
                "type {} does not support member access '{}'",
                typed_object.resolved_type, field
            ),
        ))
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Construct a `CompileError::Type` with a consistent format:
    /// `"at byte {span.start}: {description}"`
    fn type_error(&self, span: Span, message: String) -> CompileError {
        CompileError::Type(format!("at byte {}: {}", span.start, message))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;
    use crate::parser::ast::{
        Assignment, BinOp, ElifBranch, Expr, ExprKind, ForLoop, IfStmt, Stmt, UnaryOp, WhileLoop,
    };

    fn make_expr(kind: ExprKind) -> Expr {
        Expr { kind, span: Span::new(0, 1) }
    }

    // -----------------------------------------------------------------------
    // 1. Literals
    // -----------------------------------------------------------------------

    #[test]
    fn test_literal_int() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::IntLiteral(42))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_literal_float() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::FloatLiteral(3.14))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_literal_string() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::StringLiteral("hello".to_string()))).unwrap();
        assert_eq!(result.resolved_type, FluxType::String);
    }

    #[test]
    fn test_literal_bool() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::BoolLiteral(true))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    // -----------------------------------------------------------------------
    // 2. Identifier resolution
    // -----------------------------------------------------------------------

    #[test]
    fn test_ident_resolved() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let result = tc.check_expr(make_expr(ExprKind::Ident("x".to_string()))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_ident_undefined() {
        let mut tc = TypeChecker::new();
        let err = tc.check_expr(make_expr(ExprKind::Ident("unknown".to_string()))).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undefined identifier 'unknown'"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 3. Binary ops - arithmetic
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_int_int() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::IntLiteral(2))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_add_float_float() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::FloatLiteral(1.0))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::FloatLiteral(2.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_add_int_float() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::FloatLiteral(2.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_add_string_string() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("hello".to_string()))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::StringLiteral(" world".to_string()))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::String);
    }

    #[test]
    fn test_add_string_int_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("hello".to_string()))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("numeric operands") || msg.contains("String") && msg.contains("Int"),
            "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 4. Binary ops - comparison
    // -----------------------------------------------------------------------

    #[test]
    fn test_lt_numeric() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Lt,
            right: Box::new(make_expr(ExprKind::FloatLiteral(2.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_lt_non_numeric_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("a".to_string()))),
            op: BinOp::Lt,
            right: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("numeric operands"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 5. Binary ops - equality
    // -----------------------------------------------------------------------

    #[test]
    fn test_eq_same_type() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Eq,
            right: Box::new(make_expr(ExprKind::IntLiteral(2))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_eq_numeric_cross() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Eq,
            right: Box::new(make_expr(ExprKind::FloatLiteral(1.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_eq_incompatible_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("a".to_string()))),
            op: BinOp::Eq,
            right: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("matching types"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 6. Binary ops - logical
    // -----------------------------------------------------------------------

    #[test]
    fn test_and_bool_bool() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::BoolLiteral(true))),
            op: BinOp::And,
            right: Box::new(make_expr(ExprKind::BoolLiteral(false))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_or_non_bool_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Or,
            right: Box::new(make_expr(ExprKind::BoolLiteral(true))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("boolean operands"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 7. Unary ops
    // -----------------------------------------------------------------------

    #[test]
    fn test_neg_int() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Neg,
            operand: Box::new(make_expr(ExprKind::IntLiteral(5))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_neg_string_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Neg,
            operand: Box::new(make_expr(ExprKind::StringLiteral("x".to_string()))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("numeric operand"), "got: {}", msg);
    }

    #[test]
    fn test_not_bool() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(make_expr(ExprKind::BoolLiteral(true))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_not_int_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("boolean operand"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 8. Function calls
    // -----------------------------------------------------------------------

    #[test]
    fn test_call_variadic_numeric() {
        let mut tc = TypeChecker::new();
        tc.env.insert("sma".to_string(), FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        });
        let expr = make_expr(ExprKind::FunctionCall {
            function: Box::new(make_expr(ExprKind::Ident("sma".to_string()))),
            args: vec![
                make_expr(ExprKind::IntLiteral(10)),
                make_expr(ExprKind::IntLiteral(20)),
            ],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_call_not_callable() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let expr = make_expr(ExprKind::FunctionCall {
            function: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
            args: vec![],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not a function"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 9. Method calls
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_len() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            method: "len".to_string(),
            args: vec![],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_list_append() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            method: "append".to_string(),
            args: vec![make_expr(ExprKind::IntLiteral(42))],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Void);
    }

    #[test]
    fn test_list_pop() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            method: "pop".to_string(),
            args: vec![],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_invalid_method_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
            method: "len".to_string(),
            args: vec![],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does not have method"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 10. Index access
    // -----------------------------------------------------------------------

    #[test]
    fn test_index_list_int() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::IndexAccess {
            object: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            index: Box::new(make_expr(ExprKind::IntLiteral(0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_index_non_list_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let expr = make_expr(ExprKind::IndexAccess {
            object: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
            index: Box::new(make_expr(ExprKind::IntLiteral(0))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does not support indexing"), "got: {}", msg);
    }

    #[test]
    fn test_index_non_int_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::IndexAccess {
            object: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            index: Box::new(make_expr(ExprKind::StringLiteral("x".to_string()))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("index must be Int"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 11. List literals
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_list() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![]));
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::List(Box::new(FluxType::Null)));
    }

    #[test]
    fn test_homogeneous_list() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![
            make_expr(ExprKind::IntLiteral(1)),
            make_expr(ExprKind::IntLiteral(2)),
            make_expr(ExprKind::IntLiteral(3)),
        ]));
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::List(Box::new(FluxType::Int)));
    }

    #[test]
    fn test_mixed_numeric_list() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![
            make_expr(ExprKind::IntLiteral(1)),
            make_expr(ExprKind::FloatLiteral(2.0)),
        ]));
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::List(Box::new(FluxType::Float)));
    }

    #[test]
    fn test_incompatible_list_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![
            make_expr(ExprKind::IntLiteral(1)),
            make_expr(ExprKind::StringLiteral("hello".to_string())),
        ]));
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("incompatible types"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 12. Statements - assignment
    // -----------------------------------------------------------------------

    fn make_stmt_assignment(target_name: &str, value: ExprKind) -> Stmt {
        Stmt::Assignment(Assignment {
            target: Expr { kind: ExprKind::Ident(target_name.to_string()), span: Span::new(0, 1) },
            value: Expr { kind: value, span: Span::new(2, 3) },
            span: Span::new(0, 3),
        })
    }

    #[test]
    fn test_assignment_new_variable() {
        let mut tc = TypeChecker::new();
        let stmt = make_stmt_assignment("x", ExprKind::IntLiteral(42));
        tc.check_stmt(stmt).unwrap();
        // New variable should now be in scope
        assert_eq!(tc.env.resolve("x"), Some(&FluxType::Int));
    }

    #[test]
    fn test_assignment_existing_variable_ok() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let stmt = make_stmt_assignment("x", ExprKind::IntLiteral(99));
        tc.check_stmt(stmt).unwrap();
        // Variable still exists with same type
        assert_eq!(tc.env.resolve("x"), Some(&FluxType::Int));
    }

    #[test]
    fn test_assignment_type_mismatch() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let stmt = make_stmt_assignment("x", ExprKind::StringLiteral("hello".to_string()));
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot assign") && msg.contains("String") && msg.contains("Int"),
            "got: {}", msg);
    }

    #[test]
    fn test_assignment_int_to_float_coercion() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Float);
        let stmt = make_stmt_assignment("x", ExprKind::IntLiteral(5));
        // Int is assignable to Float (coercion)
        tc.check_stmt(stmt).unwrap();
        assert_eq!(tc.env.resolve("x"), Some(&FluxType::Float));
    }

    // -----------------------------------------------------------------------
    // 13. Statements - if/elif/else
    // -----------------------------------------------------------------------

    #[test]
    fn test_if_bool_condition() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 10),
        });
        tc.check_stmt(stmt).unwrap();
    }

    #[test]
    fn test_if_non_bool_condition_error() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::IntLiteral(1), span: Span::new(0, 1) },
            body: vec![],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 10),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be Bool"), "got: {}", msg);
    }

    #[test]
    fn test_elif_non_bool_condition_error() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![],
            elif_branches: vec![
                ElifBranch {
                    condition: Expr { kind: ExprKind::IntLiteral(0), span: Span::new(10, 11) },
                    body: vec![],
                    span: Span::new(10, 20),
                },
            ],
            else_body: None,
            span: Span::new(0, 20),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be Bool"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 14. Statements - for loop
    // -----------------------------------------------------------------------

    #[test]
    fn test_for_list_iterable() {
        let mut tc = TypeChecker::new();
        tc.env.insert("items".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let stmt = Stmt::For(ForLoop {
            variable: "item".to_string(),
            iterable: Expr { kind: ExprKind::Ident("items".to_string()), span: Span::new(5, 10) },
            body: vec![],
            span: Span::new(0, 20),
        });
        tc.check_stmt(stmt).unwrap();
    }

    #[test]
    fn test_for_non_list_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("count".to_string(), FluxType::Int);
        let stmt = Stmt::For(ForLoop {
            variable: "item".to_string(),
            iterable: Expr { kind: ExprKind::Ident("count".to_string()), span: Span::new(5, 10) },
            body: vec![],
            span: Span::new(0, 20),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("requires List type"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 15. Statements - while loop
    // -----------------------------------------------------------------------

    #[test]
    fn test_while_bool_condition() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::While(WhileLoop {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![],
            span: Span::new(0, 10),
        });
        tc.check_stmt(stmt).unwrap();
    }

    #[test]
    fn test_while_non_bool_error() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::While(WhileLoop {
            condition: Expr { kind: ExprKind::IntLiteral(1), span: Span::new(0, 1) },
            body: vec![],
            span: Span::new(0, 10),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be Bool"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 16. Scope isolation
    // -----------------------------------------------------------------------

    #[test]
    fn test_scope_isolation_if() {
        let mut tc = TypeChecker::new();
        // Create an if statement that declares a variable inside the body
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![
                make_stmt_assignment("inner_var", ExprKind::IntLiteral(10)),
            ],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 30),
        });
        tc.check_stmt(stmt).unwrap();
        // Variable declared inside if body should NOT be accessible after
        assert_eq!(tc.env.resolve("inner_var"), None);
    }

    // -----------------------------------------------------------------------
    // 17. Top-level program checking
    // -----------------------------------------------------------------------

    use crate::parser::ast::{
        Program, Strategy, StrategyItem, Import, ParamsBlock, Param,
        StateBlock, StateVar, EventHandler, ExprStmt, Property,
    };

    fn make_program(imports: Vec<Import>, body: Vec<StrategyItem>) -> Program {
        Program {
            imports,
            strategy: Strategy {
                name: "Test".to_string(),
                body,
                span: Span::new(0, 100),
            },
            span: Span::new(0, 100),
        }
    }

    #[test]
    fn test_check_program_minimal() {
        let mut tc = TypeChecker::new();
        let program = make_program(vec![], vec![]);
        let result = tc.check_program(program);
        assert!(result.is_ok(), "minimal program should type-check: {:?}", result.err());
    }

    #[test]
    fn test_import_registration() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![Import {
                module_path: "indicators".to_string(),
                names: vec!["sma".to_string()],
                span: Span::new(0, 20),
            }],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("sma".to_string()),
                                span: Span::new(30, 33),
                            }),
                            args: vec![Expr {
                                kind: ExprKind::IntLiteral(20),
                                span: Span::new(34, 36),
                            }],
                        },
                        span: Span::new(30, 37),
                    },
                    span: Span::new(30, 37),
                })],
                span: Span::new(25, 50),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "imported fn should be callable: {:?}", result.err());
    }

    #[test]
    fn test_duplicate_import_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![
                Import {
                    module_path: "indicators".to_string(),
                    names: vec!["sma".to_string()],
                    span: Span::new(0, 20),
                },
                Import {
                    module_path: "indicators".to_string(),
                    names: vec!["sma".to_string()],
                    span: Span::new(21, 40),
                },
            ],
            vec![],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate import"), "got: {}", msg);
    }

    #[test]
    fn test_params_literal_defaults() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::ParamsBlock(ParamsBlock {
                params: vec![
                    Param {
                        name: "period".to_string(),
                        default_value: Expr { kind: ExprKind::IntLiteral(20), span: Span::new(10, 12) },
                        span: Span::new(5, 12),
                    },
                    Param {
                        name: "threshold".to_string(),
                        default_value: Expr { kind: ExprKind::FloatLiteral(2.0), span: Span::new(15, 18) },
                        span: Span::new(13, 18),
                    },
                    Param {
                        name: "name".to_string(),
                        default_value: Expr { kind: ExprKind::StringLiteral("test".to_string()), span: Span::new(20, 26) },
                        span: Span::new(19, 26),
                    },
                    Param {
                        name: "enabled".to_string(),
                        default_value: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(28, 32) },
                        span: Span::new(27, 32),
                    },
                ],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "params with literal defaults should pass: {:?}", result.err());
    }

    #[test]
    fn test_params_non_literal_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::ParamsBlock(ParamsBlock {
                params: vec![Param {
                    name: "bad".to_string(),
                    default_value: Expr {
                        kind: ExprKind::BinaryOp {
                            left: Box::new(Expr { kind: ExprKind::IntLiteral(1), span: Span::new(10, 11) }),
                            op: BinOp::Add,
                            right: Box::new(Expr { kind: ExprKind::IntLiteral(2), span: Span::new(14, 15) }),
                        },
                        span: Span::new(10, 15),
                    },
                    span: Span::new(5, 15),
                }],
                span: Span::new(0, 20),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be a literal"), "got: {}", msg);
    }

    #[test]
    fn test_state_literal_init() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::StateBlock(StateBlock {
                variables: vec![StateVar {
                    name: "count".to_string(),
                    initial_value: Expr { kind: ExprKind::IntLiteral(0), span: Span::new(10, 11) },
                    span: Span::new(5, 11),
                }],
                span: Span::new(0, 20),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "state with literal init should pass: {:?}", result.err());
    }

    #[test]
    fn test_state_list_init() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::StateBlock(StateBlock {
                variables: vec![StateVar {
                    name: "prices".to_string(),
                    initial_value: Expr { kind: ExprKind::ListLiteral(vec![]), span: Span::new(10, 12) },
                    span: Span::new(5, 12),
                }],
                span: Span::new(0, 20),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "state with [] init should pass: {:?}", result.err());
    }

    #[test]
    fn test_state_undefined_ident_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::StateBlock(StateBlock {
                variables: vec![StateVar {
                    name: "x".to_string(),
                    initial_value: Expr {
                        kind: ExprKind::Ident("undefined_var".to_string()),
                        span: Span::new(10, 23),
                    },
                    span: Span::new(5, 23),
                }],
                span: Span::new(0, 30),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undefined identifier"), "got: {}", msg);
    }

    #[test]
    fn test_event_handler_valid() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![],
                span: Span::new(0, 20),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "on_bar handler should be valid: {:?}", result.err());
    }

    #[test]
    fn test_event_handler_invalid_name() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "tick".to_string(),
                body: vec![],
                span: Span::new(0, 20),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unrecognized event handler"), "got: {}", msg);
    }

    #[test]
    fn test_market_data_inside_handler() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::Ident("close".to_string()),
                        span: Span::new(10, 15),
                    },
                    span: Span::new(10, 15),
                })],
                span: Span::new(0, 30),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "close should be accessible inside handler: {:?}", result.err());
    }

    #[test]
    fn test_market_data_outside_handler_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::Property(Property {
                name: "value".to_string(),
                value: Expr {
                    kind: ExprKind::Ident("close".to_string()),
                    span: Span::new(10, 15),
                },
                span: Span::new(5, 15),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("only available inside event handlers"), "got: {}", msg);
    }

    #[test]
    fn test_signal_open_valid() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("OPEN".to_string()),
                                span: Span::new(10, 14),
                            }),
                            args: vec![
                                Expr {
                                    kind: ExprKind::Ident("symbol".to_string()),
                                    span: Span::new(15, 21),
                                },
                                Expr {
                                    kind: ExprKind::IntLiteral(100),
                                    span: Span::new(23, 26),
                                },
                            ],
                        },
                        span: Span::new(10, 27),
                    },
                    span: Span::new(10, 27),
                })],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "OPEN(symbol, 100) should be valid: {:?}", result.err());
    }

    #[test]
    fn test_signal_open_wrong_args() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("OPEN".to_string()),
                                span: Span::new(10, 14),
                            }),
                            args: vec![
                                Expr {
                                    kind: ExprKind::IntLiteral(100),
                                    span: Span::new(15, 18),
                                },
                                Expr {
                                    kind: ExprKind::StringLiteral("hi".to_string()),
                                    span: Span::new(20, 24),
                                },
                            ],
                        },
                        span: Span::new(10, 25),
                    },
                    span: Span::new(10, 25),
                })],
                span: Span::new(0, 40),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("argument") || msg.contains("OPEN"), "got: {}", msg);
    }

    #[test]
    fn test_signal_close_one_arg() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("CLOSE".to_string()),
                                span: Span::new(10, 15),
                            }),
                            args: vec![Expr {
                                kind: ExprKind::Ident("symbol".to_string()),
                                span: Span::new(16, 22),
                            }],
                        },
                        span: Span::new(10, 23),
                    },
                    span: Span::new(10, 23),
                })],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "CLOSE(symbol) should be valid: {:?}", result.err());
    }

    #[test]
    fn test_signal_close_two_args() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("CLOSE".to_string()),
                                span: Span::new(10, 15),
                            }),
                            args: vec![
                                Expr {
                                    kind: ExprKind::Ident("symbol".to_string()),
                                    span: Span::new(16, 22),
                                },
                                Expr {
                                    kind: ExprKind::IntLiteral(50),
                                    span: Span::new(24, 26),
                                },
                            ],
                        },
                        span: Span::new(10, 27),
                    },
                    span: Span::new(10, 27),
                })],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "CLOSE(symbol, 50) should be valid: {:?}", result.err());
    }
}
