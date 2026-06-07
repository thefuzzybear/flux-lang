#![allow(dead_code)]
//! Typed Abstract Syntax Tree (AST) node definitions for the Flux language.
//!
//! These structures mirror the untyped parser AST exactly, with `TypedExpr`
//! replacing `Expr`. Every `TypedExpr` carries a `resolved_type: FluxType`
//! field and preserves the `span: Span`.

use crate::lexer::Span;
use crate::parser::ast::{BinOp, Import, UnaryOp};
use super::types::FluxType;

/// Root typed AST node representing an entire Flux source file.
/// Mirrors `Program` from the parser AST.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedProgram {
    pub imports: Vec<Import>,
    pub strategy: TypedStrategy,
    pub span: Span,
}

/// A typed strategy declaration. Mirrors `Strategy`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStrategy {
    pub name: String,
    pub body: Vec<TypedStrategyItem>,
    pub span: Span,
}

/// Typed items that can appear in a strategy body. Mirrors `StrategyItem`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedStrategyItem {
    Property(TypedProperty),
    ParamsBlock(TypedParamsBlock),
    StateBlock(TypedStateBlock),
    EventHandler(TypedEventHandler),
}

/// A typed strategy property assignment. Mirrors `Property`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedProperty {
    pub name: String,
    pub value: TypedExpr,
    pub span: Span,
}

/// A typed params block. Mirrors `ParamsBlock`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedParamsBlock {
    pub params: Vec<TypedParam>,
    pub span: Span,
}

/// A typed parameter declaration. Mirrors `Param` with an additional
/// `resolved_type` field for the inferred type from the default value.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedParam {
    pub name: String,
    pub default_value: TypedExpr,
    pub resolved_type: FluxType,
    pub span: Span,
}

/// A typed state block. Mirrors `StateBlock`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStateBlock {
    pub variables: Vec<TypedStateVar>,
    pub span: Span,
}

/// A typed state variable declaration. Mirrors `StateVar` with an additional
/// `resolved_type` field for the inferred type from the initial value.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStateVar {
    pub name: String,
    pub initial_value: TypedExpr,
    pub resolved_type: FluxType,
    pub span: Span,
}

/// A typed event handler block. Mirrors `EventHandler`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedEventHandler {
    pub event_name: String,
    pub body: Vec<TypedStmt>,
    pub span: Span,
}

// --- Statements ---

/// Typed statements. Mirrors `Stmt`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedStmt {
    Assignment(TypedAssignment),
    If(TypedIfStmt),
    For(TypedForLoop),
    While(TypedWhileLoop),
    Return(TypedReturnStmt),
    Expr(TypedExprStmt),
}

/// Typed assignment statement. Mirrors `Assignment`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedAssignment {
    pub target: TypedExpr,
    pub value: TypedExpr,
    pub span: Span,
}

/// Typed if/elif/else statement. Mirrors `IfStmt`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedIfStmt {
    pub condition: TypedExpr,
    pub body: Vec<TypedStmt>,
    pub elif_branches: Vec<TypedElifBranch>,
    pub else_body: Option<Vec<TypedStmt>>,
    pub span: Span,
}

/// A typed elif branch. Mirrors `ElifBranch`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedElifBranch {
    pub condition: TypedExpr,
    pub body: Vec<TypedStmt>,
    pub span: Span,
}

/// Typed for loop. Mirrors `ForLoop` with an additional `variable_type` field
/// for the resolved type of the loop variable (element type of the iterable).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedForLoop {
    pub variable: String,
    pub variable_type: FluxType,
    pub iterable: TypedExpr,
    pub body: Vec<TypedStmt>,
    pub span: Span,
}

/// Typed while loop. Mirrors `WhileLoop`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedWhileLoop {
    pub condition: TypedExpr,
    pub body: Vec<TypedStmt>,
    pub span: Span,
}

/// Typed return statement. Mirrors `ReturnStmt`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedReturnStmt {
    pub value: Option<TypedExpr>,
    pub span: Span,
}

/// Typed expression statement. Mirrors `ExprStmt`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedExprStmt {
    pub expr: TypedExpr,
    pub span: Span,
}

// --- Expressions ---

/// The core typed expression node. Every expression carries its resolved type.
/// Mirrors `Expr` with an additional `resolved_type` field.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub resolved_type: FluxType,
    pub span: Span,
}

/// Typed expression kinds. Mirrors `ExprKind`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedExprKind {
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BoolLiteral(bool),
    NullLiteral,
    ListLiteral(Vec<TypedExpr>),
    Ident(String),
    BinaryOp {
        left: Box<TypedExpr>,
        op: BinOp,
        right: Box<TypedExpr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<TypedExpr>,
    },
    FunctionCall {
        function: Box<TypedExpr>,
        args: Vec<TypedExpr>,
    },
    MethodCall {
        receiver: Box<TypedExpr>,
        method: String,
        args: Vec<TypedExpr>,
    },
    MemberAccess {
        object: Box<TypedExpr>,
        field: String,
    },
    IndexAccess {
        object: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },
}
