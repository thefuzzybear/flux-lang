/// A trade signal emitted by a strategy.
#[derive(Debug, Clone)]
pub enum Signal {
    /// Open a new position
    Open { symbol: String, qty: f64 },
    /// Close the entire position
    Close { symbol: String },
    /// Close a partial quantity
    CloseQty { symbol: String, qty: f64 },
}

impl Signal {
    /// Create a signal to open a position.
    /// Panics if qty <= 0.0.
    pub fn open(symbol: String, qty: f64) -> Self {
        assert!(qty > 0.0, "Signal::open qty must be greater than 0.0");
        Signal::Open { symbol, qty }
    }

    /// Create a signal to close the entire position for a symbol.
    pub fn close(symbol: String) -> Self {
        Signal::Close { symbol }
    }

    /// Create a signal to close a partial position.
    /// Panics if qty <= 0.0.
    pub fn close_qty(symbol: String, qty: f64) -> Self {
        assert!(qty > 0.0, "Signal::close_qty qty must be greater than 0.0");
        Signal::CloseQty { symbol, qty }
    }

    /// Get the symbol for this signal.
    pub fn symbol(&self) -> &str {
        match self {
            Signal::Open { symbol, .. } => symbol,
            Signal::Close { symbol } => symbol,
            Signal::CloseQty { symbol, .. } => symbol,
        }
    }

    /// Get the quantity, if applicable.
    /// Returns `Some(qty)` for Open and CloseQty, `None` for Close.
    pub fn qty(&self) -> Option<f64> {
        match self {
            Signal::Open { qty, .. } => Some(*qty),
            Signal::Close { .. } => None,
            Signal::CloseQty { qty, .. } => Some(*qty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_open_construction() {
        let signal = Signal::open("AAPL".to_string(), 100.0);
        assert!(matches!(signal, Signal::Open { ref symbol, qty } if symbol == "AAPL" && qty == 100.0));
    }

    #[test]
    fn test_close_construction() {
        let signal = Signal::close("MSFT".to_string());
        assert!(matches!(signal, Signal::Close { ref symbol } if symbol == "MSFT"));
    }

    #[test]
    fn test_close_qty_construction() {
        let signal = Signal::close_qty("GOOG".to_string(), 50.0);
        assert!(matches!(signal, Signal::CloseQty { ref symbol, qty } if symbol == "GOOG" && qty == 50.0));
    }

    #[test]
    fn test_symbol_accessor_open() {
        let signal = Signal::open("AAPL".to_string(), 10.0);
        assert_eq!(signal.symbol(), "AAPL");
    }

    #[test]
    fn test_symbol_accessor_close() {
        let signal = Signal::close("TSLA".to_string());
        assert_eq!(signal.symbol(), "TSLA");
    }

    #[test]
    fn test_symbol_accessor_close_qty() {
        let signal = Signal::close_qty("NVDA".to_string(), 25.0);
        assert_eq!(signal.symbol(), "NVDA");
    }

    #[test]
    fn test_qty_accessor_open() {
        let signal = Signal::open("AAPL".to_string(), 42.5);
        assert_eq!(signal.qty(), Some(42.5));
    }

    #[test]
    fn test_qty_accessor_close() {
        let signal = Signal::close("AAPL".to_string());
        assert_eq!(signal.qty(), None);
    }

    #[test]
    fn test_qty_accessor_close_qty() {
        let signal = Signal::close_qty("AAPL".to_string(), 7.0);
        assert_eq!(signal.qty(), Some(7.0));
    }

    #[test]
    fn test_clone() {
        let signal = Signal::open("SPY".to_string(), 200.0);
        let cloned = signal.clone();
        assert_eq!(cloned.symbol(), "SPY");
        assert_eq!(cloned.qty(), Some(200.0));
    }

    #[test]
    fn test_debug() {
        let signal = Signal::open("AAPL".to_string(), 1.0);
        let debug_str = format!("{:?}", signal);
        assert!(debug_str.contains("Open"));
        assert!(debug_str.contains("AAPL"));
    }

    #[test]
    #[should_panic(expected = "Signal::open qty must be greater than 0.0")]
    fn test_open_panics_on_zero_qty() {
        Signal::open("AAPL".to_string(), 0.0);
    }

    #[test]
    #[should_panic(expected = "Signal::open qty must be greater than 0.0")]
    fn test_open_panics_on_negative_qty() {
        Signal::open("AAPL".to_string(), -5.0);
    }

    #[test]
    #[should_panic(expected = "Signal::close_qty qty must be greater than 0.0")]
    fn test_close_qty_panics_on_zero_qty() {
        Signal::close_qty("AAPL".to_string(), 0.0);
    }

    #[test]
    #[should_panic(expected = "Signal::close_qty qty must be greater than 0.0")]
    fn test_close_qty_panics_on_negative_qty() {
        Signal::close_qty("AAPL".to_string(), -10.0);
    }

    // Feature: flux-runtime, Property 1: Signal Constructor Round-Trip
    // **Validates: Requirements 3.2, 3.3, 3.4, 3.6**
    proptest! {
        #[test]
        fn prop_signal_round_trip(
            symbol in "[A-Z]{1,5}",
            qty in 0.01..10000.0f64,
        ) {
            let open_sig = Signal::open(symbol.clone(), qty);
            prop_assert_eq!(open_sig.symbol(), symbol.as_str());
            prop_assert_eq!(open_sig.qty(), Some(qty));

            let close_sig = Signal::close(symbol.clone());
            prop_assert_eq!(close_sig.symbol(), symbol.as_str());
            prop_assert_eq!(close_sig.qty(), None);

            let close_qty_sig = Signal::close_qty(symbol.clone(), qty);
            prop_assert_eq!(close_qty_sig.symbol(), symbol.as_str());
            prop_assert_eq!(close_qty_sig.qty(), Some(qty));
        }
    }
}
