//! Abstract Syntax Tree (AST) node definitions for the Flux language.
//!
//! All AST nodes carry a `Span` for source location tracking.

use crate::lexer::Span;

/// Root AST node representing an entire Flux source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub imports: Vec<Import>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub functions: Vec<FnDef>,
    pub impl_blocks: Vec<ImplBlock>,
    pub traits: Vec<TraitDef>,
    pub data_block: Option<DataBlock>,
    pub connector_block: Option<ConnectorBlock>,
    pub strategy: Strategy,
    pub span: Span,
}

/// A struct definition: `struct Name { field: Type, ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub fields: Vec<StructField>,
    pub decorators: Vec<Decorator>,
    pub span: Span,
}

/// A single field in a struct definition.
#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: String,
    pub field_type: TypeAnnotation,
    /// Field-level decorators, e.g. `@hot`/`@cold`.
    pub field_decorators: Vec<Decorator>,
    pub span: Span,
}

/// A decorator annotation: `@name` or `@name(arg)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Decorator {
    pub name: String,
    pub arg: Option<DecoratorArg>,
    pub span: Span,
}

/// A decorator argument value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecoratorArg {
    Int(i64),
}

/// Type annotations used in struct fields and function signatures.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeAnnotation {
    F64,
    Int,
    Bool,
    Str,
    /// Another struct type, referenced by name.
    Named(String),
    /// `[Type; N]`
    FixedArray(Box<TypeAnnotation>, usize),
    /// `int(N)` for `@bitfield` structs.
    BitInt(usize),
    /// Generic type usage: `Vec[T]`, `HashMap[K, V]`
    Generic(String, Vec<TypeAnnotation>),
}

/// A user-defined function declaration: `fn name(params) { body }`
///
/// Params may optionally carry a type annotation (e.g. `fn f(q: Quote) -> f64 { ... }`).
/// Untyped functions (`fn f(q) { ... }`) remain fully supported; their params simply
/// have `param_type: None` and `return_type: None`.
#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<FnParam>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// A single function parameter: `name` or `name: Type`.
#[derive(Debug, Clone, PartialEq)]
pub struct FnParam {
    pub name: String,
    pub param_type: Option<TypeAnnotation>,
    pub span: Span,
}

/// A data block declaration: `data { key = value ... }`
///
/// Declares data acquisition configuration for the strategy.
/// All fields are optional in the AST — validation of required
/// fields and value correctness is deferred to the typechecker.
#[derive(Debug, Clone, PartialEq)]
pub struct DataBlock {
    /// Ticker symbols to fetch: `symbols = ["AAPL", "MSFT"]`
    pub symbols: Option<DataField<Vec<String>>>,
    /// Time period: `period = "1y"`
    pub period: Option<DataField<String>>,
    /// Bar interval: `interval = "1d"`
    pub interval: Option<DataField<String>>,
    /// Data provider: `source = "yahoo"`
    pub source: Option<DataField<String>>,
    /// Span of the entire data block (from `data` keyword to closing `}`)
    pub span: Span,
}

/// A single field in the data block with its value and source span.
#[derive(Debug, Clone, PartialEq)]
pub struct DataField<T> {
    pub value: T,
    pub span: Span,
}

/// A connector block: declares live data source configuration.
///
/// connector {
///     type = "websocket"
///     url = "wss://stream.example.com/v1"
///     symbols = ["AAPL", "MSFT"]
///     interval = "1m"
/// }
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorBlock {
    /// Connector type: "websocket", "poll", "replay"
    pub connector_type: Option<DataField<String>>,
    /// Endpoint URL (for websocket and poll)
    pub url: Option<DataField<String>>,
    /// Symbols to subscribe to
    pub symbols: Option<DataField<Vec<String>>>,
    /// Bar aggregation interval
    pub interval: Option<DataField<String>>,
    /// File path (for replay connector)
    pub file: Option<DataField<String>>,
    /// Span of the entire connector block
    pub span: Span,
}

/// An import statement: `from module.path import {name1, name2}`
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub module_path: String,
    pub names: Vec<String>,
    pub span: Span,
}

/// A strategy declaration: `strategy Name { ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct Strategy {
    pub name: String,
    pub body: Vec<StrategyItem>,
    pub span: Span,
}

/// Items that can appear in a strategy body.
#[derive(Debug, Clone, PartialEq)]
pub enum StrategyItem {
    Property(Property),
    ParamsBlock(ParamsBlock),
    StateBlock(StateBlock),
    EventHandler(EventHandler),
}

/// A strategy property assignment: `name = expr`
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// A params block: `params { period = 20, threshold = 2.0 }`
#[derive(Debug, Clone, PartialEq)]
pub struct ParamsBlock {
    pub params: Vec<Param>,
    pub span: Span,
}

/// A single parameter declaration: `name = default_value`
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub default_value: Expr,
    pub span: Span,
}

/// A state block: `state { prices = [] }`
#[derive(Debug, Clone, PartialEq)]
pub struct StateBlock {
    pub variables: Vec<StateVar>,
    pub span: Span,
}

/// A single state variable declaration: `name = initial_value`
#[derive(Debug, Clone, PartialEq)]
pub struct StateVar {
    pub name: String,
    pub initial_value: Expr,
    pub span: Span,
}

/// An event handler block: `on_bar { ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct EventHandler {
    pub event_name: String,
    pub body: Vec<Stmt>,
    pub span: Span,
}

// --- Statements ---

/// Statements
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Assignment(Assignment),
    If(IfStmt),
    For(ForLoop),
    While(WhileLoop),
    Return(ReturnStmt),
    Expr(ExprStmt),
}

/// Assignment statement: `target = value`
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub target: Expr,
    pub value: Expr,
    pub span: Span,
}

/// If/elif/else statement
#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub condition: Expr,
    pub body: Vec<Stmt>,
    pub elif_branches: Vec<ElifBranch>,
    pub else_body: Option<Vec<Stmt>>,
    pub span: Span,
}

/// A single elif branch
#[derive(Debug, Clone, PartialEq)]
pub struct ElifBranch {
    pub condition: Expr,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// For loop: `for variable in iterable { body }`
#[derive(Debug, Clone, PartialEq)]
pub struct ForLoop {
    pub variable: String,
    pub iterable: Expr,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// While loop: `while condition { body }`
#[derive(Debug, Clone, PartialEq)]
pub struct WhileLoop {
    pub condition: Expr,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Return statement (optional value)
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStmt {
    pub value: Option<Expr>,
    pub span: Span,
}

/// Expression statement (wraps an expression used as a statement)
#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt {
    pub expr: Expr,
    pub span: Span,
}

// --- Expressions ---

/// Expressions
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

/// The kinds of expressions
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BoolLiteral(bool),
    NullLiteral,
    ListLiteral(Vec<Expr>),
    Ident(String),
    BinaryOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    FunctionCall {
        function: Box<Expr>,
        args: Vec<Expr>,
    },
    MethodCall {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    MemberAccess {
        object: Box<Expr>,
        field: String,
    },
    IndexAccess {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    /// A struct literal: `StructName { field = value, ... }`
    StructLiteral {
        struct_name: String,
        fields: Vec<(String, Expr)>,
    },
    /// Enum variant construction: EnumName.Variant or EnumName.Variant(args)
    EnumConstruction {
        enum_name: String,
        variant_name: String,
        args: Vec<Expr>,
    },
    /// Match expression
    Match(MatchExpr),
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

// --- Enum and Match AST Nodes ---

/// Enum definition: `enum Name { Variant1, Variant2(field: Type) }`
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

/// A single variant in an enum definition.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<EnumField>,
    pub span: Span,
}

/// A named field in an enum variant.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumField {
    pub name: String,
    pub field_type: TypeAnnotation,
    pub span: Span,
}

/// Match expression: `match expr { Pattern => body, ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct MatchExpr {
    pub scrutinee: Box<Expr>,
    pub arms: Vec<MatchArm>,
    pub span: Span,
}

/// A single arm in a match expression.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Patterns used in match arms.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// EnumName.VariantName or EnumName.VariantName(a, b, c)
    Variant {
        enum_name: String,
        variant_name: String,
        bindings: Vec<String>,
        span: Span,
    },
    /// _ wildcard
    Wildcard { span: Span },
}

/// Type parameter: `T` or `T: TraitBound`
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub bound: Option<String>,
    pub span: Span,
}

/// An impl block: `impl StructName { fn method(self, ...) { ... } }`
/// or `impl TraitName for StructName { ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    /// If present, this is a trait impl (e.g., `impl TraitName for StructName`)
    pub trait_name: Option<String>,
    /// The target struct type name
    pub target_type: String,
    /// Type parameters (for future generic impls)
    pub type_params: Vec<TypeParam>,
    /// Methods defined in this impl block
    pub methods: Vec<FnDef>,
    pub span: Span,
}

/// A trait definition: `trait Name { fn method(self, ...) -> Type }`
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
    pub name: String,
    pub methods: Vec<TraitMethodSig>,
    pub span: Span,
}

/// A method signature within a trait definition (no body).
#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethodSig {
    pub name: String,
    pub params: Vec<FnParam>,
    pub return_type: Option<TypeAnnotation>,
    pub span: Span,
}

// --- Account Manifest AST Nodes ---

/// Root AST node for an account manifest file (account.flux).
/// Separate from Program — manifest files have no strategy, no functions.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestProgram {
    pub blocks: Vec<ManifestBlock>,
    pub span: Span,
}

/// A single top-level manifest block.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestBlock {
    pub kind: ManifestBlockKind,
    pub span: Span,
}

/// The different manifest block types.
#[derive(Debug, Clone, PartialEq)]
pub enum ManifestBlockKind {
    Account(Vec<ManifestField>),
    Gateway(Vec<ManifestField>),
    Data(Vec<ManifestField>),
    Database(Vec<ManifestField>),
    Risk(Vec<ManifestField>),
    Products(Vec<ManifestEntry>),
    Strategies(Vec<ManifestEntry>),
}

/// A key = value field inside a simple block (account, gateway, etc.).
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestField {
    pub name: String,
    pub value: ManifestValue,
    pub span: Span,
}

/// A named entry in a products/strategies block: `KEY = { fields... }`
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestEntry {
    pub name: String,
    pub fields: Vec<ManifestField>,
    pub span: Span,
}

/// Possible values in manifest fields.
#[derive(Debug, Clone, PartialEq)]
pub enum ManifestValue {
    String(String),
    Int(i64),
    Float(f64),
    StringList(Vec<String>),
    EnvCall(String),
}
