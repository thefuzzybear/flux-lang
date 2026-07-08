//! Abstract Syntax Tree (AST) node definitions for the Flux language.
//!
//! All AST nodes carry a `Span` for source location tracking.

use crate::lexer::Span;

/// Root AST node representing an entire Flux source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub imports: Vec<Import>,
    pub data_block: Option<DataBlock>,
    pub connector_block: Option<ConnectorBlock>,
    pub strategy: Strategy,
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
