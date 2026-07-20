//! Integration tests for MarketCalendar end-to-end loading.
//! Feature: market-calendar

use chrono::{NaiveDate, NaiveTime};
use std::path::Path;

use flux_cli::live::market_calendar::MarketCalendar;

// ---------------------------------------------------------------------------
// Test: load fixture → construct MarketCalendar → verify queries
// Requirements: 1.1
// ---------------------------------------------------------------------------

#[test]
fn load_fixture_and_verify_queries() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/market_calendar.toml");
    let cal = MarketCalendar::from_file(&fixture_path).unwrap();

    // Verify exchanges loaded
    let mut exchanges = cal.exchanges();
    exchanges.sort();
    assert_eq!(exchanges, vec!["CME", "NYSE"]);

    // Verify holiday detection
    let new_years = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    assert!(!cal.is_trading_day(new_years));

    // Verify normal trading day
    let monday = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    assert!(cal.is_trading_day(monday));

    // Verify half-day
    let half_day = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
    assert_eq!(
        cal.half_day_close(half_day),
        Some(NaiveTime::from_hms_opt(13, 0, 0).unwrap())
    );

    // Verify session times on half-day
    let (open, close) = cal.session_times_for_date("CME", half_day).unwrap();
    assert_eq!(open, NaiveTime::from_hms_opt(9, 30, 0).unwrap());
    assert_eq!(close, NaiveTime::from_hms_opt(13, 0, 0).unwrap());

    // Verify timezone
    let tz = cal.timezone("CME").unwrap();
    assert_eq!(tz, chrono_tz::US::Eastern);
}

#[test]
fn load_fixture_weekend_detection() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/market_calendar.toml");
    let cal = MarketCalendar::from_file(&fixture_path).unwrap();

    // 2026-01-03 is Saturday, 2026-01-04 is Sunday
    let sat = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
    let sun = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
    assert!(!cal.is_trading_day(sat));
    assert!(!cal.is_trading_day(sun));
}

#[test]
fn load_fixture_all_holidays_detected() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/market_calendar.toml");
    let cal = MarketCalendar::from_file(&fixture_path).unwrap();

    let holidays = vec![
        (2026, 1, 1),
        (2026, 1, 19),
        (2026, 2, 16),
        (2026, 4, 3),
        (2026, 5, 25),
        (2026, 7, 3),
        (2026, 9, 7),
        (2026, 11, 26),
        (2026, 12, 25),
    ];
    for (y, m, d) in holidays {
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert!(
            !cal.is_trading_day(date),
            "expected {} to be a holiday",
            date
        );
    }
}

// ---------------------------------------------------------------------------
// Test: harness idle behavior on holidays
// Requirements: 7.1
// ---------------------------------------------------------------------------

#[test]
fn harness_idle_on_holiday() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/market_calendar.toml");
    let cal = MarketCalendar::from_file(&fixture_path).unwrap();

    // Thanksgiving 2026 is a holiday — the harness should idle
    let thanksgiving = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
    assert!(
        !cal.is_trading_day(thanksgiving),
        "Thanksgiving should be a holiday"
    );

    // Christmas 2026 is a holiday
    let christmas = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
    assert!(
        !cal.is_trading_day(christmas),
        "Christmas should be a holiday"
    );

    // Day after a holiday (2026-01-02, Friday) should be a trading day
    let day_after_new_years = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    assert!(
        cal.is_trading_day(day_after_new_years),
        "Day after New Year's (Jan 2, a Friday) should be a trading day"
    );
}

// ---------------------------------------------------------------------------
// Test: harness flatten timing on half-days
// Requirements: 8.1
// ---------------------------------------------------------------------------

#[test]
fn harness_flatten_timing_on_half_days() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/market_calendar.toml");
    let cal = MarketCalendar::from_file(&fixture_path).unwrap();

    // 2026-11-27 (day after Thanksgiving) is a half-day with early close 13:00
    let half_day = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
    assert!(cal.is_trading_day(half_day), "Half-day should still be a trading day");

    let early_close = cal.half_day_close(half_day);
    assert_eq!(
        early_close,
        Some(NaiveTime::from_hms_opt(13, 0, 0).unwrap()),
        "Half-day should have early close at 13:00"
    );

    // session_times_for_date should return early close for both exchanges
    let (cme_open, cme_close) = cal.session_times_for_date("CME", half_day).unwrap();
    assert_eq!(cme_open, NaiveTime::from_hms_opt(9, 30, 0).unwrap());
    assert_eq!(cme_close, NaiveTime::from_hms_opt(13, 0, 0).unwrap());

    let (nyse_open, nyse_close) = cal.session_times_for_date("NYSE", half_day).unwrap();
    assert_eq!(nyse_open, NaiveTime::from_hms_opt(9, 30, 0).unwrap());
    assert_eq!(nyse_close, NaiveTime::from_hms_opt(13, 0, 0).unwrap());

    // 2026-12-24 (Christmas Eve) is also a half-day
    let xmas_eve = NaiveDate::from_ymd_opt(2026, 12, 24).unwrap();
    assert!(cal.is_trading_day(xmas_eve), "Christmas Eve should be a trading day");
    assert_eq!(
        cal.half_day_close(xmas_eve),
        Some(NaiveTime::from_hms_opt(13, 0, 0).unwrap()),
        "Christmas Eve should have early close at 13:00"
    );

    // Normal trading day should NOT have early close
    let normal_day = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    assert_eq!(
        cal.half_day_close(normal_day),
        None,
        "Normal trading day should not have early close"
    );

    // Normal day session times use full close
    let (_, normal_close) = cal.session_times_for_date("CME", normal_day).unwrap();
    assert_eq!(
        normal_close,
        NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
        "Normal day should close at 16:00"
    );
}
