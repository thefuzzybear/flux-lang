use super::types::{FluxType, FnParams};

/// Market data identifiers available inside event handlers.
///
/// These bindings are injected into the handler-level scope when type-checking
/// event handler bodies. They represent real-time market data fields.
pub(crate) fn market_data_bindings() -> Vec<(&'static str, FluxType)> {
    vec![
        ("close", FluxType::Float),
        ("open", FluxType::Float),
        ("high", FluxType::Float),
        ("low", FluxType::Float),
        ("volume", FluxType::Float),
        ("symbol", FluxType::String),
        ("in_position", FluxType::Bool),
    ]
}

/// Trading signal functions available inside event handlers.
///
/// `OPEN` takes a symbol (String) and quantity (Float) and returns a Signal.
/// `CLOSE` is overloaded: it accepts either just a symbol (String), or a symbol
/// and quantity (String, Float). Both forms return a Signal.
pub(crate) fn signal_function_bindings() -> Vec<(&'static str, FluxType)> {
    vec![
        ("OPEN", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::String, FluxType::Float]),
            ret: Box::new(FluxType::Signal),
        }),
        ("CLOSE", FluxType::Fn {
            params: FnParams::Overloaded(vec![
                vec![FluxType::String],
                vec![FluxType::String, FluxType::Float],
            ]),
            ret: Box::new(FluxType::Signal),
        }),
        ("CLOSE_QTY", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::String, FluxType::Float]),
            ret: Box::new(FluxType::Signal),
        }),
        ("SHORT", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::String, FluxType::Float]),
            ret: Box::new(FluxType::Signal),
        }),
    ]
}

/// Math, statistics, and portfolio function bindings.
///
/// These are injected into the type environment alongside signal bindings,
/// making them available without explicit imports. Organized in three tiers:
///
/// - **Tier 1**: Core math builtins (abs, sqrt, exp, log, floor, ceil, round, sign, pow, min, max)
/// - **Tier 2**: Rolling statistical indicators (stddev, variance, zscore, rsi, corr, covariance, atr)
/// - **Tier 3**: Matrix operations and portfolio functions (mat_mul, transpose, inverse, det,
///   cov_matrix, corr_matrix, min_variance_weights, portfolio_var, sharpe)
pub(crate) fn math_function_bindings() -> Vec<(&'static str, FluxType)> {
    vec![
        // ── Tier 1: Core Math — single-argument (VariadicNumeric → Float) ──
        ("abs", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("sqrt", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("exp", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("log", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("floor", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("ceil", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("round", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("sign", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),

        // ── Tier 1: Core Math — multi-argument (VariadicNumeric → Float) ──
        ("pow", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("min", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("max", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),

        // ── Tier 2: Rolling Statistical Indicators (VariadicNumeric → Float) ──
        ("stddev", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("variance", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("zscore", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("rsi", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("corr", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("covariance", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),
        ("atr", FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        }),

        // ── Tier 3: Matrix Operations ──
        ("mat_mul", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::MatFloat, FluxType::MatFloat]),
            ret: Box::new(FluxType::MatFloat),
        }),
        ("transpose", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::MatFloat]),
            ret: Box::new(FluxType::MatFloat),
        }),
        ("inverse", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::MatFloat]),
            ret: Box::new(FluxType::MatFloat),
        }),
        ("det", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::MatFloat]),
            ret: Box::new(FluxType::Float),
        }),

        // ── Tier 3: Portfolio — Covariance/Correlation Matrices ──
        ("cov_matrix", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::VecFloat, FluxType::Int]),
            ret: Box::new(FluxType::MatFloat),
        }),
        ("corr_matrix", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::VecFloat, FluxType::Int]),
            ret: Box::new(FluxType::MatFloat),
        }),

        // ── Tier 3: Portfolio — Optimization & Risk ──
        ("min_variance_weights", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::MatFloat, FluxType::VecFloat]),
            ret: Box::new(FluxType::VecFloat),
        }),
        ("portfolio_var", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::VecFloat, FluxType::MatFloat]),
            ret: Box::new(FluxType::Float),
        }),
        ("sharpe", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::VecFloat, FluxType::Float]),
            ret: Box::new(FluxType::Float),
        }),

        // ── Cross-Asset Data Accessor ──
        ("ret", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::String]),
            ret: Box::new(FluxType::Float),
        }),

        // ── Iteration Utilities ──
        ("range", FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::Int, FluxType::Int]),
            ret: Box::new(FluxType::List(Box::new(FluxType::Int))),
        }),
    ]
}

/// Recognized event handler names.
///
/// Only event names in this list are accepted by the type checker.
/// The parser strips the `on_` prefix from event handler syntax (e.g., `on_bar`
/// becomes event_name = "bar"), so these entries use the stripped form.
pub(crate) fn valid_event_names() -> &'static [&'static str] {
    &["bar"]
}
