//! Market calendar module for trading session awareness.
//!
//! Loads a static TOML configuration at startup and provides O(1) lookups
//! for trading days, holidays, half-days, and session times per exchange.

use chrono::{Datelike, NaiveDate, NaiveTime, Weekday};
use chrono_tz::Tz;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Load and query trading calendar data.
///
/// Constructed once at startup from a TOML file, then queried
/// on every bar/session-reset to determine trading eligibility.
#[derive(Debug, Clone)]
pub struct MarketCalendar {
    /// Per-exchange session configurations.
    exchanges: HashMap<String, SessionConfig>,
    /// Set of holiday dates (O(1) lookup).
    holidays: HashSet<NaiveDate>,
    /// Half-day dates mapped to their early close time.
    half_days: HashMap<NaiveDate, NaiveTime>,
}

/// Per-exchange session parameters.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Exchange name (e.g., "CME", "NYSE").
    pub exchange: String,
    /// Session open time in the exchange's timezone.
    pub open: NaiveTime,
    /// Normal session close time in the exchange's timezone.
    pub close: NaiveTime,
    /// Timezone identifier (e.g., "US/Eastern").
    pub timezone: Tz,
}

/// Errors from calendar parsing and queries.
#[derive(Debug, thiserror::Error)]
pub enum CalendarError {
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("invalid date format '{value}': expected YYYY-MM-DD")]
    InvalidDate { value: String },

    #[error("invalid time format '{value}': expected HH:MM")]
    InvalidTime { value: String },

    #[error("missing required field '{field}' in session config")]
    MissingField { field: String },

    #[error("unknown exchange '{exchange}'")]
    UnknownExchange { exchange: String },

    #[error("date {date} is not a trading day")]
    NotTradingDay { date: NaiveDate },

    #[error("invalid timezone '{tz}': {reason}")]
    InvalidTimezone { tz: String, reason: String },

    #[error("validation error: session open {open} >= close {close}")]
    OpenAfterClose { open: NaiveTime, close: NaiveTime },

    #[error("validation error: half-day close {half_close} >= normal close {normal_close}")]
    HalfDayAfterClose {
        half_close: NaiveTime,
        normal_close: NaiveTime,
    },

    #[error("validation error: half-day close {half_close} <= session open {open}")]
    HalfDayBeforeOpen {
        half_close: NaiveTime,
        open: NaiveTime,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Non-fatal warnings from validation.
#[derive(Debug, Clone)]
pub enum CalendarWarning {
    /// A holiday was listed that already falls on a weekend.
    HolidayOnWeekend { date: NaiveDate },
}

// ---------------------------------------------------------------------------
// Intermediate serde structs for raw TOML deserialization
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct RawCalendarConfig {
    #[serde(default)]
    session: Vec<RawSessionEntry>,
    #[serde(flatten)]
    sections: HashMap<String, toml::Value>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct RawSessionEntry {
    exchange: String,
    open: String,
    close: String,
    timezone: String,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl MarketCalendar {
    /// Parse and validate a TOML string into a MarketCalendar.
    pub fn from_toml(toml_str: &str) -> Result<Self, CalendarError> {
        let raw: RawCalendarConfig = toml::from_str(toml_str)?;

        // Parse session entries
        let mut exchanges = HashMap::new();
        for entry in &raw.session {
            if entry.exchange.is_empty() {
                return Err(CalendarError::MissingField {
                    field: "exchange".to_string(),
                });
            }
            if entry.open.is_empty() {
                return Err(CalendarError::MissingField {
                    field: "open".to_string(),
                });
            }
            if entry.close.is_empty() {
                return Err(CalendarError::MissingField {
                    field: "close".to_string(),
                });
            }
            if entry.timezone.is_empty() {
                return Err(CalendarError::MissingField {
                    field: "timezone".to_string(),
                });
            }

            let open = parse_time(&entry.open)?;
            let close = parse_time(&entry.close)?;
            let tz: Tz = entry.timezone.parse().map_err(|_| {
                CalendarError::InvalidTimezone {
                    tz: entry.timezone.clone(),
                    reason: "unrecognized timezone identifier".to_string(),
                }
            })?;

            // Validate open < close
            if open >= close {
                return Err(CalendarError::OpenAfterClose { open, close });
            }

            exchanges.insert(
                entry.exchange.clone(),
                SessionConfig {
                    exchange: entry.exchange.clone(),
                    open,
                    close,
                    timezone: tz,
                },
            );
        }

        // Parse holiday and half-day sections from flattened map
        let mut holidays = HashSet::new();
        let mut half_days = HashMap::new();

        for (key, value) in &raw.sections {
            if key.starts_with("holidays") {
                // Extract dates array
                if let Some(table) = value.as_table() {
                    if let Some(dates_val) = table.get("dates") {
                        if let Some(dates_arr) = dates_val.as_array() {
                            for d in dates_arr {
                                if let Some(s) = d.as_str() {
                                    let date = parse_date(s)?;
                                    holidays.insert(date);
                                }
                            }
                        }
                    }
                }
            } else if key.starts_with("half_days") {
                // Extract dates array + early_close
                if let Some(table) = value.as_table() {
                    let early_close_str = table
                        .get("early_close")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| CalendarError::MissingField {
                            field: "early_close".to_string(),
                        })?;
                    let early_close = parse_time(early_close_str)?;

                    if let Some(dates_val) = table.get("dates") {
                        if let Some(dates_arr) = dates_val.as_array() {
                            for d in dates_arr {
                                if let Some(s) = d.as_str() {
                                    let date = parse_date(s)?;
                                    half_days.insert(date, early_close);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Validate half-day constraints against all exchanges
        for (_, early_close) in &half_days {
            for session in exchanges.values() {
                if *early_close >= session.close {
                    return Err(CalendarError::HalfDayAfterClose {
                        half_close: *early_close,
                        normal_close: session.close,
                    });
                }
                if *early_close <= session.open {
                    return Err(CalendarError::HalfDayBeforeOpen {
                        half_close: *early_close,
                        open: session.open,
                    });
                }
            }
        }

        Ok(MarketCalendar {
            exchanges,
            holidays,
            half_days,
        })
    }

    /// Load from a file path (convenience wrapper).
    pub fn from_file(path: &Path) -> Result<Self, CalendarError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml(&content)
    }

    /// Serialize back to a TOML string (pretty-printed).
    pub fn to_toml(&self) -> String {
        let mut output = String::new();

        // Serialize sessions
        for session in self.exchanges.values() {
            output.push_str("[[session]]\n");
            output.push_str(&format!("exchange = \"{}\"\n", session.exchange));
            output.push_str(&format!("open = \"{}\"\n", session.open.format("%H:%M")));
            output.push_str(&format!("close = \"{}\"\n", session.close.format("%H:%M")));
            output.push_str(&format!("timezone = \"{}\"\n", session.timezone));
            output.push('\n');
        }

        // Group holidays by year
        let mut holidays_by_year: HashMap<i32, Vec<NaiveDate>> = HashMap::new();
        for &date in &self.holidays {
            holidays_by_year.entry(date.year()).or_default().push(date);
        }
        for (year, mut dates) in holidays_by_year {
            dates.sort();
            output.push_str(&format!("[holidays_{}]\n", year));
            output.push_str("dates = [\n");
            for date in &dates {
                output.push_str(&format!("    \"{}\",\n", date.format("%Y-%m-%d")));
            }
            output.push_str("]\n\n");
        }

        // Group half-days by year
        let mut half_days_by_year: HashMap<i32, (NaiveTime, Vec<NaiveDate>)> = HashMap::new();
        for (&date, &time) in &self.half_days {
            let entry = half_days_by_year
                .entry(date.year())
                .or_insert_with(|| (time, Vec::new()));
            entry.1.push(date);
        }
        for (year, (early_close, mut dates)) in half_days_by_year {
            dates.sort();
            output.push_str(&format!("[half_days_{}]\n", year));
            output.push_str("dates = [\n");
            for date in &dates {
                output.push_str(&format!("    \"{}\",\n", date.format("%Y-%m-%d")));
            }
            output.push_str("]\n");
            output.push_str(&format!(
                "early_close = \"{}\"\n",
                early_close.format("%H:%M")
            ));
            output.push('\n');
        }

        output
    }

    /// Returns true if the given date is a trading day.
    /// False for weekends and holidays.
    pub fn is_trading_day(&self, date: NaiveDate) -> bool {
        let weekday = date.weekday();
        if weekday == Weekday::Sat || weekday == Weekday::Sun {
            return false;
        }
        !self.holidays.contains(&date)
    }

    /// Returns the early close time if the date is a half-day,
    /// or None if it's a normal trading day.
    pub fn half_day_close(&self, date: NaiveDate) -> Option<NaiveTime> {
        self.half_days.get(&date).copied()
    }

    /// Returns session open and close times for a given date and exchange.
    /// On half-days, the close time is the early close time.
    /// Returns Err if the exchange is unknown or the date is not a trading day.
    pub fn session_times_for_date(
        &self,
        exchange: &str,
        date: NaiveDate,
    ) -> Result<(NaiveTime, NaiveTime), CalendarError> {
        let session = self
            .exchanges
            .get(exchange)
            .ok_or_else(|| CalendarError::UnknownExchange {
                exchange: exchange.to_string(),
            })?;

        if !self.is_trading_day(date) {
            return Err(CalendarError::NotTradingDay { date });
        }

        let close = self.half_days.get(&date).copied().unwrap_or(session.close);
        Ok((session.open, close))
    }

    /// Returns the configured timezone for an exchange.
    pub fn timezone(&self, exchange: &str) -> Result<Tz, CalendarError> {
        let session = self
            .exchanges
            .get(exchange)
            .ok_or_else(|| CalendarError::UnknownExchange {
                exchange: exchange.to_string(),
            })?;
        Ok(session.timezone)
    }

    /// Returns all configured exchange names.
    pub fn exchanges(&self) -> Vec<&str> {
        self.exchanges.keys().map(|s| s.as_str()).collect()
    }

    /// Validate the calendar configuration, emitting warnings for
    /// redundant entries (e.g., holidays on weekends).
    pub fn validate(&self) -> Vec<CalendarWarning> {
        let mut warnings = Vec::new();
        for &date in &self.holidays {
            let weekday = date.weekday();
            if weekday == Weekday::Sat || weekday == Weekday::Sun {
                warnings.push(CalendarWarning::HolidayOnWeekend { date });
            }
        }
        warnings
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse a date string in YYYY-MM-DD format.
fn parse_date(s: &str) -> Result<NaiveDate, CalendarError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| CalendarError::InvalidDate {
        value: s.to_string(),
    })
}

/// Parse a time string in HH:MM format.
fn parse_time(s: &str) -> Result<NaiveTime, CalendarError> {
    NaiveTime::parse_from_str(s, "%H:%M").map_err(|_| CalendarError::InvalidTime {
        value: s.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"

[[session]]
exchange = "NYSE"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"

[holidays_2026]
dates = [
    "2026-01-01",
    "2026-01-19",
    "2026-02-16",
    "2026-04-03",
    "2026-05-25",
    "2026-07-03",
    "2026-09-07",
    "2026-11-26",
    "2026-12-25",
]

[half_days_2026]
dates = ["2026-11-27", "2026-12-24"]
early_close = "13:00"
"#;

    #[test]
    fn parse_valid_toml() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        assert_eq!(cal.exchanges.len(), 2);
        assert!(cal.exchanges.contains_key("CME"));
        assert!(cal.exchanges.contains_key("NYSE"));
        assert_eq!(cal.holidays.len(), 9);
        assert_eq!(cal.half_days.len(), 2);
    }

    #[test]
    fn holiday_detection() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        // New Year's 2026 is a Thursday
        let new_years = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert!(!cal.is_trading_day(new_years));
    }

    #[test]
    fn weekend_detection() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        // 2026-01-03 is a Saturday
        let saturday = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
        assert!(!cal.is_trading_day(saturday));
        // 2026-01-04 is a Sunday
        let sunday = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
        assert!(!cal.is_trading_day(sunday));
    }

    #[test]
    fn normal_trading_day() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        // 2026-01-05 is a Monday, not a holiday
        let monday = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        assert!(cal.is_trading_day(monday));
    }

    #[test]
    fn half_day_close_time() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let half_day = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
        let expected_close = NaiveTime::from_hms_opt(13, 0, 0).unwrap();
        assert_eq!(cal.half_day_close(half_day), Some(expected_close));
    }

    #[test]
    fn normal_day_no_half_close() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let normal = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        assert_eq!(cal.half_day_close(normal), None);
    }

    #[test]
    fn session_times_normal_day() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let monday = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let (open, close) = cal.session_times_for_date("CME", monday).unwrap();
        assert_eq!(open, NaiveTime::from_hms_opt(9, 30, 0).unwrap());
        assert_eq!(close, NaiveTime::from_hms_opt(16, 0, 0).unwrap());
    }

    #[test]
    fn session_times_half_day() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let half_day = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
        let (open, close) = cal.session_times_for_date("CME", half_day).unwrap();
        assert_eq!(open, NaiveTime::from_hms_opt(9, 30, 0).unwrap());
        assert_eq!(close, NaiveTime::from_hms_opt(13, 0, 0).unwrap());
    }

    #[test]
    fn session_times_not_trading_day() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let holiday = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let result = cal.session_times_for_date("CME", holiday);
        assert!(result.is_err());
    }

    #[test]
    fn unknown_exchange_error() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let monday = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let result = cal.session_times_for_date("UNKNOWN", monday);
        assert!(matches!(
            result,
            Err(CalendarError::UnknownExchange { .. })
        ));
    }

    #[test]
    fn timezone_lookup() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let tz = cal.timezone("CME").unwrap();
        assert_eq!(tz, chrono_tz::US::Eastern);
    }

    #[test]
    fn exchanges_list() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let mut names = cal.exchanges();
        names.sort();
        assert_eq!(names, vec!["CME", "NYSE"]);
    }

    #[test]
    fn invalid_date_format() {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"

[holidays_2026]
dates = ["not-a-date"]
"#;
        let result = MarketCalendar::from_toml(toml_str);
        assert!(matches!(result, Err(CalendarError::InvalidDate { .. })));
    }

    #[test]
    fn invalid_time_format() {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "9:30am"
close = "16:00"
timezone = "US/Eastern"
"#;
        let result = MarketCalendar::from_toml(toml_str);
        assert!(matches!(result, Err(CalendarError::InvalidTime { .. })));
    }

    #[test]
    fn open_after_close_error() {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "16:00"
close = "09:30"
timezone = "US/Eastern"
"#;
        let result = MarketCalendar::from_toml(toml_str);
        assert!(matches!(result, Err(CalendarError::OpenAfterClose { .. })));
    }

    #[test]
    fn invalid_timezone_error() {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "Fake/Timezone"
"#;
        let result = MarketCalendar::from_toml(toml_str);
        assert!(matches!(
            result,
            Err(CalendarError::InvalidTimezone { .. })
        ));
    }

    #[test]
    fn validate_weekend_holiday_warning() {
        // 2026-01-03 is a Saturday
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"

[holidays_2026]
dates = ["2026-01-03"]
"#;
        let cal = MarketCalendar::from_toml(toml_str).unwrap();
        let warnings = cal.validate();
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0],
            CalendarWarning::HolidayOnWeekend { .. }
        ));
    }

    #[test]
    fn toml_round_trip() {
        let cal = MarketCalendar::from_toml(SAMPLE_TOML).unwrap();
        let serialized = cal.to_toml();
        let cal2 = MarketCalendar::from_toml(&serialized).unwrap();

        assert_eq!(cal.holidays, cal2.holidays);
        assert_eq!(cal.half_days, cal2.half_days);
        assert_eq!(cal.exchanges.len(), cal2.exchanges.len());
        for (name, session) in &cal.exchanges {
            let session2 = cal2.exchanges.get(name).unwrap();
            assert_eq!(session.open, session2.open);
            assert_eq!(session.close, session2.close);
            assert_eq!(session.timezone, session2.timezone);
        }
    }

    #[test]
    fn missing_field_error() {
        // Session with empty exchange name triggers MissingField
        let toml_str = r#"
[[session]]
exchange = ""
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#;
        let result = MarketCalendar::from_toml(toml_str);
        assert!(matches!(result, Err(CalendarError::MissingField { .. })));
        if let Err(CalendarError::MissingField { field }) = result {
            assert_eq!(field, "exchange");
        }

        // Session with empty open triggers MissingField
        let toml_str2 = r#"
[[session]]
exchange = "CME"
open = ""
close = "16:00"
timezone = "US/Eastern"
"#;
        let result2 = MarketCalendar::from_toml(toml_str2);
        assert!(matches!(result2, Err(CalendarError::MissingField { .. })));
        if let Err(CalendarError::MissingField { field }) = result2 {
            assert_eq!(field, "open");
        }

        // Session with empty close triggers MissingField
        let toml_str3 = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = ""
timezone = "US/Eastern"
"#;
        let result3 = MarketCalendar::from_toml(toml_str3);
        assert!(matches!(result3, Err(CalendarError::MissingField { .. })));
        if let Err(CalendarError::MissingField { field }) = result3 {
            assert_eq!(field, "close");
        }

        // Session with empty timezone triggers MissingField
        let toml_str4 = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = ""
"#;
        let result4 = MarketCalendar::from_toml(toml_str4);
        assert!(matches!(result4, Err(CalendarError::MissingField { .. })));
        if let Err(CalendarError::MissingField { field }) = result4 {
            assert_eq!(field, "timezone");
        }
    }

    #[test]
    fn from_file_success() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test_calendar.toml");
        let mut file = std::fs::File::create(&file_path).unwrap();
        write!(file, "{}", SAMPLE_TOML).unwrap();

        let cal = MarketCalendar::from_file(&file_path).unwrap();
        assert_eq!(cal.exchanges.len(), 2);
        assert_eq!(cal.holidays.len(), 9);
    }

    #[test]
    fn from_file_not_found() {
        let result = MarketCalendar::from_file(Path::new("/nonexistent/path/calendar.toml"));
        assert!(matches!(result, Err(CalendarError::Io(_))));
    }

    #[test]
    fn half_day_after_close_error() {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "13:00"
timezone = "US/Eastern"

[half_days_2026]
dates = ["2026-11-27"]
early_close = "14:00"
"#;
        let result = MarketCalendar::from_toml(toml_str);
        assert!(matches!(
            result,
            Err(CalendarError::HalfDayAfterClose { .. })
        ));
    }

    #[test]
    fn half_day_before_open_error() {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"

[half_days_2026]
dates = ["2026-11-27"]
early_close = "09:00"
"#;
        let result = MarketCalendar::from_toml(toml_str);
        assert!(matches!(
            result,
            Err(CalendarError::HalfDayBeforeOpen { .. })
        ));
    }
}
