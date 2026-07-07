// Feature: flux-run-harness, Property 6: OhlcvRecord to BarContext field preservation
//
// **Validates: Requirements 5.7**
//
// For any valid OhlcvRecord (with finite positive f64 values for OHLCV fields and non-empty
// symbol string), converting it via ohlcv_to_bars SHALL produce a BarContext where close, open,
// high, low, volume, and symbol exactly equal the corresponding OhlcvRecord fields, and
// in_position is false.

use proptest::prelude::*;
use proptest::collection::vec;

use chrono::NaiveDate;
use flux_cli::commands::run::ohlcv_to_bars;
use flux_cli::data::OhlcvRecord;

/// Generate a valid OhlcvRecord with finite positive f64 values and non-empty symbol.
fn arb_ohlcv_record() -> impl Strategy<Value = OhlcvRecord> {
    (
        (2000i32..2030, 1u32..=12, 1u32..=28),
        "[A-Z]{1,5}",
        0.01f64..10000.0,
        0.01f64..10000.0,
        0.01f64..10000.0,
        0.01f64..10000.0,
        0.01f64..10000.0,
    )
        .prop_map(|((y, m, d), sym, open, high, low, close, volume)| {
            OhlcvRecord {
                timestamp: NaiveDate::from_ymd_opt(y, m, d)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                symbol: sym,
                open,
                high,
                low,
                close,
                volume,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 6: Each BarContext produced by ohlcv_to_bars has fields matching
    /// the source OhlcvRecord, and in_position is always false.
    #[test]
    fn ohlcv_to_bars_preserves_fields(records in vec(arb_ohlcv_record(), 1..10)) {
        let (bars, timestamps) = ohlcv_to_bars(&records);

        // Output lengths must equal input length
        prop_assert_eq!(bars.len(), records.len());
        prop_assert_eq!(timestamps.len(), records.len());

        for (record, bar) in records.iter().zip(bars.iter()) {
            prop_assert_eq!(bar.close, record.close, "close mismatch");
            prop_assert_eq!(bar.open, record.open, "open mismatch");
            prop_assert_eq!(bar.high, record.high, "high mismatch");
            prop_assert_eq!(bar.low, record.low, "low mismatch");
            prop_assert_eq!(bar.volume, record.volume, "volume mismatch");
            prop_assert_eq!(&bar.symbol, &record.symbol, "symbol mismatch");
            prop_assert_eq!(bar.in_position, false, "in_position must be false");
        }
    }
}
