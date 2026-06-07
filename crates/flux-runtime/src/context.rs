/// Market data for a single bar, passed to strategy event handlers.
#[derive(Debug, Clone)]
pub struct BarContext {
    /// Bar closing price
    pub close: f64,
    /// Bar opening price
    pub open: f64,
    /// Bar high price
    pub high: f64,
    /// Bar low price
    pub low: f64,
    /// Bar volume
    pub volume: f64,
    /// Instrument symbol
    pub symbol: String,
    /// Whether the strategy currently holds a position
    pub in_position: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construction() {
        let ctx = BarContext {
            close: 150.0,
            open: 148.0,
            high: 152.0,
            low: 147.0,
            volume: 1_000_000.0,
            symbol: "AAPL".to_string(),
            in_position: false,
        };

        assert_eq!(ctx.close, 150.0);
        assert_eq!(ctx.open, 148.0);
        assert_eq!(ctx.high, 152.0);
        assert_eq!(ctx.low, 147.0);
        assert_eq!(ctx.volume, 1_000_000.0);
        assert_eq!(ctx.symbol, "AAPL");
        assert_eq!(ctx.in_position, false);
    }

    #[test]
    fn test_clone_equality() {
        let ctx = BarContext {
            close: 100.5,
            open: 99.0,
            high: 101.0,
            low: 98.5,
            volume: 500_000.0,
            symbol: "MSFT".to_string(),
            in_position: true,
        };

        let cloned = ctx.clone();

        assert_eq!(cloned.close, ctx.close);
        assert_eq!(cloned.open, ctx.open);
        assert_eq!(cloned.high, ctx.high);
        assert_eq!(cloned.low, ctx.low);
        assert_eq!(cloned.volume, ctx.volume);
        assert_eq!(cloned.symbol, ctx.symbol);
        assert_eq!(cloned.in_position, ctx.in_position);
    }

    #[test]
    fn test_debug_formatting() {
        let ctx = BarContext {
            close: 50.0,
            open: 49.0,
            high: 51.0,
            low: 48.0,
            volume: 200.0,
            symbol: "TSLA".to_string(),
            in_position: false,
        };

        let debug_str = format!("{:?}", ctx);

        assert!(debug_str.contains("BarContext"));
        assert!(debug_str.contains("close"));
        assert!(debug_str.contains("50.0"));
        assert!(debug_str.contains("TSLA"));
        assert!(debug_str.contains("in_position"));
    }
}
