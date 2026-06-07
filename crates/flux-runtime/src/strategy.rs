use crate::context::BarContext;
use crate::signal::Signal;

/// The trait that all generated Flux strategy structs implement.
///
/// Object-safe: supports `Box<dyn Strategy>`.
pub trait Strategy {
    /// Process a single bar and return zero or more trade signals.
    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time check: Strategy must be object-safe
    fn _assert_object_safe(_: Box<dyn Strategy>) {}
}
