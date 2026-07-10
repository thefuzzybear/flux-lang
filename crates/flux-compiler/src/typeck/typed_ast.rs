#![allow(dead_code)]
//! Typed Abstract Syntax Tree (AST) node definitions for the Flux language.
//!
//! These structures mirror the untyped parser AST exactly, with `TypedExpr`
//! replacing `Expr`. Every `TypedExpr` carries a `resolved_type: FluxType`
//! field and preserves the `span: Span`.

use crate::lexer::Span;
use crate::parser::ast::{BinOp, Import, UnaryOp};
use super::types::FluxType;

/// A validated decorator that has been resolved from a parsed `Decorator` AST node.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedDecorator {
    pub kind: DecoratorKind,
    pub span: Span,
}

/// The set of recognized decorator kinds in Flux.
#[derive(Debug, Clone, PartialEq)]
pub enum DecoratorKind {
    Stack,
    Heap,
    Aligned(u32),
    Packed,
    Prefetch,
    Streaming,
    Soa,
    Pool(u32),
    Hot,
    Cold,
    Volatile,
    Bitfield,
    Simd(u32),
    ZeroInit,
    Immutable,
}

/// A typed user-defined function declaration.
/// Mirrors `FnDef` from the parser AST with an inferred `return_type`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedFnDef {
    pub name: String,
    pub params: Vec<String>,
    pub param_types: Vec<FluxType>,
    pub body: Vec<TypedStmt>,
    pub return_type: FluxType,
    pub span: Span,
}

/// A typed struct field with its resolved FluxType.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStructField {
    pub name: String,
    pub resolved_type: FluxType,
    /// Bit width for `@bitfield` struct fields: `Some(1)` for bool, `Some(n)` for `int(n)`, `None` otherwise.
    pub bit_width: Option<usize>,
    /// Field-level decorator names (e.g. `"hot"`, `"cold"`), populated from the parsed AST.
    pub field_decorator_names: Vec<String>,
    pub span: Span,
}

/// A typed struct definition. Fields carry resolved types and the vec is ordered
/// by dependency (structs used as field types appear first in `TypedProgram.structs`).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedStructDef {
    pub name: String,
    pub fields: Vec<TypedStructField>,
    pub decorators: Vec<ValidatedDecorator>,
    pub span: Span,
}

/// Root typed AST node representing an entire Flux source file.
/// Mirrors `Program` from the parser AST.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedProgram {
    pub imports: Vec<Import>,
    /// Struct definitions in dependency-sorted order (referenced structs appear first).
    pub structs: Vec<TypedStructDef>,
    /// Enum definitions with resolved variant field types.
    pub enums: Vec<TypedEnumDef>,
    pub functions: Vec<TypedFnDef>,
    /// Impl blocks with typechecked method bodies.
    pub impl_blocks: Vec<TypedImplBlock>,
    pub data_block: Option<TypedDataBlock>,
    pub connector_block: Option<TypedConnectorBlock>,
    pub strategy: TypedStrategy,
    pub span: Span,
}

/// Typed data block — values have been validated by the typechecker.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedDataBlock {
    pub symbols: Option<Vec<String>>,
    pub period: Option<String>,
    pub interval: Option<String>,
    pub source: Option<String>,
    pub span: Span,
}

/// Typed connector block — values have been validated by the typechecker.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedConnectorBlock {
    pub connector_type: Option<String>,
    pub url: Option<String>,
    pub symbols: Option<Vec<String>>,
    pub interval: Option<String>,
    pub file: Option<String>,
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
    StructLiteral {
        struct_name: String,
        fields: Vec<(String, TypedExpr)>,
    },
    /// Enum variant construction: EnumName.Variant or EnumName.Variant(args)
    EnumConstruction {
        enum_name: String,
        variant_name: String,
        args: Vec<TypedExpr>,
    },
    /// Match expression
    Match(TypedMatchExpr),
}

// --- Enum Typed AST Nodes ---

/// A typed enum definition with resolved variant field types.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedEnumDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<TypedEnumVariant>,
    pub span: Span,
}

/// A typed enum variant with resolved field types.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedEnumVariant {
    pub name: String,
    pub fields: Vec<(String, FluxType)>,
    pub span: Span,
}

/// A typed match expression with resolved types.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedMatchExpr {
    pub scrutinee: Box<TypedExpr>,
    pub arms: Vec<TypedMatchArm>,
    pub result_type: FluxType,
    pub span: Span,
}

/// A typed match arm with pattern and body.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedMatchArm {
    pub pattern: TypedPattern,
    pub body: Vec<TypedStmt>,
    pub span: Span,
}

/// Typed patterns used in match arms.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedPattern {
    /// Variant pattern with bindings and their resolved types.
    Variant {
        enum_name: String,
        variant_name: String,
        bindings: Vec<(String, FluxType)>,
        span: Span,
    },
    /// Wildcard pattern `_`
    Wildcard { span: Span },
}

// --- Impl Block Typed AST Nodes ---

/// A typed impl block with typechecked method bodies.
///
/// Represents either an inherent impl (`impl StructName { ... }`) or a
/// trait impl (`impl TraitName for StructName { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedImplBlock {
    /// If present, this is a trait impl (e.g., `impl TraitName for StructName`).
    pub trait_name: Option<String>,
    /// The target struct type name.
    pub target_type: String,
    /// Typechecked method definitions.
    pub methods: Vec<TypedFnDef>,
    pub span: Span,
}
