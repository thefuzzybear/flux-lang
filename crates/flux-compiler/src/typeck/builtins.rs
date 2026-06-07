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
