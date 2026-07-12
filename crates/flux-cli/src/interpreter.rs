use std::collections::HashMap;
use std::fmt;

use flux_compiler::parser::ast::{BinOp, UnaryOp};
use flux_compiler::typeck::typed_ast::*;
use flux_compiler::typeck::types::FluxType;
use flux_runtime::{BarContext, Signal};

/// Runtime value representation for the AST interpreter.
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
    List(Vec<Value>),
    Signal(Signal),
    /// A one-dimensional array of floats (e.g., asset weights, return vectors).
    VecFloat(Vec<f64>),
    /// A two-dimensional matrix of floats stored in row-major order.
    /// Element (i, j) is at index `i * cols + j` in `data`.
    MatFloat { data: Vec<f64>, rows: usize, cols: usize },
    /// A struct instance with a type name and named fields.
    Struct { type_name: String, fields: HashMap<String, Value> },
    /// An enum value with enum name, variant name, and named field values.
    Enum { enum_name: String, variant_name: String, fields: Vec<(String, Value)> },
    /// A HashMap (key-value associative container).
    HashMap(HashMap<String, Value>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(v) => write!(f, "{}", v),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Null => write!(f, "null"),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Signal(sig) => write!(f, "{:?}", sig),
            Value::VecFloat(v) => {
                write!(f, "[")?;
                for (i, val) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", val)?;
                }
                write!(f, "]")
            }
            Value::MatFloat { data, rows, cols } => {
                write!(f, "Matrix({}x{}, {:?})", rows, cols, data)
            }
            Value::Struct { type_name, fields } => {
                write!(f, "{} {{ ", type_name)?;
                let mut entries: Vec<_> = fields.iter().collect();
                entries.sort_by_key(|(k, _)| *k);
                for (i, (name, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, value)?;
                }
                write!(f, " }}")
            }
            Value::Enum { enum_name, variant_name, fields } => {
                if fields.is_empty() {
                    write!(f, "{}.{}", enum_name, variant_name)
                } else {
                    write!(f, "{}.{}(", enum_name, variant_name)?;
                    for (i, (name, value)) in fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", name, value)?;
                    }
                    write!(f, ")")
                }
            }
            Value::HashMap(map) => {
                write!(f, "HashMap {{")?;
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by_key(|(k, _)| *k);
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, " {}: {}", key, value)?;
                }
                if !entries.is_empty() {
                    write!(f, " ")?;
                }
                write!(f, "}}")
            }
        }
    }
}

/// State entry for interpreter-managed indicator computations.
///
/// Each distinct `sma()` or `ema()` call-site (keyed by AST span) gets its own
/// independent state buffer, avoiding the `#[track_caller]` collision in the runtime.
#[derive(Debug, Clone)]
pub enum IndicatorStateEntry {
    Sma {
        buffer: Vec<f64>,
        period: usize,
        index: usize,
        count: usize,
        sum: f64,
    },
    Ema {
        prev_ema: Option<f64>,
        k: f64,
    },
    /// Rolling state for stddev, variance, and zscore computations.
    /// Maintains a circular buffer with running sum and sum-of-squares
    /// for O(1) incremental updates.
    RollingStats {
        buffer: Vec<f64>,
        period: usize,
        index: usize,
        count: usize,
        sum: f64,
        sum_sq: f64,
    },
    /// Rolling state for corr and covariance computations.
    /// Maintains paired circular buffers for two series.
    RollingPair {
        buffer_a: Vec<f64>,
        buffer_b: Vec<f64>,
        period: usize,
        index: usize,
        count: usize,
    },
    /// Rolling state for RSI (Relative Strength Index) using Wilder's smoothing.
    /// Tracks the previous value and exponentially smoothed average gain/loss.
    Rsi {
        prev_value: Option<f64>,
        avg_gain: f64,
        avg_loss: f64,
        period: usize,
        count: usize,
    },
    /// Rolling state for ATR (Average True Range) using Wilder's smoothing.
    /// Tracks previous close for True Range calculation and the smoothed ATR value.
    Atr {
        prev_close: Option<f64>,
        atr_value: Option<f64>,
        period: usize,
        count: usize,
    },
    /// Rolling state for cov_matrix and corr_matrix computations.
    /// Maintains a circular buffer of return vectors (one per bar).
    RollingMatrix {
        window: Vec<Vec<f64>>,
        period: usize,
        index: usize,
        count: usize,
        n_assets: usize,
    },
}

/// A lightweight AST interpreter that walks the TypedProgram and evaluates
/// expressions against BarContext values bar-by-bar.
pub struct Interpreter {
    pub params: HashMap<String, Value>,
    pub state: HashMap<String, Value>,
    pub event_handler: Option<TypedEventHandler>,
    pub indicators: HashMap<String, IndicatorStateEntry>,
    pub in_position: bool,
    /// Previous bar group's close prices per symbol (for ret() computation).
    pub prev_closes: HashMap<String, f64>,
    /// Current bar group's close prices per symbol.
    pub current_closes: HashMap<String, f64>,
    /// Registry of user-defined functions, keyed by function name.
    pub functions: HashMap<String, TypedFnDef>,
    /// Registry of enum definitions, keyed by enum name.
    /// Each entry maps to its list of typed variants.
    pub enum_defs: HashMap<String, Vec<TypedEnumVariant>>,
    /// Current call stack depth (for detecting stack overflow).
    pub call_depth: usize,
    /// Maximum allowed call stack depth (default 64).
    pub max_call_depth: usize,
    /// Signals emitted by the most recent user-function call.
    /// Drained by the caller after each function call expression.
    pub fn_signals: Vec<Signal>,
    /// Holds the return value from a `return` statement in a nested block.
    /// Used to propagate struct (and other complex) values through the
    /// sentinel error mechanism without lossy string encoding.
    pub pending_return: Option<Value>,
    /// Registry of impl block methods, keyed by type name → method name → TypedFnDef.
    /// Used to resolve method calls on struct instances.
    pub impl_methods: HashMap<String, HashMap<String, TypedFnDef>>,
}

impl Interpreter {
    /// Create a new Interpreter from a TypedProgram.
    ///
    /// Initializes params from `TypedParamsBlock` defaults, state from
    /// `TypedStateBlock` initial values, and stores the `on_bar` handler.
    pub fn new(program: &TypedProgram) -> Self {
        let mut params = HashMap::new();
        let mut state = HashMap::new();
        let mut event_handler = None;

        for item in &program.strategy.body {
            match item {
                TypedStrategyItem::ParamsBlock(pb) => {
                    for param in &pb.params {
                        let value = eval_literal(&param.default_value);
                        params.insert(param.name.clone(), value);
                    }
                }
                TypedStrategyItem::StateBlock(sb) => {
                    for var in &sb.variables {
                        let value = eval_literal(&var.initial_value);
                        state.insert(var.name.clone(), value);
                    }
                }
                TypedStrategyItem::EventHandler(eh) => {
                    if eh.event_name == "bar" {
                        event_handler = Some(eh.clone());
                    }
                }
                TypedStrategyItem::Property(_) => {
                    // Properties are metadata; not needed at runtime
                }
            }
        }

        let mut functions = HashMap::new();
        for fn_def in &program.functions {
            functions.insert(fn_def.name.clone(), fn_def.clone());
        }

        let mut enum_defs = HashMap::new();
        for enum_def in &program.enums {
            enum_defs.insert(enum_def.name.clone(), enum_def.variants.clone());
        }

        // Build impl method registry from typed impl blocks.
        // Inherent impl methods are registered first (they appear before trait impl
        // blocks in the typed program). For trait impl methods, we only insert if no
        // inherent method with the same name exists — this ensures inherent methods
        // take priority over trait methods during dispatch (Requirement 6.6).
        let mut impl_methods: HashMap<String, HashMap<String, TypedFnDef>> = HashMap::new();
        for impl_block in &program.impl_blocks {
            let type_methods = impl_methods
                .entry(impl_block.target_type.clone())
                .or_default();
            for method in &impl_block.methods {
                if impl_block.trait_name.is_some() {
                    // Trait impl method: only register if no inherent method exists
                    type_methods
                        .entry(method.name.clone())
                        .or_insert_with(|| method.clone());
                } else {
                    // Inherent impl method: always registers (takes priority)
                    type_methods.insert(method.name.clone(), method.clone());
                }
            }
        }

        Interpreter {
            params,
            state,
            event_handler,
            indicators: HashMap::new(),
            in_position: false,
            prev_closes: HashMap::new(),
            current_closes: HashMap::new(),
            functions,
            enum_defs,
            call_depth: 0,
            max_call_depth: 64,
            fn_signals: Vec::new(),
            pending_return: None,
            impl_methods,
        }
    }

    /// Update per-symbol price state for a new bar group.
    ///
    /// Copies `current_closes` into `prev_closes`, then sets `current_closes`
    /// to the new bar group's close prices.
    pub fn update_prices(&mut self, new_closes: &HashMap<String, f64>) {
        self.prev_closes = self.current_closes.clone();
        self.current_closes = new_closes.clone();
    }

    /// Execute the `on_bar` event handler against the given bar context.
    ///
    /// Binds bar context fields as local variables, executes the event handler body,
    /// and collects all emitted signals.
    pub fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
        let mut locals = HashMap::new();
        locals.insert("close".to_string(), Value::Float(ctx.close));
        locals.insert("open".to_string(), Value::Float(ctx.open));
        locals.insert("high".to_string(), Value::Float(ctx.high));
        locals.insert("low".to_string(), Value::Float(ctx.low));
        locals.insert("volume".to_string(), Value::Float(ctx.volume));
        locals.insert("symbol".to_string(), Value::Str(ctx.symbol.clone()));
        locals.insert("in_position".to_string(), Value::Bool(self.in_position));

        if let Some(handler) = &self.event_handler {
            let body = handler.body.clone();
            match self.exec_stmts(&body, &mut locals) {
                Ok(signals) => {
                    // Update in_position based on emitted signals, processing in order
                    // so the final state reflects the last relevant signal on this bar.
                    for signal in &signals {
                        match signal {
                            Signal::Open { .. } => self.in_position = true,
                            Signal::Close { .. } => self.in_position = false,
                            Signal::CloseQty { .. } => {} // partial close does not flatten
                        }
                    }
                    signals
                }
                Err(msg) if msg == "__RETURN__" => {
                    // A return statement in the event handler — treat as early exit.
                    // Any signals emitted before the return are in fn_signals.
                    // Clear the pending_return since the event handler doesn't use it.
                    self.pending_return = None;
                    let signals: Vec<Signal> = self.fn_signals.drain(..).collect();
                    for signal in &signals {
                        match signal {
                            Signal::Open { .. } => self.in_position = true,
                            Signal::Close { .. } => self.in_position = false,
                            Signal::CloseQty { .. } => {}
                        }
                    }
                    signals
                }
                Err(msg) => {
                    eprintln!("warning: runtime error in on_bar handler: {}", msg);
                    vec![]
                }
            }
        } else {
            vec![]
        }
    }

    /// Evaluate a typed expression, resolving identifiers from locals, params, then state.
    pub fn eval_expr(
        &mut self,
        expr: &TypedExpr,
        locals: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        match &expr.kind {
            // --- Literals ---
            TypedExprKind::IntLiteral(i) => Ok(Value::Int(*i)),
            TypedExprKind::FloatLiteral(f) => Ok(Value::Float(*f)),
            TypedExprKind::StringLiteral(s) => Ok(Value::Str(s.clone())),
            TypedExprKind::BoolLiteral(b) => Ok(Value::Bool(*b)),
            TypedExprKind::NullLiteral => Ok(Value::Null),
            TypedExprKind::ListLiteral(items) => {
                if expr.resolved_type == FluxType::VecFloat {
                    let values: Result<Vec<f64>, String> = items
                        .iter()
                        .map(|item| {
                            let val = self.eval_expr(item, locals)?;
                            match val {
                                Value::Float(f) => Ok(f),
                                Value::Int(i) => Ok(i as f64),
                                _ => Err("expected a numeric value in VecFloat literal".to_string()),
                            }
                        })
                        .collect();
                    Ok(Value::VecFloat(values?))
                } else {
                    let values: Result<Vec<Value>, String> =
                        items.iter().map(|item| self.eval_expr(item, locals)).collect();
                    Ok(Value::List(values?))
                }
            }

            // --- Identifier lookup: locals → params → state ---
            TypedExprKind::Ident(name) => {
                if let Some(val) = locals.get(name) {
                    Ok(val.clone())
                } else if let Some(val) = self.params.get(name) {
                    Ok(val.clone())
                } else if let Some(val) = self.state.get(name) {
                    Ok(val.clone())
                } else {
                    Err(format!("undefined variable: '{}'", name))
                }
            }

            // --- Binary operations ---
            TypedExprKind::BinaryOp { left, op, right } => {
                // Short-circuit for logical operators
                if *op == BinOp::And {
                    let left_val = self.eval_expr(left, locals)?;
                    match left_val {
                        Value::Bool(false) => return Ok(Value::Bool(false)),
                        Value::Bool(true) => {
                            let right_val = self.eval_expr(right, locals)?;
                            match right_val {
                                Value::Bool(b) => return Ok(Value::Bool(b)),
                                _ => return Err("logical AND requires boolean operands".to_string()),
                            }
                        }
                        _ => return Err("logical AND requires boolean operands".to_string()),
                    }
                }
                if *op == BinOp::Or {
                    let left_val = self.eval_expr(left, locals)?;
                    match left_val {
                        Value::Bool(true) => return Ok(Value::Bool(true)),
                        Value::Bool(false) => {
                            let right_val = self.eval_expr(right, locals)?;
                            match right_val {
                                Value::Bool(b) => return Ok(Value::Bool(b)),
                                _ => return Err("logical OR requires boolean operands".to_string()),
                            }
                        }
                        _ => return Err("logical OR requires boolean operands".to_string()),
                    }
                }

                let left_val = self.eval_expr(left, locals)?;
                let right_val = self.eval_expr(right, locals)?;

                match op {
                    // Arithmetic
                    BinOp::Add => eval_arith(&left_val, &right_val, "+", |a, b| a + b, |a, b| a + b),
                    BinOp::Sub => eval_arith(&left_val, &right_val, "-", |a, b| a - b, |a, b| a - b),
                    BinOp::Mul => eval_arith(&left_val, &right_val, "*", |a, b| a * b, |a, b| a * b),
                    BinOp::Div => {
                        // Check for division by zero
                        match (&left_val, &right_val) {
                            (Value::Int(_), Value::Int(0)) => {
                                Err("division by zero".to_string())
                            }
                            (Value::Int(_), Value::Float(f)) if *f == 0.0 => {
                                Err("division by zero".to_string())
                            }
                            (Value::Float(_), Value::Int(0)) => {
                                Err("division by zero".to_string())
                            }
                            (Value::Float(_), Value::Float(f)) if *f == 0.0 => {
                                Err("division by zero".to_string())
                            }
                            _ => eval_arith(&left_val, &right_val, "/", |a, b| a / b, |a, b| a / b),
                        }
                    }
                    BinOp::Mod => {
                        match (&left_val, &right_val) {
                            (Value::Int(_), Value::Int(0)) => {
                                Err("division by zero".to_string())
                            }
                            _ => eval_arith(&left_val, &right_val, "%", |a, b| a % b, |a, b| a % b),
                        }
                    }

                    // Comparisons
                    BinOp::Gt => eval_cmp(&left_val, &right_val, ">", |a, b| a > b, |a, b| a > b),
                    BinOp::Lt => eval_cmp(&left_val, &right_val, "<", |a, b| a < b, |a, b| a < b),
                    BinOp::Ge => eval_cmp(&left_val, &right_val, ">=", |a, b| a >= b, |a, b| a >= b),
                    BinOp::Le => eval_cmp(&left_val, &right_val, "<=", |a, b| a <= b, |a, b| a <= b),
                    BinOp::Eq => eval_eq(&left_val, &right_val),
                    BinOp::Ne => eval_eq(&left_val, &right_val).map(|v| match v {
                        Value::Bool(b) => Value::Bool(!b),
                        other => other,
                    }),

                    // Logical (already handled above via short-circuit)
                    BinOp::And | BinOp::Or => unreachable!(),
                }
            }

            // --- Unary operations ---
            TypedExprKind::UnaryOp { op, operand } => {
                let val = self.eval_expr(operand, locals)?;
                match op {
                    UnaryOp::Neg => match val {
                        Value::Int(i) => Ok(Value::Int(-i)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => Err("negation requires a numeric value".to_string()),
                    },
                    UnaryOp::Not => match val {
                        Value::Bool(b) => Ok(Value::Bool(!b)),
                        _ => Err("logical not requires a boolean value".to_string()),
                    },
                }
            }

            // --- Function calls ---
            TypedExprKind::FunctionCall { function, args } => {
                // Get the function name from the function expression
                let func_name = match &function.kind {
                    TypedExprKind::Ident(name) => name.clone(),
                    _ => return Err("unsupported function expression".to_string()),
                };

                // Construct a unique call-site key from the expression span
                let call_site_key = format!("{}_{}_{}", func_name, expr.span.start, expr.span.end);

                // Evaluate all arguments eagerly for tier dispatch
                let evaluated_args: Vec<Value> = args
                    .iter()
                    .map(|a| self.eval_expr(a, locals))
                    .collect::<Result<Vec<_>, _>>()?;

                // Try Tier 1 — stateless math builtins
                if let Some(result) = crate::math_builtins::eval_math_builtin(&func_name, &evaluated_args)? {
                    return Ok(result);
                }

                // Try Tier 2 — stateful rolling indicators
                if let Some(result) = crate::stat_indicators::eval_stat_indicator(
                    &func_name,
                    &evaluated_args,
                    &mut self.indicators,
                    &call_site_key,
                )? {
                    return Ok(result);
                }

                // Try Tier 3 — stateless matrix operations
                if let Some(result) = crate::portfolio_ops::eval_matrix_op(&func_name, &evaluated_args)? {
                    return Ok(result);
                }

                // Try Tier 3 — stateful portfolio operations (cov_matrix, corr_matrix, etc.)
                if let Some(result) = crate::portfolio_ops::eval_portfolio_op(
                    &func_name,
                    &evaluated_args,
                    &mut self.indicators,
                    &call_site_key,
                )? {
                    return Ok(result);
                }

                // Existing built-in functions: OPEN, CLOSE, CLOSE_QTY, sma, ema
                match func_name.as_str() {
                    "OPEN" => {
                        if evaluated_args.len() != 2 {
                            return Err("OPEN requires 2 arguments (symbol, qty)".to_string());
                        }
                        let symbol = match &evaluated_args[0] {
                            Value::Str(s) if s.is_empty() => {
                                return Err("OPEN: invalid symbol (empty string)".to_string());
                            }
                            Value::Str(s) => s.clone(),
                            _ => return Err("expected a string value".to_string()),
                        };
                        let qty = match &evaluated_args[1] {
                            Value::Float(f) => *f,
                            Value::Int(i) => *i as f64,
                            _ => return Err("expected a numeric value".to_string()),
                        };
                        Ok(Value::Signal(Signal::open(symbol, qty)))
                    }
                    "CLOSE" => {
                        if evaluated_args.len() != 1 {
                            return Err("CLOSE requires 1 argument (symbol)".to_string());
                        }
                        let symbol = match &evaluated_args[0] {
                            Value::Str(s) if s.is_empty() => {
                                return Err("CLOSE: invalid symbol (empty string)".to_string());
                            }
                            Value::Str(s) => s.clone(),
                            _ => return Err("expected a string value".to_string()),
                        };
                        Ok(Value::Signal(Signal::close(symbol)))
                    }
                    "CLOSE_QTY" => {
                        if evaluated_args.len() != 2 {
                            return Err("CLOSE_QTY requires 2 arguments (symbol, qty)".to_string());
                        }
                        let symbol = match &evaluated_args[0] {
                            Value::Str(s) if s.is_empty() => {
                                return Err("CLOSE_QTY: invalid symbol (empty string)".to_string());
                            }
                            Value::Str(s) => s.clone(),
                            _ => return Err("expected a string value".to_string()),
                        };
                        let qty = match &evaluated_args[1] {
                            Value::Float(f) => *f,
                            Value::Int(i) => *i as f64,
                            _ => return Err("expected a numeric value".to_string()),
                        };
                        Ok(Value::Signal(Signal::close_qty(symbol, qty)))
                    }
                    "sma" => {
                        if evaluated_args.len() != 2 {
                            return Err("sma requires 2 arguments (value, period)".to_string());
                        }
                        let value = match &evaluated_args[0] {
                            Value::Float(f) => *f,
                            Value::Int(i) => *i as f64,
                            _ => return Err("expected a numeric value".to_string()),
                        };
                        let period = match &evaluated_args[1] {
                            Value::Int(i) => *i as usize,
                            _ => return Err("expected an integer value".to_string()),
                        };
                        let key = format!("sma_{}_{}", expr.span.start, expr.span.end);

                        let entry = self.indicators.entry(key).or_insert_with(|| {
                            IndicatorStateEntry::Sma {
                                buffer: vec![0.0; period],
                                period,
                                index: 0,
                                count: 0,
                                sum: 0.0,
                            }
                        });

                        let result = match entry {
                            IndicatorStateEntry::Sma {
                                buffer,
                                period: p,
                                index,
                                count,
                                sum,
                            } => {
                                if *count < *p {
                                    buffer[*index] = value;
                                    *sum += value;
                                    *count += 1;
                                    *index = (*index + 1) % *p;
                                    *sum / *count as f64
                                } else {
                                    *sum -= buffer[*index];
                                    buffer[*index] = value;
                                    *sum += value;
                                    *index = (*index + 1) % *p;
                                    *sum / *p as f64
                                }
                            }
                            _ => return Err("indicator state mismatch for sma".to_string()),
                        };

                        Ok(Value::Float(result))
                    }
                    "ema" => {
                        if evaluated_args.len() != 2 {
                            return Err("ema requires 2 arguments (value, period)".to_string());
                        }
                        let value = match &evaluated_args[0] {
                            Value::Float(f) => *f,
                            Value::Int(i) => *i as f64,
                            _ => return Err("expected a numeric value".to_string()),
                        };
                        let period = match &evaluated_args[1] {
                            Value::Int(i) => *i as usize,
                            _ => return Err("expected an integer value".to_string()),
                        };
                        let key = format!("ema_{}_{}", expr.span.start, expr.span.end);

                        let entry = self.indicators.entry(key).or_insert_with(|| {
                            IndicatorStateEntry::Ema {
                                prev_ema: None,
                                k: 2.0 / (period as f64 + 1.0),
                            }
                        });

                        let result = match entry {
                            IndicatorStateEntry::Ema { prev_ema, k } => {
                                let ema = match *prev_ema {
                                    None => value,
                                    Some(prev) => value * *k + prev * (1.0 - *k),
                                };
                                *prev_ema = Some(ema);
                                ema
                            }
                            _ => return Err("indicator state mismatch for ema".to_string()),
                        };

                        Ok(Value::Float(result))
                    }
                    "ret" => {
                        if evaluated_args.len() != 1 {
                            return Err("ret requires 1 argument (symbol)".to_string());
                        }
                        let symbol = match &evaluated_args[0] {
                            Value::Str(s) => s.clone(),
                            _ => return Err("ret: expected a string argument".to_string()),
                        };
                        let current_close = self.current_closes.get(&symbol).copied();
                        let prev_close = self.prev_closes.get(&symbol).copied();
                        match (current_close, prev_close) {
                            (Some(curr), Some(prev)) if prev != 0.0 => {
                                Ok(Value::Float((curr / prev) - 1.0))
                            }
                            _ => Ok(Value::Float(0.0)),
                        }
                    }
                    _ => {
                        // Try user-defined functions
                        if let Some(fn_def) = self.functions.get(&func_name).cloned() {
                            if self.call_depth >= self.max_call_depth {
                                return Err(format!(
                                    "stack overflow: maximum call depth ({}) exceeded",
                                    self.max_call_depth
                                ));
                            }
                            self.call_depth += 1;

                            // Create new scope with parameter bindings
                            let mut fn_locals = HashMap::new();
                            for (param_name, arg_value) in fn_def.params.iter().zip(evaluated_args.iter()) {
                                fn_locals.insert(param_name.clone(), arg_value.clone());
                            }
                            // Inject bar context from caller's locals
                            for name in &["close", "open", "high", "low", "volume", "symbol", "in_position"] {
                                if let Some(val) = locals.get(*name) {
                                    fn_locals.insert(name.to_string(), val.clone());
                                }
                            }

                            let result = self.exec_fn_body(&fn_def.body, &mut fn_locals);
                            self.call_depth -= 1;
                            result
                        } else {
                            Err(format!("unknown function: '{}'", func_name))
                        }
                    }
                }
            }

            // --- Member access ---
            TypedExprKind::MemberAccess { object, field } => {
                let obj_val = self.eval_expr(object, locals)?;
                match obj_val {
                    Value::Struct { type_name: ref tn, fields: ref field_map } => {
                        field_map.get(field).cloned().ok_or_else(|| {
                            format!("runtime error: struct '{}' has no field '{}'", tn, field)
                        })
                    }
                    _ => Err(format!("member access requires a struct value, got {}", obj_val)),
                }
            }

            // --- Method call ---
            TypedExprKind::MethodCall { receiver, method, args } => {
                // Handle HashMap.new() static constructor before evaluating receiver
                // (since "HashMap" is not a runtime value, it cannot be looked up as a variable)
                if method == "new" {
                    if let TypedExprKind::Ident(ref name) = receiver.kind {
                        if name == "HashMap" {
                            return Ok(Value::HashMap(HashMap::new()));
                        }
                    }
                }

                // General static method dispatch: check if receiver is a type name
                // that matches a key in impl_methods and the method's first param is NOT `self`
                if let TypedExprKind::Ident(ref name) = receiver.kind {
                    if let Some(method_def) = self
                        .impl_methods
                        .get(name)
                        .and_then(|methods| methods.get(method))
                        .cloned()
                    {
                        // Check that the first param is NOT "self" (static method)
                        let is_static = method_def.params.first().map_or(true, |p| p != "self");
                        if is_static {
                            if self.call_depth >= self.max_call_depth {
                                return Err(format!(
                                    "stack overflow: maximum call depth ({}) exceeded",
                                    self.max_call_depth
                                ));
                            }
                            self.call_depth += 1;

                            // Evaluate arguments
                            let evaluated_args: Vec<Value> = args
                                .iter()
                                .map(|a| self.eval_expr(a, locals))
                                .collect::<Result<Vec<_>, _>>()?;

                            // Bind arguments to parameters
                            let mut fn_locals = HashMap::new();
                            for (param_name, arg_value) in method_def.params.iter().zip(evaluated_args.iter()) {
                                fn_locals.insert(param_name.clone(), arg_value.clone());
                            }
                            // Inject bar context from caller's locals
                            for ctx_name in &["close", "open", "high", "low", "volume", "symbol", "in_position"] {
                                if let Some(val) = locals.get(*ctx_name) {
                                    fn_locals.insert(ctx_name.to_string(), val.clone());
                                }
                            }

                            let result = self.exec_fn_body(&method_def.body, &mut fn_locals);
                            self.call_depth -= 1;
                            return result;
                        }
                    } else if self.impl_methods.contains_key(name) {
                        // Type exists but method doesn't — return descriptive error
                        // (only if this name is NOT resolvable as a variable)
                        let is_variable = locals.contains_key(name)
                            || self.params.contains_key(name)
                            || self.state.contains_key(name);
                        if !is_variable {
                            return Err(format!(
                                "No static method '{}' on type '{}'", method, name
                            ));
                        }
                    }
                }

                let receiver_val = self.eval_expr(receiver, locals)?;

                // Evaluate arguments eagerly
                let evaluated_args: Vec<Value> = args
                    .iter()
                    .map(|a| self.eval_expr(a, locals))
                    .collect::<Result<Vec<_>, _>>()?;

                // Handle built-in list methods
                match (&receiver_val, method.as_str()) {
                    (Value::List(_), "len") => {
                        if let Value::List(items) = &receiver_val {
                            return Ok(Value::Int(items.len() as i64));
                        }
                    }
                    (Value::List(_), "pop") => {
                        // pop returns the last element (non-mutating in this interpreter)
                        if let Value::List(items) = &receiver_val {
                            return items.last().cloned().ok_or_else(|| {
                                "runtime error: pop on empty list".to_string()
                            });
                        }
                    }
                    (Value::Str(_), "len") => {
                        if let Value::Str(s) = &receiver_val {
                            return Ok(Value::Int(s.len() as i64));
                        }
                    }
                    _ => {}
                }

                // Handle built-in HashMap methods
                if let Value::HashMap(ref map) = receiver_val {
                    match method.as_str() {
                        "insert" => {
                            if evaluated_args.len() != 2 {
                                return Err("HashMap.insert requires 2 arguments (key, value)".to_string());
                            }
                            let key = match &evaluated_args[0] {
                                Value::Str(s) => s.clone(),
                                other => format!("{}", other),
                            };
                            let value = evaluated_args[1].clone();
                            let mut new_map = map.clone();
                            new_map.insert(key, value);
                            return Ok(Value::HashMap(new_map));
                        }
                        "get" => {
                            if evaluated_args.len() != 1 {
                                return Err("HashMap.get requires 1 argument (key)".to_string());
                            }
                            let key = match &evaluated_args[0] {
                                Value::Str(s) => s.clone(),
                                other => format!("{}", other),
                            };
                            match map.get(&key) {
                                Some(val) => return Ok(val.clone()),
                                None => return Ok(Value::Null),
                            }
                        }
                        "contains_key" => {
                            if evaluated_args.len() != 1 {
                                return Err("HashMap.contains_key requires 1 argument (key)".to_string());
                            }
                            let key = match &evaluated_args[0] {
                                Value::Str(s) => s.clone(),
                                other => format!("{}", other),
                            };
                            return Ok(Value::Bool(map.contains_key(&key)));
                        }
                        "remove" => {
                            if evaluated_args.len() != 1 {
                                return Err("HashMap.remove requires 1 argument (key)".to_string());
                            }
                            let key = match &evaluated_args[0] {
                                Value::Str(s) => s.clone(),
                                other => format!("{}", other),
                            };
                            let mut new_map = map.clone();
                            new_map.remove(&key);
                            return Ok(Value::HashMap(new_map));
                        }
                        "len" => {
                            return Ok(Value::Int(map.len() as i64));
                        }
                        _ => {
                            return Err(format!("No method '{}' on HashMap", method));
                        }
                    }
                }

                // Check if receiver is a struct with user-defined impl methods
                if let Value::Struct { ref type_name, .. } = receiver_val {
                    if let Some(method_def) = self
                        .impl_methods
                        .get(type_name)
                        .and_then(|methods| methods.get(method))
                        .cloned()
                    {
                        // Check call depth for recursion safety
                        if self.call_depth >= self.max_call_depth {
                            return Err(format!(
                                "stack overflow: maximum call depth ({}) exceeded",
                                self.max_call_depth
                            ));
                        }
                        self.call_depth += 1;

                        // Create a new scope for the method body
                        let mut method_locals = HashMap::new();

                        // Bind parameters: first param is "self" for instance methods
                        let mut arg_idx = 0;
                        for param_name in &method_def.params {
                            if param_name == "self" {
                                method_locals.insert("self".to_string(), receiver_val.clone());
                            } else {
                                if arg_idx < evaluated_args.len() {
                                    method_locals.insert(param_name.clone(), evaluated_args[arg_idx].clone());
                                    arg_idx += 1;
                                }
                            }
                        }

                        // Inject bar context from caller's locals
                        for name in &["close", "open", "high", "low", "volume", "symbol", "in_position"] {
                            if let Some(val) = locals.get(*name) {
                                method_locals.insert(name.to_string(), val.clone());
                            }
                        }

                        let result = self.exec_fn_body(&method_def.body, &mut method_locals);
                        self.call_depth -= 1;
                        return result;
                    }
                }

                // Fall through: method not found in impl blocks
                Err(format!("No method '{}' on type '{}'", method, match &receiver_val {
                    Value::Struct { type_name, .. } => type_name.clone(),
                    other => format!("{}", other),
                }))
            }

            // --- Index access ---
            TypedExprKind::IndexAccess { object, index } => {
                let obj_val = self.eval_expr(object, locals)?;
                let idx_val = self.eval_expr(index, locals)?;
                match (&obj_val, &idx_val) {
                    (Value::VecFloat(vec), Value::Int(i)) => {
                        if *i < 0 {
                            Err(format!("index {} out of bounds (length {})", i, vec.len()))
                        } else {
                            let idx = *i as usize;
                            if idx >= vec.len() {
                                Err(format!("index {} out of bounds (length {})", i, vec.len()))
                            } else {
                                Ok(Value::Float(vec[idx]))
                            }
                        }
                    }
                    (Value::List(items), Value::Int(i)) => {
                        if *i < 0 {
                            Err(format!(
                                "runtime error: index {} out of bounds for array of size {}",
                                i,
                                items.len()
                            ))
                        } else {
                            let idx = *i as usize;
                            if idx >= items.len() {
                                Err(format!(
                                    "runtime error: index {} out of bounds for array of size {}",
                                    i,
                                    items.len()
                                ))
                            } else {
                                Ok(items[idx].clone())
                            }
                        }
                    }
                    _ => Err("index access requires a list and integer index".to_string()),
                }
            }

            // --- Struct literal ---
            TypedExprKind::StructLiteral { struct_name, fields } => {
                let mut field_map = HashMap::new();
                for (field_name, field_expr) in fields {
                    let val = self.eval_expr(field_expr, locals)?;
                    field_map.insert(field_name.clone(), val);
                }
                Ok(Value::Struct {
                    type_name: struct_name.clone(),
                    fields: field_map,
                })
            }

            // --- Enum construction ---
            TypedExprKind::EnumConstruction {
                enum_name,
                variant_name,
                args,
            } => {
                // Handle HashMap.new() as a built-in container constructor
                if enum_name == "HashMap" && variant_name == "new" {
                    return Ok(Value::HashMap(HashMap::new()));
                }

                // Look up field names from the enum definition
                let field_names: Vec<String> = self
                    .enum_defs
                    .get(enum_name)
                    .and_then(|variants| {
                        variants.iter().find(|v| v.name == *variant_name)
                    })
                    .map(|variant| {
                        variant.fields.iter().map(|(name, _)| name.clone()).collect()
                    })
                    .unwrap_or_default();

                let mut field_values = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let val = self.eval_expr(arg, locals)?;
                    let name = field_names
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("_{}", i));
                    field_values.push((name, val));
                }
                Ok(Value::Enum {
                    enum_name: enum_name.clone(),
                    variant_name: variant_name.clone(),
                    fields: field_values,
                })
            }
            TypedExprKind::Match(match_expr) => {
                // Evaluate the scrutinee
                let scrutinee_val = self.eval_expr(&match_expr.scrutinee, locals)?;

                // Try to match against each arm in order
                for arm in &match_expr.arms {
                    match &arm.pattern {
                        flux_compiler::typeck::TypedPattern::Variant {
                            variant_name: pat_variant,
                            bindings,
                            ..
                        } => {
                            // Check if scrutinee matches this variant
                            if let Value::Enum {
                                variant_name: ref val_variant,
                                ref fields,
                                ..
                            } = scrutinee_val
                            {
                                if val_variant == pat_variant {
                                    // Bind fields to local variables by position
                                    let mut arm_locals = locals.clone();
                                    for (i, (binding_name, _)) in bindings.iter().enumerate() {
                                        if let Some((_, value)) = fields.get(i) {
                                            arm_locals.insert(
                                                binding_name.clone(),
                                                value.clone(),
                                            );
                                        }
                                    }
                                    // Execute arm body, collecting signals and capturing last expr value
                                    let mut result = Value::Null;
                                    for (i, stmt) in arm.body.iter().enumerate() {
                                        let stmt_signals = self.exec_stmt(stmt, &mut arm_locals)?;
                                        self.fn_signals.extend(stmt_signals);
                                        // Capture value of the last expression statement
                                        if i == arm.body.len() - 1 {
                                            if let TypedStmt::Expr(expr_stmt) = stmt {
                                                result = self.eval_expr(&expr_stmt.expr, &arm_locals)?;
                                            }
                                        }
                                    }
                                    return Ok(result);
                                }
                            }
                        }
                        flux_compiler::typeck::TypedPattern::Wildcard { .. } => {
                            // Wildcard matches anything
                            let mut arm_locals = locals.clone();
                            let mut result = Value::Null;
                            for (i, stmt) in arm.body.iter().enumerate() {
                                let stmt_signals = self.exec_stmt(stmt, &mut arm_locals)?;
                                self.fn_signals.extend(stmt_signals);
                                if i == arm.body.len() - 1 {
                                    if let TypedStmt::Expr(expr_stmt) = stmt {
                                        result = self.eval_expr(&expr_stmt.expr, &arm_locals)?;
                                    }
                                }
                            }
                            return Ok(result);
                        }
                    }
                }

                // No arm matched — return a descriptive runtime error
                if let Value::Enum { ref enum_name, ref variant_name, .. } = scrutinee_val {
                    Err(format!(
                        "runtime error: non-exhaustive match on {}.{}",
                        enum_name, variant_name
                    ))
                } else {
                    Err("runtime error: non-exhaustive match on unknown value".to_string())
                }
            }
        }
    }

    /// Helper: evaluate an expression and coerce the result to a String.
    fn eval_expr_as_string(
        &mut self,
        expr: &TypedExpr,
        locals: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let val = self.eval_expr(expr, locals)?;
        match val {
            Value::Str(s) => Ok(s),
            _ => Err("expected a string value".to_string()),
        }
    }

    /// Helper: evaluate an expression and coerce the result to f64.
    fn eval_expr_as_f64(
        &mut self,
        expr: &TypedExpr,
        locals: &HashMap<String, Value>,
    ) -> Result<f64, String> {
        let val = self.eval_expr(expr, locals)?;
        match val {
            Value::Float(f) => Ok(f),
            Value::Int(i) => Ok(i as f64),
            _ => Err("expected a numeric value".to_string()),
        }
    }

    /// Helper: evaluate an expression and coerce the result to i64.
    fn eval_expr_as_i64(
        &mut self,
        expr: &TypedExpr,
        locals: &HashMap<String, Value>,
    ) -> Result<i64, String> {
        let val = self.eval_expr(expr, locals)?;
        match val {
            Value::Int(i) => Ok(i),
            _ => Err("expected an integer value".to_string()),
        }
    }

    /// Execute a single typed statement, returning any signals emitted.
    pub fn exec_stmt(
        &mut self,
        stmt: &TypedStmt,
        locals: &mut HashMap<String, Value>,
    ) -> Result<Vec<Signal>, String> {
        match stmt {
            TypedStmt::Assignment(assignment) => {
                // Get target name from the target expression (expect Ident)
                let name = match &assignment.target.kind {
                    TypedExprKind::Ident(name) => name.clone(),
                    _ => return Err("assignment target must be an identifier".to_string()),
                };

                // Evaluate the RHS
                let value = self.eval_expr(&assignment.value, locals)?;

                // Drain any signals emitted by user-function calls in the RHS
                let mut signals = Vec::new();
                signals.append(&mut self.fn_signals);

                // If the name exists in state, update state; otherwise store in locals
                if self.state.contains_key(&name) {
                    self.state.insert(name, value);
                } else {
                    locals.insert(name, value);
                }

                Ok(signals)
            }

            TypedStmt::If(if_stmt) => {
                // Evaluate the main condition
                let cond_val = self.eval_expr(&if_stmt.condition, locals)?;
                let cond = match cond_val {
                    Value::Bool(b) => b,
                    _ => return Err("if condition must be a boolean".to_string()),
                };

                if cond {
                    return self.exec_stmts(&if_stmt.body, locals);
                }

                // Check elif branches
                for elif in &if_stmt.elif_branches {
                    let elif_cond_val = self.eval_expr(&elif.condition, locals)?;
                    let elif_cond = match elif_cond_val {
                        Value::Bool(b) => b,
                        _ => return Err("elif condition must be a boolean".to_string()),
                    };

                    if elif_cond {
                        return self.exec_stmts(&elif.body, locals);
                    }
                }

                // Execute else body if present
                if let Some(else_body) = &if_stmt.else_body {
                    return self.exec_stmts(else_body, locals);
                }

                Ok(vec![])
            }

            TypedStmt::For(for_loop) => {
                // Evaluate iterable — must be Value::List
                let iterable_val = self.eval_expr(&for_loop.iterable, locals)?;
                let items = match iterable_val {
                    Value::List(items) => items,
                    _ => return Err("for loop iterable must be a list".to_string()),
                };

                let mut signals = Vec::new();

                for item in items {
                    locals.insert(for_loop.variable.clone(), item);
                    let stmts_signals = self.exec_stmts(&for_loop.body, locals)?;
                    signals.extend(stmts_signals);
                }

                // Remove the loop variable after the loop completes
                locals.remove(&for_loop.variable);

                Ok(signals)
            }

            TypedStmt::While(while_loop) => {
                let mut signals = Vec::new();
                let max_iterations = 10_000;
                let mut iteration = 0;

                loop {
                    if iteration >= max_iterations {
                        return Err(format!(
                            "while loop exceeded maximum iterations ({})",
                            max_iterations
                        ));
                    }

                    let cond_val = self.eval_expr(&while_loop.condition, locals)?;
                    let cond = match cond_val {
                        Value::Bool(b) => b,
                        _ => return Err("while condition must be a boolean".to_string()),
                    };

                    if !cond {
                        break;
                    }

                    let stmts_signals = self.exec_stmts(&while_loop.body, locals)?;
                    signals.extend(stmts_signals);
                    iteration += 1;
                }

                Ok(signals)
            }

            TypedStmt::Expr(expr_stmt) => {
                let value = self.eval_expr(&expr_stmt.expr, locals)?;
                let mut signals = Vec::new();
                // Drain any signals emitted by user-function calls
                signals.append(&mut self.fn_signals);

                // Implicit reassignment for mutating HashMap methods (insert, remove).
                // When a MethodCall on an Ident receiver with a mutating method produces
                // a Value::HashMap, write it back to the variable so the user doesn't need
                // to write `map = map.insert(k, v)` explicitly.
                if let TypedExprKind::MethodCall { receiver, method, .. } = &expr_stmt.expr.kind {
                    if matches!(method.as_str(), "insert" | "remove") {
                        if let TypedExprKind::Ident(name) = &receiver.kind {
                            if matches!(&value, Value::HashMap(_)) {
                                if self.state.contains_key(name) {
                                    self.state.insert(name.clone(), value.clone());
                                } else {
                                    locals.insert(name.clone(), value.clone());
                                }
                            }
                        }
                    }
                }

                // Also collect if the expression itself is a signal
                match value {
                    Value::Signal(sig) => {
                        signals.push(sig);
                    }
                    _ => {}
                }
                Ok(signals)
            }

            TypedStmt::Return(return_stmt) => {
                // Evaluate the return value (if any) and propagate as a special error
                // so that exec_fn_body can catch it and extract the value.
                let value = match &return_stmt.value {
                    Some(expr) => self.eval_expr(expr, locals)?,
                    None => Value::Null,
                };
                // Store the return value and propagate a sentinel error string.
                // exec_fn_body will intercept this to extract the value from pending_return.
                self.pending_return = Some(value);
                Err("__RETURN__".to_string())
            }
        }
    }

    /// Execute a sequence of statements, collecting all emitted signals.
    pub fn exec_stmts(
        &mut self,
        stmts: &[TypedStmt],
        locals: &mut HashMap<String, Value>,
    ) -> Result<Vec<Signal>, String> {
        let mut signals = Vec::new();
        for stmt in stmts {
            let stmt_signals = self.exec_stmt(stmt, locals)?;
            signals.extend(stmt_signals);
        }
        Ok(signals)
    }

    /// Execute a user-defined function body, handling `return` as early termination.
    ///
    /// Signals emitted inside the function body (via OPEN/CLOSE/CLOSE_QTY)
    /// are propagated to the caller through `self.fn_signals`.
    /// Returns the function's return value (or Null if no return statement).
    fn exec_fn_body(
        &mut self,
        body: &[TypedStmt],
        locals: &mut HashMap<String, Value>,
    ) -> Result<Value, String> {
        let mut all_signals = Vec::new();

        for stmt in body {
            match stmt {
                TypedStmt::Return(return_stmt) => {
                    let value = match &return_stmt.value {
                        Some(expr) => {
                            let val = self.eval_expr(expr, locals)?;
                            // Drain any signals from evaluating the return expression
                            all_signals.append(&mut self.fn_signals);
                            val
                        }
                        None => Value::Null,
                    };
                    // Flush accumulated signals to the caller's signal list
                    self.fn_signals.extend(all_signals);
                    return Ok(value);
                }
                _ => {
                    match self.exec_stmt(stmt, locals) {
                        Ok(stmt_signals) => {
                            all_signals.extend(stmt_signals);
                        }
                        Err(e) if e == "__RETURN__" => {
                            // A return statement was hit inside a nested block (if/while/for).
                            // Extract the value from pending_return.
                            let value = self.pending_return.take().unwrap_or(Value::Null);
                            self.fn_signals.extend(all_signals);
                            return Ok(value);
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }

        // No explicit return — flush signals and return Null
        self.fn_signals.extend(all_signals);
        Ok(Value::Null)
    }

    /// Decode a return value from the sentinel error string format.
    /// Kept for backward compatibility with tests that exercise the sentinel path.
    #[allow(dead_code)]
    fn decode_return_value(encoded: &str) -> Value {
        let rest = &encoded["__RETURN__:".len()..];
        if let Some(val_str) = rest.strip_prefix("float:") {
            Value::Float(val_str.parse::<f64>().unwrap_or(0.0))
        } else if let Some(val_str) = rest.strip_prefix("int:") {
            Value::Int(val_str.parse::<i64>().unwrap_or(0))
        } else if let Some(val_str) = rest.strip_prefix("bool:") {
            Value::Bool(val_str == "true")
        } else if let Some(val_str) = rest.strip_prefix("str:") {
            Value::Str(val_str.to_string())
        } else {
            Value::Null
        }
    }
}

/// Evaluate a typed expression that is expected to be a literal value.
///
/// This is used for param defaults and state initial values, which are
/// always literals after type checking.
fn eval_literal(expr: &TypedExpr) -> Value {
    match &expr.kind {
        TypedExprKind::IntLiteral(i) => Value::Int(*i),
        TypedExprKind::FloatLiteral(f) => Value::Float(*f),
        TypedExprKind::StringLiteral(s) => Value::Str(s.clone()),
        TypedExprKind::BoolLiteral(b) => Value::Bool(*b),
        TypedExprKind::NullLiteral => Value::Null,
        TypedExprKind::ListLiteral(items) => {
            Value::List(items.iter().map(eval_literal).collect())
        }
        _ => panic!(
            "eval_literal: unexpected non-literal expression kind in default/initial value"
        ),
    }
}

/// Helper for arithmetic binary operations with Int/Float coercion.
fn eval_arith(
    left: &Value,
    right: &Value,
    op_name: &str,
    int_op: impl Fn(i64, i64) -> i64,
    float_op: impl Fn(f64, f64) -> f64,
) -> Result<Value, String> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(int_op(*a, *b))),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(float_op(*a as f64, *b))),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(float_op(*a, *b as f64))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(*a, *b))),
        _ => Err(format!("operator '{}' requires numeric operands", op_name)),
    }
}

/// Helper for comparison binary operations with Int/Float coercion.
fn eval_cmp(
    left: &Value,
    right: &Value,
    op_name: &str,
    int_cmp: impl Fn(i64, i64) -> bool,
    float_cmp: impl Fn(f64, f64) -> bool,
) -> Result<Value, String> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(int_cmp(*a, *b))),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool(float_cmp(*a as f64, *b))),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(float_cmp(*a, *b as f64))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(float_cmp(*a, *b))),
        _ => Err(format!("operator '{}' requires numeric operands", op_name)),
    }
}

/// Helper for equality comparison, supporting all Value types.
fn eval_eq(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) == *b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a == (*b as f64))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a == b)),
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
        (Value::Str(a), Value::Str(b)) => Ok(Value::Bool(a == b)),
        (Value::Null, Value::Null) => Ok(Value::Bool(true)),
        (Value::Null, _) | (_, Value::Null) => Ok(Value::Bool(false)),
        _ => Err("equality comparison not supported for these types".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_compiler::lexer::Span;
    use flux_compiler::typeck::types::FluxType;

    /// Helper to create a TypedExpr with a given kind and type
    fn typed_expr(kind: TypedExprKind, resolved_type: FluxType) -> TypedExpr {
        TypedExpr {
            kind,
            resolved_type,
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn test_eval_literal_int() {
        let expr = typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int);
        match eval_literal(&expr) {
            Value::Int(v) => assert_eq!(v, 42),
            other => panic!("Expected Value::Int, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_literal_float() {
        let expr = typed_expr(TypedExprKind::FloatLiteral(3.14), FluxType::Float);
        match eval_literal(&expr) {
            Value::Float(v) => assert!((v - 3.14).abs() < f64::EPSILON),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_literal_string() {
        let expr = typed_expr(
            TypedExprKind::StringLiteral("hello".to_string()),
            FluxType::String,
        );
        match eval_literal(&expr) {
            Value::Str(v) => assert_eq!(v, "hello"),
            other => panic!("Expected Value::Str, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_literal_bool() {
        let expr = typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool);
        match eval_literal(&expr) {
            Value::Bool(v) => assert!(v),
            other => panic!("Expected Value::Bool, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_literal_null() {
        let expr = typed_expr(TypedExprKind::NullLiteral, FluxType::Null);
        match eval_literal(&expr) {
            Value::Null => {}
            other => panic!("Expected Value::Null, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_literal_list() {
        let items = vec![
            typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
            typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
        ];
        let expr = typed_expr(
            TypedExprKind::ListLiteral(items),
            FluxType::List(Box::new(FluxType::Int)),
        );
        match eval_literal(&expr) {
            Value::List(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(v[0], Value::Int(1)));
                assert!(matches!(v[1], Value::Int(2)));
                assert!(matches!(v[2], Value::Int(3)));
            }
            other => panic!("Expected Value::List, got {:?}", other),
        }
    }

    #[test]
    fn test_interpreter_new_basic() {
        // Build a minimal TypedProgram with params and state
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "Test".to_string(),
                body: vec![
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![
                            TypedParam {
                                name: "period".to_string(),
                                default_value: typed_expr(
                                    TypedExprKind::IntLiteral(20),
                                    FluxType::Int,
                                ),
                                resolved_type: FluxType::Int,
                                span: Span::new(0, 0),
                            },
                            TypedParam {
                                name: "threshold".to_string(),
                                default_value: typed_expr(
                                    TypedExprKind::FloatLiteral(2.5),
                                    FluxType::Float,
                                ),
                                resolved_type: FluxType::Float,
                                span: Span::new(0, 0),
                            },
                        ],
                        span: Span::new(0, 0),
                    }),
                    TypedStrategyItem::StateBlock(TypedStateBlock {
                        variables: vec![
                            TypedStateVar {
                                name: "count".to_string(),
                                initial_value: typed_expr(
                                    TypedExprKind::IntLiteral(0),
                                    FluxType::Int,
                                ),
                                resolved_type: FluxType::Int,
                                span: Span::new(0, 0),
                            },
                            TypedStateVar {
                                name: "active".to_string(),
                                initial_value: typed_expr(
                                    TypedExprKind::BoolLiteral(false),
                                    FluxType::Bool,
                                ),
                                resolved_type: FluxType::Bool,
                                span: Span::new(0, 0),
                            },
                        ],
                        span: Span::new(0, 0),
                    }),
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![],
                        span: Span::new(0, 0),
                    }),
                ],
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        };

        let interp = Interpreter::new(&program);

        // Verify params
        assert_eq!(interp.params.len(), 2);
        assert!(matches!(interp.params.get("period"), Some(Value::Int(20))));
        assert!(matches!(
            interp.params.get("threshold"),
            Some(Value::Float(f)) if (*f - 2.5).abs() < f64::EPSILON
        ));

        // Verify state
        assert_eq!(interp.state.len(), 2);
        assert!(matches!(interp.state.get("count"), Some(Value::Int(0))));
        assert!(matches!(
            interp.state.get("active"),
            Some(Value::Bool(false))
        ));

        // Verify event handler stored
        assert!(interp.event_handler.is_some());
        assert_eq!(interp.event_handler.as_ref().unwrap().event_name, "bar");
    }

    #[test]
    fn test_interpreter_new_no_handler() {
        // A program with no event handler
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "Empty".to_string(),
                body: vec![],
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        };

        let interp = Interpreter::new(&program);

        assert!(interp.params.is_empty());
        assert!(interp.state.is_empty());
        assert!(interp.event_handler.is_none());
    }

    #[test]
    #[should_panic(expected = "unexpected non-literal expression kind")]
    fn test_eval_literal_panics_on_non_literal() {
        let expr = typed_expr(
            TypedExprKind::Ident("x".to_string()),
            FluxType::Int,
        );
        eval_literal(&expr);
    }

    // ========================================================================
    // Helper: create a minimal interpreter for expression evaluation tests
    // ========================================================================

    fn make_interp() -> Interpreter {
        Interpreter {
            params: HashMap::new(),
            state: HashMap::new(),
            event_handler: None,
            indicators: HashMap::new(),
            in_position: false,
            prev_closes: HashMap::new(),
            current_closes: HashMap::new(),
            functions: HashMap::new(),
            enum_defs: HashMap::new(),
            call_depth: 0,
            max_call_depth: 64,
            fn_signals: Vec::new(),
            pending_return: None,
            impl_methods: HashMap::new(),
        }
    }

    fn make_binop(left: TypedExpr, op: BinOp, right: TypedExpr, ty: FluxType) -> TypedExpr {
        typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            },
            ty,
        )
    }

    fn make_unaryop(op: UnaryOp, operand: TypedExpr, ty: FluxType) -> TypedExpr {
        typed_expr(
            TypedExprKind::UnaryOp {
                op,
                operand: Box::new(operand),
            },
            ty,
        )
    }

    fn make_fn_call(name: &str, args: Vec<TypedExpr>, ty: FluxType) -> TypedExpr {
        typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident(name.to_string()),
                    FluxType::Fn {
                        params: flux_compiler::typeck::types::FnParams::Fixed(vec![]),
                        ret: Box::new(ty.clone()),
                    },
                )),
                args,
            },
            ty,
        )
    }

    // ========================================================================
    // Expression evaluation tests: arithmetic
    // ========================================================================

    #[test]
    fn test_eval_expr_int_add() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
            BinOp::Add,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_eval_expr_int_sub() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            BinOp::Sub,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(7)));
    }

    #[test]
    fn test_eval_expr_int_mul() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(4), FluxType::Int),
            BinOp::Mul,
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(20)));
    }

    #[test]
    fn test_eval_expr_int_div() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        // Integer division: 10 / 3 = 3
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            BinOp::Div,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(3)));
    }

    #[test]
    fn test_eval_expr_int_mod() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            BinOp::Mod,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(1)));
    }

    #[test]
    fn test_eval_expr_float_add() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::FloatLiteral(1.5), FluxType::Float),
            BinOp::Add,
            typed_expr(TypedExprKind::FloatLiteral(2.5), FluxType::Float),
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Float(f) => assert!((f - 4.0).abs() < f64::EPSILON),
            other => panic!("Expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_expr_float_div() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::FloatLiteral(10.0), FluxType::Float),
            BinOp::Div,
            typed_expr(TypedExprKind::FloatLiteral(3.0), FluxType::Float),
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Float(f) => assert!((f - 10.0 / 3.0).abs() < f64::EPSILON),
            other => panic!("Expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_expr_mixed_int_float_add() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        // 1 + 2.5 = 3.5 (Int promoted to Float)
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
            BinOp::Add,
            typed_expr(TypedExprKind::FloatLiteral(2.5), FluxType::Float),
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.5).abs() < f64::EPSILON),
            other => panic!("Expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_expr_division_by_zero_int() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            BinOp::Div,
            typed_expr(TypedExprKind::IntLiteral(0), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("division by zero"));
    }

    #[test]
    fn test_eval_expr_mod_by_zero() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            BinOp::Mod,
            typed_expr(TypedExprKind::IntLiteral(0), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("division by zero"));
    }

    // ========================================================================
    // Expression evaluation tests: comparisons
    // ========================================================================

    #[test]
    fn test_eval_expr_gt_true() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            BinOp::Gt,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_gt_false() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            BinOp::Gt,
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn test_eval_expr_eq_true() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            BinOp::Eq,
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_eq_false() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            BinOp::Eq,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn test_eval_expr_lt() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            BinOp::Lt,
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_le() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            BinOp::Le,
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_ge() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            BinOp::Ge,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_ne() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            BinOp::Ne,
            typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    // ========================================================================
    // Expression evaluation tests: boolean logic
    // ========================================================================

    #[test]
    fn test_eval_expr_and_false() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            BinOp::And,
            typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn test_eval_expr_and_true() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            BinOp::And,
            typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_or_true() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            BinOp::Or,
            typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_or_false() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_binop(
            typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
            BinOp::Or,
            typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn test_eval_expr_not_true() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_unaryop(
            UnaryOp::Not,
            typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn test_eval_expr_not_false() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_unaryop(
            UnaryOp::Not,
            typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_eval_expr_negation() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_unaryop(
            UnaryOp::Neg,
            typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int),
            FluxType::Int,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(-42)));
    }

    // ========================================================================
    // Variable lookup tests
    // ========================================================================

    #[test]
    fn test_eval_expr_ident_from_locals() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();
        locals.insert("x".to_string(), Value::Int(99));
        let expr = typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int);
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(99)));
    }

    #[test]
    fn test_eval_expr_ident_from_params() {
        let mut interp = make_interp();
        interp.params.insert("period".to_string(), Value::Int(20));
        let locals = HashMap::new();
        let expr = typed_expr(TypedExprKind::Ident("period".to_string()), FluxType::Int);
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(20)));
    }

    #[test]
    fn test_eval_expr_ident_from_state() {
        let mut interp = make_interp();
        interp.state.insert("count".to_string(), Value::Int(5));
        let locals = HashMap::new();
        let expr = typed_expr(TypedExprKind::Ident("count".to_string()), FluxType::Int);
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_eval_expr_ident_locals_shadow_params() {
        let mut interp = make_interp();
        interp.params.insert("x".to_string(), Value::Int(100));
        let mut locals = HashMap::new();
        locals.insert("x".to_string(), Value::Int(42));
        let expr = typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int);
        let result = interp.eval_expr(&expr, &locals).unwrap();
        // Locals take priority over params
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_eval_expr_ident_undefined_variable() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = typed_expr(TypedExprKind::Ident("undefined_var".to_string()), FluxType::Int);
        let result = interp.eval_expr(&expr, &locals);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("undefined variable"));
    }

    // ========================================================================
    // Statement execution tests: assignment
    // ========================================================================

    #[test]
    fn test_exec_stmt_assignment_to_locals() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();
        let stmt = TypedStmt::Assignment(TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int),
            span: Span::new(0, 0),
        });
        let signals = interp.exec_stmt(&stmt, &mut locals).unwrap();
        assert!(signals.is_empty());
        assert!(matches!(locals.get("x"), Some(Value::Int(42))));
    }

    #[test]
    fn test_exec_stmt_assignment_to_state() {
        let mut interp = make_interp();
        interp.state.insert("count".to_string(), Value::Int(0));
        let mut locals = HashMap::new();
        let stmt = TypedStmt::Assignment(TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("count".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            span: Span::new(0, 0),
        });
        let signals = interp.exec_stmt(&stmt, &mut locals).unwrap();
        assert!(signals.is_empty());
        // Should update state, not locals
        assert!(matches!(interp.state.get("count"), Some(Value::Int(10))));
        assert!(locals.get("count").is_none());
    }

    // ========================================================================
    // Statement execution tests: if/else branching
    // ========================================================================

    #[test]
    fn test_exec_stmt_if_true_branch() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();
        let stmt = TypedStmt::If(TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Assignment(TypedAssignment {
                target: typed_expr(TypedExprKind::Ident("result".to_string()), FluxType::Int),
                value: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 0),
            })],
            elif_branches: vec![],
            else_body: Some(vec![TypedStmt::Assignment(TypedAssignment {
                target: typed_expr(TypedExprKind::Ident("result".to_string()), FluxType::Int),
                value: typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                span: Span::new(0, 0),
            })]),
            span: Span::new(0, 0),
        });
        let signals = interp.exec_stmt(&stmt, &mut locals).unwrap();
        assert!(signals.is_empty());
        assert!(matches!(locals.get("result"), Some(Value::Int(1))));
    }

    #[test]
    fn test_exec_stmt_if_else_branch() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();
        let stmt = TypedStmt::If(TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
            body: vec![TypedStmt::Assignment(TypedAssignment {
                target: typed_expr(TypedExprKind::Ident("result".to_string()), FluxType::Int),
                value: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 0),
            })],
            elif_branches: vec![],
            else_body: Some(vec![TypedStmt::Assignment(TypedAssignment {
                target: typed_expr(TypedExprKind::Ident("result".to_string()), FluxType::Int),
                value: typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                span: Span::new(0, 0),
            })]),
            span: Span::new(0, 0),
        });
        let signals = interp.exec_stmt(&stmt, &mut locals).unwrap();
        assert!(signals.is_empty());
        assert!(matches!(locals.get("result"), Some(Value::Int(2))));
    }

    #[test]
    fn test_exec_stmt_if_elif_branch() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();
        let stmt = TypedStmt::If(TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
            body: vec![TypedStmt::Assignment(TypedAssignment {
                target: typed_expr(TypedExprKind::Ident("result".to_string()), FluxType::Int),
                value: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 0),
            })],
            elif_branches: vec![TypedElifBranch {
                condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
                body: vec![TypedStmt::Assignment(TypedAssignment {
                    target: typed_expr(TypedExprKind::Ident("result".to_string()), FluxType::Int),
                    value: typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
                    span: Span::new(0, 0),
                })],
                span: Span::new(0, 0),
            }],
            else_body: Some(vec![TypedStmt::Assignment(TypedAssignment {
                target: typed_expr(TypedExprKind::Ident("result".to_string()), FluxType::Int),
                value: typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                span: Span::new(0, 0),
            })]),
            span: Span::new(0, 0),
        });
        let signals = interp.exec_stmt(&stmt, &mut locals).unwrap();
        assert!(signals.is_empty());
        assert!(matches!(locals.get("result"), Some(Value::Int(3))));
    }

    // ========================================================================
    // Signal emission tests
    // ========================================================================

    #[test]
    fn test_eval_expr_open_signal() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "OPEN",
            vec![
                typed_expr(TypedExprKind::StringLiteral("AAPL".to_string()), FluxType::String),
                typed_expr(TypedExprKind::FloatLiteral(100.0), FluxType::Float),
            ],
            FluxType::Signal,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Signal(sig) => {
                assert_eq!(sig.symbol(), "AAPL");
                assert_eq!(sig.qty(), Some(100.0));
                assert!(matches!(sig, Signal::Open { .. }));
            }
            other => panic!("Expected Signal, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_expr_close_signal() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "CLOSE",
            vec![typed_expr(
                TypedExprKind::StringLiteral("MSFT".to_string()),
                FluxType::String,
            )],
            FluxType::Signal,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Signal(sig) => {
                assert_eq!(sig.symbol(), "MSFT");
                assert_eq!(sig.qty(), None);
                assert!(matches!(sig, Signal::Close { .. }));
            }
            other => panic!("Expected Signal, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_expr_close_qty_signal() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "CLOSE_QTY",
            vec![
                typed_expr(TypedExprKind::StringLiteral("GOOG".to_string()), FluxType::String),
                typed_expr(TypedExprKind::FloatLiteral(50.0), FluxType::Float),
            ],
            FluxType::Signal,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Signal(sig) => {
                assert_eq!(sig.symbol(), "GOOG");
                assert_eq!(sig.qty(), Some(50.0));
                assert!(matches!(sig, Signal::CloseQty { .. }));
            }
            other => panic!("Expected Signal, got {:?}", other),
        }
    }

    #[test]
    fn test_exec_stmt_signal_emission() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();
        // An expression statement that calls OPEN → emits signal
        let stmt = TypedStmt::Expr(TypedExprStmt {
            expr: make_fn_call(
                "OPEN",
                vec![
                    typed_expr(TypedExprKind::StringLiteral("SPY".to_string()), FluxType::String),
                    typed_expr(TypedExprKind::FloatLiteral(10.0), FluxType::Float),
                ],
                FluxType::Signal,
            ),
            span: Span::new(0, 0),
        });
        let signals = interp.exec_stmt(&stmt, &mut locals).unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].symbol(), "SPY");
        assert!(matches!(signals[0], Signal::Open { .. }));
    }

    // ========================================================================
    // Indicator function tests (sma, ema)
    // ========================================================================

    #[test]
    fn test_eval_expr_sma_call() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "sma",
            vec![
                typed_expr(TypedExprKind::FloatLiteral(100.0), FluxType::Float),
                typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            ],
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        // sma with one value and period 5 should return that value / 5 or the initial call
        // The exact value depends on the runtime implementation, just verify it's a Float
        assert!(matches!(result, Value::Float(_)));
    }

    #[test]
    fn test_eval_expr_ema_call() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "ema",
            vec![
                typed_expr(TypedExprKind::FloatLiteral(50.0), FluxType::Float),
                typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            ],
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Float(_)));
    }

    // ========================================================================
    // on_bar integration test
    // ========================================================================

    #[test]
    fn test_on_bar_emits_open_when_close_gt_open() {
        // Build a program: on bar { if close > open { OPEN(symbol, 100.0) } }
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "TestStrategy".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![TypedStmt::If(TypedIfStmt {
                        condition: make_binop(
                            typed_expr(TypedExprKind::Ident("close".to_string()), FluxType::Float),
                            BinOp::Gt,
                            typed_expr(TypedExprKind::Ident("open".to_string()), FluxType::Float),
                            FluxType::Bool,
                        ),
                        body: vec![TypedStmt::Expr(TypedExprStmt {
                            expr: make_fn_call(
                                "OPEN",
                                vec![
                                    typed_expr(
                                        TypedExprKind::Ident("symbol".to_string()),
                                        FluxType::String,
                                    ),
                                    typed_expr(TypedExprKind::FloatLiteral(100.0), FluxType::Float),
                                ],
                                FluxType::Signal,
                            ),
                            span: Span::new(0, 0),
                        })],
                        elif_branches: vec![],
                        else_body: None,
                        span: Span::new(0, 0),
                    })],
                    span: Span::new(0, 0),
                })],
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        };

        let mut interp = Interpreter::new(&program);

        // Bar where close > open → should emit Open signal
        let ctx_bullish = BarContext {
            close: 150.0,
            open: 140.0,
            high: 155.0,
            low: 138.0,
            volume: 1_000_000.0,
            symbol: "AAPL".to_string(),
            in_position: false,
        };
        let signals = interp.on_bar(&ctx_bullish);
        assert_eq!(signals.len(), 1);
        assert!(matches!(&signals[0], Signal::Open { symbol, qty } if symbol == "AAPL" && *qty == 100.0));

        // Bar where close <= open → no signals
        let ctx_bearish = BarContext {
            close: 130.0,
            open: 140.0,
            high: 145.0,
            low: 128.0,
            volume: 500_000.0,
            symbol: "AAPL".to_string(),
            in_position: false,
        };
        let signals = interp.on_bar(&ctx_bearish);
        assert!(signals.is_empty());
    }

    #[test]
    fn test_on_bar_no_handler_returns_empty() {
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "Empty".to_string(),
                body: vec![],
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        };
        let mut interp = Interpreter::new(&program);
        let ctx = BarContext {
            close: 100.0,
            open: 99.0,
            high: 101.0,
            low: 98.0,
            volume: 1000.0,
            symbol: "TEST".to_string(),
            in_position: false,
        };
        let signals = interp.on_bar(&ctx);
        assert!(signals.is_empty());
    }

    // ========================================================================
    // Dispatch integration tests (Task 7.2)
    // Validates: Requirements 16.1, 16.3
    // ========================================================================

    /// Test that a Tier 1 math function (abs) dispatches correctly through eval_expr.
    /// Validates: Requirement 16.1 — stateless functions produce identical outputs for identical inputs.
    #[test]
    fn test_dispatch_math_abs_negative_float() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "abs",
            vec![typed_expr(TypedExprKind::FloatLiteral(-3.5), FluxType::Float)],
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.5).abs() < f64::EPSILON, "abs(-3.5) should be 3.5, got {}", f),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    /// Test that a Tier 1 math function (sqrt) dispatches correctly through eval_expr.
    #[test]
    fn test_dispatch_math_sqrt() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "sqrt",
            vec![typed_expr(TypedExprKind::FloatLiteral(9.0), FluxType::Float)],
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.0).abs() < f64::EPSILON, "sqrt(9.0) should be 3.0, got {}", f),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    /// Test that a Tier 1 math function (abs) works with Int values through eval_expr.
    #[test]
    fn test_dispatch_math_abs_int() {
        let mut interp = make_interp();
        let locals = HashMap::new();
        let expr = make_fn_call(
            "abs",
            vec![typed_expr(TypedExprKind::IntLiteral(-7), FluxType::Int)],
            FluxType::Float, // dispatch returns Float for type resolution, but abs(Int) → Int
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::Int(i) => assert_eq!(i, 7, "abs(-7) should be 7"),
            other => panic!("Expected Value::Int, got {:?}", other),
        }
    }

    /// Test that a stateful indicator (stddev) maintains state across multiple calls
    /// through the interpreter dispatch.
    /// Validates: Requirement 16.3 — stateful functions use per-call-site state.
    #[test]
    fn test_dispatch_stddev_maintains_state_across_calls() {
        let mut interp = make_interp();
        let locals = HashMap::new();

        // Feed values [2.0, 4.0, 4.0, 4.0, 5.0] with period=5
        // Population stddev of [2,4,4,4,5] = sqrt(((2-3.8)^2 + (4-3.8)^2 + (4-3.8)^2 + (4-3.8)^2 + (5-3.8)^2)/5)
        // = sqrt((3.24 + 0.04 + 0.04 + 0.04 + 1.44)/5) = sqrt(4.8/5) = sqrt(0.96) ≈ 0.9798
        let values = [2.0, 4.0, 4.0, 4.0, 5.0];

        // Use span (10, 20) for the call-site key
        let mut last_result = Value::Float(0.0);
        for &val in &values {
            let expr = TypedExpr {
                kind: TypedExprKind::FunctionCall {
                    function: Box::new(TypedExpr {
                        kind: TypedExprKind::Ident("stddev".to_string()),
                        resolved_type: FluxType::Fn {
                            params: flux_compiler::typeck::types::FnParams::Fixed(vec![]),
                            ret: Box::new(FluxType::Float),
                        },
                        span: Span::new(10, 20),
                    }),
                    args: vec![
                        typed_expr(TypedExprKind::FloatLiteral(val), FluxType::Float),
                        typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
                    ],
                },
                resolved_type: FluxType::Float,
                span: Span::new(10, 20),
            };
            last_result = interp.eval_expr(&expr, &locals).unwrap();
        }

        // After all 5 values, stddev should be sqrt(0.96) ≈ 0.9798
        match last_result {
            Value::Float(f) => {
                let expected = (0.96_f64).sqrt();
                assert!(
                    (f - expected).abs() < 1e-10,
                    "stddev of [2,4,4,4,5] with period=5 should be {}, got {}",
                    expected,
                    f
                );
            }
            other => panic!("Expected Value::Float, got {:?}", other),
        }

        // Verify that indicator state was actually stored
        assert!(!interp.indicators.is_empty(), "Indicators map should have state entries after stateful calls");
    }

    /// Test that feeding a constant series through stddev yields 0.0 (no variance).
    #[test]
    fn test_dispatch_stddev_constant_series_zero() {
        let mut interp = make_interp();
        let locals = HashMap::new();

        let mut last_result = Value::Float(0.0);
        for _ in 0..5 {
            let expr = TypedExpr {
                kind: TypedExprKind::FunctionCall {
                    function: Box::new(TypedExpr {
                        kind: TypedExprKind::Ident("stddev".to_string()),
                        resolved_type: FluxType::Fn {
                            params: flux_compiler::typeck::types::FnParams::Fixed(vec![]),
                            ret: Box::new(FluxType::Float),
                        },
                        span: Span::new(30, 40),
                    }),
                    args: vec![
                        typed_expr(TypedExprKind::FloatLiteral(5.0), FluxType::Float),
                        typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
                    ],
                },
                resolved_type: FluxType::Float,
                span: Span::new(30, 40),
            };
            last_result = interp.eval_expr(&expr, &locals).unwrap();
        }

        match last_result {
            Value::Float(f) => assert!(
                f.abs() < 1e-10,
                "stddev of constant series should be 0.0, got {}",
                f
            ),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    /// Test that existing sma function still works correctly through the dispatch.
    /// Validates: Requirement 16.3 — existing sma/ema still work after refactoring.
    #[test]
    fn test_dispatch_sma_correct_results() {
        let mut interp = make_interp();
        let locals = HashMap::new();

        // Feed values [10.0, 20.0, 30.0] with period=3
        // After 3 values, sma should be (10+20+30)/3 = 20.0
        let values = [10.0, 20.0, 30.0];
        let mut last_result = Value::Float(0.0);

        for &val in &values {
            let expr = make_fn_call(
                "sma",
                vec![
                    typed_expr(TypedExprKind::FloatLiteral(val), FluxType::Float),
                    typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
                ],
                FluxType::Float,
            );
            last_result = interp.eval_expr(&expr, &locals).unwrap();
        }

        match last_result {
            Value::Float(f) => assert!(
                (f - 20.0).abs() < f64::EPSILON,
                "sma([10, 20, 30], 3) should be 20.0, got {}",
                f
            ),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    /// Test sma with a rolling window — verify older values are dropped.
    #[test]
    fn test_dispatch_sma_rolling_window() {
        let mut interp = make_interp();
        let locals = HashMap::new();

        // Feed values [10.0, 20.0, 30.0, 40.0] with period=3
        // After 4th value, window should be [20, 30, 40], sma = 30.0
        let values = [10.0, 20.0, 30.0, 40.0];
        let mut last_result = Value::Float(0.0);

        for &val in &values {
            let expr = make_fn_call(
                "sma",
                vec![
                    typed_expr(TypedExprKind::FloatLiteral(val), FluxType::Float),
                    typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
                ],
                FluxType::Float,
            );
            last_result = interp.eval_expr(&expr, &locals).unwrap();
        }

        match last_result {
            Value::Float(f) => assert!(
                (f - 30.0).abs() < f64::EPSILON,
                "sma([10,20,30,40], period=3) final should be 30.0, got {}",
                f
            ),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    /// Test that existing ema function still works correctly through the dispatch.
    #[test]
    fn test_dispatch_ema_correct_results() {
        let mut interp = make_interp();
        let locals = HashMap::new();

        // Feed values [10.0, 20.0, 30.0] with period=3
        // EMA: k = 2/(3+1) = 0.5
        // After value 10.0: ema = 10.0 (first value)
        // After value 20.0: ema = 20.0 * 0.5 + 10.0 * 0.5 = 15.0
        // After value 30.0: ema = 30.0 * 0.5 + 15.0 * 0.5 = 22.5
        let values = [10.0, 20.0, 30.0];
        let mut last_result = Value::Float(0.0);

        for &val in &values {
            let expr = make_fn_call(
                "ema",
                vec![
                    typed_expr(TypedExprKind::FloatLiteral(val), FluxType::Float),
                    typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
                ],
                FluxType::Float,
            );
            last_result = interp.eval_expr(&expr, &locals).unwrap();
        }

        match last_result {
            Value::Float(f) => assert!(
                (f - 22.5).abs() < f64::EPSILON,
                "ema([10, 20, 30], period=3) should be 22.5, got {}",
                f
            ),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    /// Test that ema first call returns the input value itself.
    #[test]
    fn test_dispatch_ema_first_value() {
        let mut interp = make_interp();
        let locals = HashMap::new();

        let expr = make_fn_call(
            "ema",
            vec![
                typed_expr(TypedExprKind::FloatLiteral(42.0), FluxType::Float),
                typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            ],
            FluxType::Float,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();

        match result {
            Value::Float(f) => assert!(
                (f - 42.0).abs() < f64::EPSILON,
                "ema first value should return the input itself (42.0), got {}",
                f
            ),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    // =========================================================================
    // HashMap evaluation tests
    // =========================================================================

    #[test]
    fn test_hashmap_new() {
        let mut interp = make_interp();
        let locals = HashMap::new();

        // HashMap.new() is routed through EnumConstruction with enum_name="HashMap", variant_name="new"
        let expr = typed_expr(
            TypedExprKind::EnumConstruction {
                enum_name: "HashMap".to_string(),
                variant_name: "new".to_string(),
                args: vec![],
            },
            FluxType::Generic("HashMap".to_string(), vec![FluxType::String, FluxType::Float]),
        );

        let result = interp.eval_expr(&expr, &locals).unwrap();
        match result {
            Value::HashMap(map) => assert!(map.is_empty(), "HashMap.new() should create empty map"),
            other => panic!("Expected Value::HashMap, got {:?}", other),
        }
    }

    #[test]
    fn test_hashmap_insert_and_get() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Create empty map and store in locals
        locals.insert("m".to_string(), Value::HashMap(HashMap::new()));

        // Call m.insert("key", 42.0)
        let receiver = typed_expr(TypedExprKind::Ident("m".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let insert_expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: "insert".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("price".to_string()), FluxType::String),
                    typed_expr(TypedExprKind::FloatLiteral(42.0), FluxType::Float),
                ],
            },
            FluxType::Generic("HashMap".to_string(), vec![]),
        );

        let result = interp.eval_expr(&insert_expr, &locals).unwrap();
        // Store result back
        locals.insert("m".to_string(), result);

        // Call m.get("price")
        let receiver2 = typed_expr(TypedExprKind::Ident("m".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let get_expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver2),
                method: "get".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("price".to_string()), FluxType::String),
                ],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&get_expr, &locals).unwrap();
        match result {
            Value::Float(f) => assert!((f - 42.0).abs() < f64::EPSILON, "get('price') should return 42.0, got {}", f),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    #[test]
    fn test_hashmap_contains_key() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Start with a map that already has "AAPL" key
        let mut map = HashMap::new();
        map.insert("AAPL".to_string(), Value::Float(150.0));
        locals.insert("registry".to_string(), Value::HashMap(map));

        // Check contains_key("AAPL") → true
        let receiver = typed_expr(TypedExprKind::Ident("registry".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: "contains_key".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("AAPL".to_string()), FluxType::String),
                ],
            },
            FluxType::Bool,
        );
        let result = interp.eval_expr(&expr, &locals).unwrap();
        assert!(matches!(result, Value::Bool(true)), "contains_key('AAPL') should be true");

        // Check contains_key("GOOG") → false
        let receiver2 = typed_expr(TypedExprKind::Ident("registry".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let expr2 = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver2),
                method: "contains_key".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("GOOG".to_string()), FluxType::String),
                ],
            },
            FluxType::Bool,
        );
        let result2 = interp.eval_expr(&expr2, &locals).unwrap();
        assert!(matches!(result2, Value::Bool(false)), "contains_key('GOOG') should be false");
    }

    #[test]
    fn test_hashmap_remove() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Create a map with two entries
        let mut map = HashMap::new();
        map.insert("AAPL".to_string(), Value::Float(150.0));
        map.insert("GOOG".to_string(), Value::Float(2800.0));
        locals.insert("m".to_string(), Value::HashMap(map));

        // Remove "AAPL"
        let receiver = typed_expr(TypedExprKind::Ident("m".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let remove_expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: "remove".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("AAPL".to_string()), FluxType::String),
                ],
            },
            FluxType::Generic("HashMap".to_string(), vec![]),
        );

        let result = interp.eval_expr(&remove_expr, &locals).unwrap();
        match result {
            Value::HashMap(new_map) => {
                assert!(!new_map.contains_key("AAPL"), "AAPL should be removed");
                assert!(new_map.contains_key("GOOG"), "GOOG should still be present");
                assert_eq!(new_map.len(), 1);
            }
            other => panic!("Expected Value::HashMap, got {:?}", other),
        }
    }

    #[test]
    fn test_hashmap_get_missing_key_returns_null() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();
        locals.insert("m".to_string(), Value::HashMap(HashMap::new()));

        // Try to get a key that doesn't exist — should return Null
        let receiver = typed_expr(TypedExprKind::Ident("m".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let get_expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: "get".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("missing".to_string()), FluxType::String),
                ],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&get_expr, &locals);
        assert!(result.is_ok(), "get on missing key should return Ok(Null)");
        assert!(matches!(result.unwrap(), Value::Null), "get on missing key should return Null");
    }

    #[test]
    fn test_hashmap_insert_overwrites_existing_key() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Create a map with one entry
        let mut map = HashMap::new();
        map.insert("price".to_string(), Value::Float(100.0));
        locals.insert("m".to_string(), Value::HashMap(map));

        // Insert same key with new value
        let receiver = typed_expr(TypedExprKind::Ident("m".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let insert_expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: "insert".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("price".to_string()), FluxType::String),
                    typed_expr(TypedExprKind::FloatLiteral(200.0), FluxType::Float),
                ],
            },
            FluxType::Generic("HashMap".to_string(), vec![]),
        );

        let result = interp.eval_expr(&insert_expr, &locals).unwrap();
        locals.insert("m".to_string(), result);

        // Verify the overwritten value
        let receiver2 = typed_expr(TypedExprKind::Ident("m".to_string()), FluxType::Generic("HashMap".to_string(), vec![]));
        let get_expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(receiver2),
                method: "get".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::StringLiteral("price".to_string()), FluxType::String),
                ],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&get_expr, &locals).unwrap();
        match result {
            Value::Float(f) => assert!((f - 200.0).abs() < f64::EPSILON, "overwritten value should be 200.0, got {}", f),
            other => panic!("Expected Value::Float, got {:?}", other),
        }
    }

    #[test]
    fn test_hashmap_display() {
        let mut map = HashMap::new();
        map.insert("alpha".to_string(), Value::Float(1.5));
        let val = Value::HashMap(map);
        let display = format!("{}", val);
        assert!(display.contains("alpha"), "display should contain key name");
        assert!(display.contains("1.5"), "display should contain value");
        assert!(display.starts_with("HashMap {"), "display should start with HashMap brace prefix");
    }

    // ========================================================================
    // Static method dispatch tests (Task 1.1)
    // ========================================================================

    #[test]
    fn test_static_method_dispatch_basic() {
        use flux_compiler::typeck::typed_ast::{TypedFnDef, TypedReturnStmt};

        let mut interp = make_interp();
        let locals = HashMap::new();

        // Register a static method: PairState.new(lookback) -> returns a Struct
        let mut pair_state_methods = HashMap::new();
        pair_state_methods.insert(
            "new".to_string(),
            TypedFnDef {
                name: "new".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec!["lookback".to_string()], // No "self" — static method
                param_types: vec![FluxType::Int],
                body: vec![
                    // return PairState { lookback: lookback, z_score: 0.0 }
                    TypedStmt::Return(TypedReturnStmt {
                        value: Some(typed_expr(
                            TypedExprKind::StructLiteral {
                                struct_name: "PairState".to_string(),
                                fields: vec![
                                    (
                                        "lookback".to_string(),
                                        typed_expr(TypedExprKind::Ident("lookback".to_string()), FluxType::Int),
                                    ),
                                    (
                                        "z_score".to_string(),
                                        typed_expr(TypedExprKind::FloatLiteral(0.0), FluxType::Float),
                                    ),
                                ],
                            },
                            FluxType::Struct("PairState".to_string()),
                        )),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Struct("PairState".to_string()),
                span: Span::new(0, 0),
            },
        );
        interp.impl_methods.insert("PairState".to_string(), pair_state_methods);

        // Call PairState.new(20)
        let method_call = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("PairState".to_string()),
                    FluxType::Struct("PairState".to_string()),
                )),
                method: "new".to_string(),
                args: vec![typed_expr(TypedExprKind::IntLiteral(20), FluxType::Int)],
            },
            FluxType::Struct("PairState".to_string()),
        );

        let result = interp.eval_expr(&method_call, &locals).unwrap();
        match result {
            Value::Struct { type_name, fields } => {
                assert_eq!(type_name, "PairState");
                assert!(matches!(fields.get("lookback"), Some(Value::Int(20))));
                assert!(matches!(fields.get("z_score"), Some(Value::Float(f)) if *f == 0.0));
            }
            other => panic!("Expected Struct, got {:?}", other),
        }
    }

    #[test]
    fn test_static_method_dispatch_error_unknown_method() {
        use flux_compiler::typeck::typed_ast::TypedFnDef;

        let mut interp = make_interp();
        let locals = HashMap::new();

        // Register a type with one method
        let mut methods = HashMap::new();
        methods.insert(
            "new".to_string(),
            TypedFnDef {
                name: "new".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec!["x".to_string()],
                param_types: vec![FluxType::Int],
                body: vec![],
                return_type: FluxType::Struct("MyType".to_string()),
                span: Span::new(0, 0),
            },
        );
        interp.impl_methods.insert("MyType".to_string(), methods);

        // Call MyType.unknown_method() — should error
        let method_call = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("MyType".to_string()),
                    FluxType::Struct("MyType".to_string()),
                )),
                method: "nonexistent".to_string(),
                args: vec![],
            },
            FluxType::Null,
        );

        let result = interp.eval_expr(&method_call, &locals);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("MyType"), "error should mention type name: {}", err);
        assert!(err.contains("nonexistent"), "error should mention method name: {}", err);
    }

    #[test]
    fn test_static_method_does_not_intercept_variable() {
        use flux_compiler::typeck::typed_ast::TypedFnDef;

        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Register a type "Foo" with a static method "bar"
        let mut methods = HashMap::new();
        methods.insert(
            "bar".to_string(),
            TypedFnDef {
                name: "bar".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec!["x".to_string()], // No self — static
                param_types: vec![FluxType::Int],
                body: vec![
                    TypedStmt::Return(flux_compiler::typeck::typed_ast::TypedReturnStmt {
                        value: Some(typed_expr(TypedExprKind::IntLiteral(999), FluxType::Int)),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Int,
                span: Span::new(0, 0),
            },
        );
        interp.impl_methods.insert("Foo".to_string(), methods);

        // Also have a local variable named "Foo" that is a Struct
        let mut foo_fields = HashMap::new();
        foo_fields.insert("val".to_string(), Value::Int(42));
        locals.insert("Foo".to_string(), Value::Struct {
            type_name: "Foo".to_string(),
            fields: foo_fields,
        });

        // Calling Foo.bar(1) — since "Foo" is in impl_methods AND the method is static,
        // it should dispatch statically (type-name takes priority for static methods)
        let method_call = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("Foo".to_string()),
                    FluxType::Struct("Foo".to_string()),
                )),
                method: "bar".to_string(),
                args: vec![typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)],
            },
            FluxType::Int,
        );

        let result = interp.eval_expr(&method_call, &locals).unwrap();
        assert!(matches!(result, Value::Int(999)));
    }

    // ========================================================================
    // Instance method self.method() nested dispatch test (Task 4.1)
    // ========================================================================

    #[test]
    fn test_instance_method_self_nested_dispatch() {
        use flux_compiler::typeck::typed_ast::{TypedFnDef, TypedReturnStmt};

        let mut interp = make_interp();
        let _locals: HashMap<String, Value> = HashMap::new();

        // Define a struct type "Calculator" with two instance methods:
        //   - get_value(self) -> self.value
        //   - double_value(self) -> self.get_value() * 2
        //
        // This tests that within a method body, calling self.get_value()
        // properly dispatches through impl_methods using the struct's type_name.

        let mut calc_methods = HashMap::new();

        // Method: get_value(self) -> returns self.value
        calc_methods.insert(
            "get_value".to_string(),
            TypedFnDef {
                name: "get_value".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec!["self".to_string()],
                param_types: vec![FluxType::Struct("Calculator".to_string())],
                body: vec![
                    // return self.value
                    TypedStmt::Return(TypedReturnStmt {
                        value: Some(typed_expr(
                            TypedExprKind::MemberAccess {
                                object: Box::new(typed_expr(
                                    TypedExprKind::Ident("self".to_string()),
                                    FluxType::Struct("Calculator".to_string()),
                                )),
                                field: "value".to_string(),
                            },
                            FluxType::Float,
                        )),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Float,
                span: Span::new(0, 0),
            },
        );

        // Method: double_value(self) -> self.get_value() * 2.0
        calc_methods.insert(
            "double_value".to_string(),
            TypedFnDef {
                name: "double_value".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec!["self".to_string()],
                param_types: vec![FluxType::Struct("Calculator".to_string())],
                body: vec![
                    // return self.get_value() * 2.0
                    TypedStmt::Return(TypedReturnStmt {
                        value: Some(typed_expr(
                            TypedExprKind::BinaryOp {
                                left: Box::new(typed_expr(
                                    TypedExprKind::MethodCall {
                                        receiver: Box::new(typed_expr(
                                            TypedExprKind::Ident("self".to_string()),
                                            FluxType::Struct("Calculator".to_string()),
                                        )),
                                        method: "get_value".to_string(),
                                        args: vec![],
                                    },
                                    FluxType::Float,
                                )),
                                op: BinOp::Mul,
                                right: Box::new(typed_expr(
                                    TypedExprKind::FloatLiteral(2.0),
                                    FluxType::Float,
                                )),
                            },
                            FluxType::Float,
                        )),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Float,
                span: Span::new(0, 0),
            },
        );

        interp.impl_methods.insert("Calculator".to_string(), calc_methods);

        // Create a Calculator struct value with value = 21.0
        let mut calc_fields = HashMap::new();
        calc_fields.insert("value".to_string(), Value::Float(21.0));
        let calc_value = Value::Struct {
            type_name: "Calculator".to_string(),
            fields: calc_fields,
        };

        // Store calc in locals so we can call methods on it
        let mut test_locals = HashMap::new();
        test_locals.insert("calc".to_string(), calc_value);

        // Call calc.double_value() — this should:
        // 1. Dispatch to Calculator's double_value method
        // 2. Bind self = calc (the Value::Struct)
        // 3. In the method body, evaluate self.get_value() which:
        //    a. Looks up "self" in method_locals → finds Value::Struct { type_name: "Calculator" }
        //    b. Dispatches through impl_methods["Calculator"]["get_value"]
        //    c. Returns self.value = 21.0
        // 4. Multiply by 2.0 → 42.0
        let method_call = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("calc".to_string()),
                    FluxType::Struct("Calculator".to_string()),
                )),
                method: "double_value".to_string(),
                args: vec![],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&method_call, &test_locals).unwrap();
        match result {
            Value::Float(f) => assert!(
                (f - 42.0).abs() < f64::EPSILON,
                "Expected 42.0, got {}",
                f
            ),
            other => panic!("Expected Value::Float(42.0), got {:?}", other),
        }
    }

    #[test]
    fn test_instance_method_self_nested_dispatch_with_args() {
        use flux_compiler::typeck::typed_ast::{TypedFnDef, TypedReturnStmt};

        let mut interp = make_interp();

        // Define a struct type "PairState" with two instance methods:
        //   - calculate_zscore(self, spread, avg, std) -> (spread - avg) / std
        //   - update(self, spread, avg, std) -> self.calculate_zscore(spread, avg, std)
        //
        // This mimics the pairs_trading pattern where update() calls self.calculate_zscore().

        let mut pair_methods = HashMap::new();

        // Method: calculate_zscore(self, spread, avg, std_dev) -> (spread - avg) / std_dev
        pair_methods.insert(
            "calculate_zscore".to_string(),
            TypedFnDef {
                name: "calculate_zscore".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec![
                    "self".to_string(),
                    "spread".to_string(),
                    "avg".to_string(),
                    "std_dev".to_string(),
                ],
                param_types: vec![
                    FluxType::Struct("PairState".to_string()),
                    FluxType::Float,
                    FluxType::Float,
                    FluxType::Float,
                ],
                body: vec![
                    // return (spread - avg) / std_dev
                    TypedStmt::Return(TypedReturnStmt {
                        value: Some(typed_expr(
                            TypedExprKind::BinaryOp {
                                left: Box::new(typed_expr(
                                    TypedExprKind::BinaryOp {
                                        left: Box::new(typed_expr(
                                            TypedExprKind::Ident("spread".to_string()),
                                            FluxType::Float,
                                        )),
                                        op: BinOp::Sub,
                                        right: Box::new(typed_expr(
                                            TypedExprKind::Ident("avg".to_string()),
                                            FluxType::Float,
                                        )),
                                    },
                                    FluxType::Float,
                                )),
                                op: BinOp::Div,
                                right: Box::new(typed_expr(
                                    TypedExprKind::Ident("std_dev".to_string()),
                                    FluxType::Float,
                                )),
                            },
                            FluxType::Float,
                        )),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Float,
                span: Span::new(0, 0),
            },
        );

        // Method: update(self, spread, avg, std_dev) -> self.calculate_zscore(spread, avg, std_dev)
        pair_methods.insert(
            "update".to_string(),
            TypedFnDef {
                name: "update".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec![
                    "self".to_string(),
                    "spread".to_string(),
                    "avg".to_string(),
                    "std_dev".to_string(),
                ],
                param_types: vec![
                    FluxType::Struct("PairState".to_string()),
                    FluxType::Float,
                    FluxType::Float,
                    FluxType::Float,
                ],
                body: vec![
                    // return self.calculate_zscore(spread, avg, std_dev)
                    TypedStmt::Return(TypedReturnStmt {
                        value: Some(typed_expr(
                            TypedExprKind::MethodCall {
                                receiver: Box::new(typed_expr(
                                    TypedExprKind::Ident("self".to_string()),
                                    FluxType::Struct("PairState".to_string()),
                                )),
                                method: "calculate_zscore".to_string(),
                                args: vec![
                                    typed_expr(TypedExprKind::Ident("spread".to_string()), FluxType::Float),
                                    typed_expr(TypedExprKind::Ident("avg".to_string()), FluxType::Float),
                                    typed_expr(TypedExprKind::Ident("std_dev".to_string()), FluxType::Float),
                                ],
                            },
                            FluxType::Float,
                        )),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Float,
                span: Span::new(0, 0),
            },
        );

        interp.impl_methods.insert("PairState".to_string(), pair_methods);

        // Create a PairState struct value
        let mut pair_fields = HashMap::new();
        pair_fields.insert("lookback".to_string(), Value::Int(20));
        pair_fields.insert("z_score".to_string(), Value::Float(0.0));
        let pair_value = Value::Struct {
            type_name: "PairState".to_string(),
            fields: pair_fields,
        };

        let mut test_locals = HashMap::new();
        test_locals.insert("pair".to_string(), pair_value);

        // Call pair.update(10.0, 8.0, 2.0)
        // Expected: self.calculate_zscore(10.0, 8.0, 2.0) = (10.0 - 8.0) / 2.0 = 1.0
        let method_call = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("pair".to_string()),
                    FluxType::Struct("PairState".to_string()),
                )),
                method: "update".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::FloatLiteral(10.0), FluxType::Float),
                    typed_expr(TypedExprKind::FloatLiteral(8.0), FluxType::Float),
                    typed_expr(TypedExprKind::FloatLiteral(2.0), FluxType::Float),
                ],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&method_call, &test_locals).unwrap();
        match result {
            Value::Float(f) => assert!(
                (f - 1.0).abs() < f64::EPSILON,
                "Expected 1.0, got {}",
                f
            ),
            other => panic!("Expected Value::Float(1.0), got {:?}", other),
        }
    }

    // ========================================================================
    // Trait method dispatch tests (Task 5.1)
    // ========================================================================

    /// Test that a trait method registered via `entry().or_insert_with()` is callable
    /// on a struct instance through the impl_methods registry.
    /// This confirms that trait impl methods are dispatched at runtime via the
    /// concrete type_name from Value::Struct.
    /// Requirements: 8.1, 8.2
    #[test]
    fn test_trait_method_callable_on_struct_instance() {
        use flux_compiler::typeck::typed_ast::{TypedFnDef, TypedReturnStmt};

        let mut interp = make_interp();

        // Register a trait impl method: impl RegimeDetector for TrendDetector
        //   fn detect(self, fast_avg, slow_avg, volatility) -> returns Float(1.0)
        let mut trend_methods = HashMap::new();
        trend_methods.insert(
            "detect".to_string(),
            TypedFnDef {
                name: "detect".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec![
                    "self".to_string(),
                    "fast_avg".to_string(),
                    "slow_avg".to_string(),
                    "volatility".to_string(),
                ],
                param_types: vec![
                    FluxType::Struct("TrendDetector".to_string()),
                    FluxType::Float,
                    FluxType::Float,
                    FluxType::Float,
                ],
                body: vec![
                    // return 1.0 (simplified — just proves dispatch works)
                    TypedStmt::Return(TypedReturnStmt {
                        value: Some(typed_expr(TypedExprKind::FloatLiteral(1.0), FluxType::Float)),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Float,
                span: Span::new(0, 0),
            },
        );
        interp.impl_methods.insert("TrendDetector".to_string(), trend_methods);

        // Create a TrendDetector struct value
        let mut det_fields = HashMap::new();
        det_fields.insert("crossover_pct".to_string(), Value::Float(0.02));
        let det_value = Value::Struct {
            type_name: "TrendDetector".to_string(),
            fields: det_fields,
        };

        // Store in locals
        let mut test_locals = HashMap::new();
        test_locals.insert("detector".to_string(), det_value);

        // Call detector.detect(10.0, 9.5, 0.5)
        let method_call = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("detector".to_string()),
                    FluxType::Struct("TrendDetector".to_string()),
                )),
                method: "detect".to_string(),
                args: vec![
                    typed_expr(TypedExprKind::FloatLiteral(10.0), FluxType::Float),
                    typed_expr(TypedExprKind::FloatLiteral(9.5), FluxType::Float),
                    typed_expr(TypedExprKind::FloatLiteral(0.5), FluxType::Float),
                ],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&method_call, &test_locals).unwrap();
        match result {
            Value::Float(f) => assert!(
                (f - 1.0).abs() < f64::EPSILON,
                "Expected 1.0, got {}",
                f
            ),
            other => panic!("Expected Value::Float(1.0), got {:?}", other),
        }
    }

    /// Test that inherent methods take priority over trait methods with the same name.
    /// When both an inherent and a trait impl method exist for the same name, the
    /// inherent method body should execute.
    /// Requirement: 8.3
    #[test]
    fn test_inherent_method_priority_over_trait() {
        use flux_compiler::typeck::typed_ast::{TypedFnDef, TypedReturnStmt};

        let mut interp = make_interp();

        // Simulate the Interpreter::new registration behavior:
        // First register inherent impl (uses insert — unconditional)
        // Then register trait impl (uses entry().or_insert — only if absent)
        let mut type_methods: HashMap<String, TypedFnDef> = HashMap::new();

        // Inherent method: detect(self) -> returns 42.0
        let inherent_method = TypedFnDef {
            name: "detect".to_string(),
            type_params: vec![],
            type_param_bounds: vec![],
            params: vec!["self".to_string()],
            param_types: vec![FluxType::Struct("MyDetector".to_string())],
            body: vec![
                TypedStmt::Return(TypedReturnStmt {
                    value: Some(typed_expr(TypedExprKind::FloatLiteral(42.0), FluxType::Float)),
                    span: Span::new(0, 0),
                }),
            ],
            return_type: FluxType::Float,
            span: Span::new(0, 0),
        };

        // Trait method: detect(self) -> returns 99.0 (should NOT be used)
        let trait_method = TypedFnDef {
            name: "detect".to_string(),
            type_params: vec![],
            type_param_bounds: vec![],
            params: vec!["self".to_string()],
            param_types: vec![FluxType::Struct("MyDetector".to_string())],
            body: vec![
                TypedStmt::Return(TypedReturnStmt {
                    value: Some(typed_expr(TypedExprKind::FloatLiteral(99.0), FluxType::Float)),
                    span: Span::new(0, 0),
                }),
            ],
            return_type: FluxType::Float,
            span: Span::new(0, 0),
        };

        // Simulate inherent registration: insert() always overwrites
        type_methods.insert("detect".to_string(), inherent_method);
        // Simulate trait registration: entry().or_insert() — should NOT overwrite
        type_methods.entry("detect".to_string()).or_insert(trait_method);

        interp.impl_methods.insert("MyDetector".to_string(), type_methods);

        // Create a MyDetector struct and call detect
        let mut det_fields = HashMap::new();
        det_fields.insert("x".to_string(), Value::Int(1));
        let det_value = Value::Struct {
            type_name: "MyDetector".to_string(),
            fields: det_fields,
        };

        let mut test_locals = HashMap::new();
        test_locals.insert("det".to_string(), det_value);

        let method_call = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("det".to_string()),
                    FluxType::Struct("MyDetector".to_string()),
                )),
                method: "detect".to_string(),
                args: vec![],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&method_call, &test_locals).unwrap();
        match result {
            Value::Float(f) => assert!(
                (f - 42.0).abs() < f64::EPSILON,
                "Expected inherent method to return 42.0, got {} (trait method would return 99.0)",
                f
            ),
            other => panic!("Expected Value::Float(42.0), got {:?}", other),
        }
    }

    /// Test that a generic function parameter resolves its concrete type for dispatch.
    /// Simulates: fn detect_regime[T: RegimeDetector](detector: T, ...) calling
    /// detector.detect(...) — the concrete type should be determined from Value::Struct.
    /// Requirements: 8.2, 8.3
    #[test]
    fn test_generic_function_parameter_dispatches_via_concrete_type() {
        use flux_compiler::typeck::typed_ast::{TypedFnDef, TypedReturnStmt};

        let mut interp = make_interp();

        // Register a trait impl method for "TrendDetector":
        // detect(self, fast, slow, vol) -> returns self.crossover_pct (proving self is bound)
        let mut trend_methods = HashMap::new();
        trend_methods.insert(
            "detect".to_string(),
            TypedFnDef {
                name: "detect".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec![
                    "self".to_string(),
                    "fast".to_string(),
                    "slow".to_string(),
                    "vol".to_string(),
                ],
                param_types: vec![
                    FluxType::Struct("TrendDetector".to_string()),
                    FluxType::Float,
                    FluxType::Float,
                    FluxType::Float,
                ],
                body: vec![
                    // return self.crossover_pct
                    TypedStmt::Return(TypedReturnStmt {
                        value: Some(typed_expr(
                            TypedExprKind::MemberAccess {
                                object: Box::new(typed_expr(
                                    TypedExprKind::Ident("self".to_string()),
                                    FluxType::Struct("TrendDetector".to_string()),
                                )),
                                field: "crossover_pct".to_string(),
                            },
                            FluxType::Float,
                        )),
                        span: Span::new(0, 0),
                    }),
                ],
                return_type: FluxType::Float,
                span: Span::new(0, 0),
            },
        );
        interp.impl_methods.insert("TrendDetector".to_string(), trend_methods);

        // Register a generic function: detect_regime(detector, fast, slow, vol)
        // Body: return detector.detect(fast, slow, vol)
        let generic_fn = TypedFnDef {
            name: "detect_regime".to_string(),
            type_params: vec!["T".to_string()],
            type_param_bounds: vec![Some("RegimeDetector".to_string())],
            params: vec![
                "detector".to_string(),
                "fast".to_string(),
                "slow".to_string(),
                "vol".to_string(),
            ],
            param_types: vec![
                FluxType::TypeParam("T".to_string()),
                FluxType::Float,
                FluxType::Float,
                FluxType::Float,
            ],
            body: vec![
                // return detector.detect(fast, slow, vol)
                TypedStmt::Return(TypedReturnStmt {
                    value: Some(typed_expr(
                        TypedExprKind::MethodCall {
                            receiver: Box::new(typed_expr(
                                TypedExprKind::Ident("detector".to_string()),
                                FluxType::TypeParam("T".to_string()),
                            )),
                            method: "detect".to_string(),
                            args: vec![
                                typed_expr(TypedExprKind::Ident("fast".to_string()), FluxType::Float),
                                typed_expr(TypedExprKind::Ident("slow".to_string()), FluxType::Float),
                                typed_expr(TypedExprKind::Ident("vol".to_string()), FluxType::Float),
                            ],
                        },
                        FluxType::Float,
                    )),
                    span: Span::new(0, 0),
                }),
            ],
            return_type: FluxType::Float,
            span: Span::new(0, 0),
        };
        interp.functions.insert("detect_regime".to_string(), generic_fn);

        // Create a TrendDetector struct value with crossover_pct = 0.05
        let mut det_fields = HashMap::new();
        det_fields.insert("crossover_pct".to_string(), Value::Float(0.05));
        let det_value = Value::Struct {
            type_name: "TrendDetector".to_string(),
            fields: det_fields,
        };

        let mut test_locals = HashMap::new();
        test_locals.insert("trend_det".to_string(), det_value);

        // Call detect_regime(trend_det, 10.0, 9.0, 1.0)
        // This should:
        // 1. Evaluate trend_det → Value::Struct { type_name: "TrendDetector", ... }
        // 2. Bind detector = that struct value in fn_locals
        // 3. In the function body, evaluate detector.detect(fast, slow, vol)
        // 4. Evaluate detector → Value::Struct { type_name: "TrendDetector" }
        // 5. Dispatch via impl_methods["TrendDetector"]["detect"]
        // 6. Bind self = detector struct, access self.crossover_pct → 0.05
        let fn_call = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("detect_regime".to_string()),
                    FluxType::Float,
                )),
                args: vec![
                    typed_expr(TypedExprKind::Ident("trend_det".to_string()), FluxType::TypeParam("T".to_string())),
                    typed_expr(TypedExprKind::FloatLiteral(10.0), FluxType::Float),
                    typed_expr(TypedExprKind::FloatLiteral(9.0), FluxType::Float),
                    typed_expr(TypedExprKind::FloatLiteral(1.0), FluxType::Float),
                ],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&fn_call, &test_locals).unwrap();
        match result {
            Value::Float(f) => assert!(
                (f - 0.05).abs() < f64::EPSILON,
                "Expected 0.05 (self.crossover_pct), got {}. \
                 This means the generic function correctly dispatched detector.detect() \
                 to TrendDetector's impl via the concrete type_name.",
                f
            ),
            other => panic!("Expected Value::Float(0.05), got {:?}", other),
        }
    }

    // ========================================================================
    // HashMap implicit reassignment tests (Task 2.2)
    // ========================================================================

    #[test]
    fn test_hashmap_implicit_reassignment_insert_in_locals() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Pre-populate locals with an empty HashMap
        locals.insert("registry".to_string(), Value::HashMap(HashMap::new()));

        // Build expression statement: registry.insert("AAPL", 150.0)
        let receiver = typed_expr(
            TypedExprKind::Ident("registry".to_string()),
            FluxType::Generic("HashMap".to_string(), vec![]),
        );
        let expr_stmt = TypedStmt::Expr(TypedExprStmt {
            expr: typed_expr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "insert".to_string(),
                    args: vec![
                        typed_expr(TypedExprKind::StringLiteral("AAPL".to_string()), FluxType::String),
                        typed_expr(TypedExprKind::FloatLiteral(150.0), FluxType::Float),
                    ],
                },
                FluxType::Generic("HashMap".to_string(), vec![]),
            ),
            span: Span::new(0, 0),
        });

        let signals = interp.exec_stmt(&expr_stmt, &mut locals).unwrap();
        assert!(signals.is_empty());

        // The implicit reassignment should have updated "registry" in locals
        match locals.get("registry") {
            Some(Value::HashMap(map)) => {
                assert!(map.contains_key("AAPL"), "registry should contain 'AAPL' after implicit reassignment");
                match map.get("AAPL") {
                    Some(Value::Float(f)) => assert!((f - 150.0).abs() < f64::EPSILON),
                    other => panic!("Expected Value::Float(150.0), got {:?}", other),
                }
            }
            other => panic!("Expected Value::HashMap in locals, got {:?}", other),
        }
    }

    #[test]
    fn test_hashmap_implicit_reassignment_insert_in_state() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Pre-populate state with an empty HashMap
        interp.state.insert("pair_registry".to_string(), Value::HashMap(HashMap::new()));

        // Build expression statement: pair_registry.insert("symbol", 1.0)
        let receiver = typed_expr(
            TypedExprKind::Ident("pair_registry".to_string()),
            FluxType::Generic("HashMap".to_string(), vec![]),
        );
        let expr_stmt = TypedStmt::Expr(TypedExprStmt {
            expr: typed_expr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "insert".to_string(),
                    args: vec![
                        typed_expr(TypedExprKind::StringLiteral("symbol".to_string()), FluxType::String),
                        typed_expr(TypedExprKind::FloatLiteral(1.0), FluxType::Float),
                    ],
                },
                FluxType::Generic("HashMap".to_string(), vec![]),
            ),
            span: Span::new(0, 0),
        });

        let signals = interp.exec_stmt(&expr_stmt, &mut locals).unwrap();
        assert!(signals.is_empty());

        // The implicit reassignment should have updated "pair_registry" in state
        match interp.state.get("pair_registry") {
            Some(Value::HashMap(map)) => {
                assert!(map.contains_key("symbol"), "pair_registry in state should contain 'symbol'");
                match map.get("symbol") {
                    Some(Value::Float(f)) => assert!((f - 1.0).abs() < f64::EPSILON),
                    other => panic!("Expected Value::Float(1.0), got {:?}", other),
                }
            }
            other => panic!("Expected Value::HashMap in state, got {:?}", other),
        }
        // Locals should NOT have pair_registry
        assert!(locals.get("pair_registry").is_none());
    }

    #[test]
    fn test_hashmap_implicit_reassignment_remove_in_locals() {
        let mut interp = make_interp();
        let mut locals = HashMap::new();

        // Pre-populate locals with a HashMap that has a key
        let mut map = HashMap::new();
        map.insert("AAPL".to_string(), Value::Float(150.0));
        locals.insert("registry".to_string(), Value::HashMap(map));

        // Build expression statement: registry.remove("AAPL")
        let receiver = typed_expr(
            TypedExprKind::Ident("registry".to_string()),
            FluxType::Generic("HashMap".to_string(), vec![]),
        );
        let expr_stmt = TypedStmt::Expr(TypedExprStmt {
            expr: typed_expr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "remove".to_string(),
                    args: vec![
                        typed_expr(TypedExprKind::StringLiteral("AAPL".to_string()), FluxType::String),
                    ],
                },
                FluxType::Generic("HashMap".to_string(), vec![]),
            ),
            span: Span::new(0, 0),
        });

        let signals = interp.exec_stmt(&expr_stmt, &mut locals).unwrap();
        assert!(signals.is_empty());

        // The implicit reassignment should have updated "registry" with the key removed
        match locals.get("registry") {
            Some(Value::HashMap(map)) => {
                assert!(!map.contains_key("AAPL"), "registry should NOT contain 'AAPL' after remove");
                assert!(map.is_empty(), "registry should be empty after removing the only key");
            }
            other => panic!("Expected Value::HashMap in locals, got {:?}", other),
        }
    }
}
