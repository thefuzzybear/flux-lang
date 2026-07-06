use chrono::NaiveDate;
use std::fmt;
use std::str::FromStr;

/// Relative time period options for data fetching.
///
/// Specifies how far back from the current date to fetch data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day1,
    Day5,
    Month1,
    Month3,
    Month6,
    Year1,
    Year2,
    Year5,
    Max,
}

impl Period {
    /// Returns all valid string representations for use in error messages.
    pub fn all_values() -> &'static [&'static str] {
        &["1d", "5d", "1mo", "3mo", "6mo", "1y", "2y", "5y", "max"]
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Period::Day1 => "1d",
            Period::Day5 => "5d",
            Period::Month1 => "1mo",
            Period::Month3 => "3mo",
            Period::Month6 => "6mo",
            Period::Year1 => "1y",
            Period::Year2 => "2y",
            Period::Year5 => "5y",
            Period::Max => "max",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for Period {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1d" => Ok(Period::Day1),
            "5d" => Ok(Period::Day5),
            "1mo" => Ok(Period::Month1),
            "3mo" => Ok(Period::Month3),
            "6mo" => Ok(Period::Month6),
            "1y" => Ok(Period::Year1),
            "2y" => Ok(Period::Year2),
            "5y" => Ok(Period::Year5),
            "max" => Ok(Period::Max),
            _ => Err(format!(
                "invalid period '{}'. Valid options: {}",
                s,
                Period::all_values().join(", ")
            )),
        }
    }
}

/// Bar interval/granularity options for data fetching.
///
/// Specifies the time granularity of each data bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    Min1,
    Min5,
    Min15,
    Hour1,
    Day1,
    Week1,
    Month1,
}

impl Interval {
    /// Returns all valid string representations for use in error messages.
    pub fn all_values() -> &'static [&'static str] {
        &["1m", "5m", "15m", "1h", "1d", "1wk", "1mo"]
    }

    /// Returns true if this interval represents intraday granularity.
    ///
    /// Intraday intervals are: 1m, 5m, 15m, 1h.
    pub fn is_intraday(&self) -> bool {
        matches!(self, Interval::Min1 | Interval::Min5 | Interval::Min15 | Interval::Hour1)
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Interval::Min1 => "1m",
            Interval::Min5 => "5m",
            Interval::Min15 => "15m",
            Interval::Hour1 => "1h",
            Interval::Day1 => "1d",
            Interval::Week1 => "1wk",
            Interval::Month1 => "1mo",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for Interval {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1m" => Ok(Interval::Min1),
            "5m" => Ok(Interval::Min5),
            "15m" => Ok(Interval::Min15),
            "1h" => Ok(Interval::Hour1),
            "1d" => Ok(Interval::Day1),
            "1wk" => Ok(Interval::Week1),
            "1mo" => Ok(Interval::Month1),
            _ => Err(format!(
                "invalid interval '{}'. Valid options: {}",
                s,
                Interval::all_values().join(", ")
            )),
        }
    }
}

/// Time range specification — either relative (period) or absolute (date range).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeRange {
    /// Relative period from current date (e.g., "1y" = past year).
    Period(Period),
    /// Absolute date range with start and end dates.
    DateRange { from: NaiveDate, to: NaiveDate },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_display_all_variants() {
        assert_eq!(Period::Day1.to_string(), "1d");
        assert_eq!(Period::Day5.to_string(), "5d");
        assert_eq!(Period::Month1.to_string(), "1mo");
        assert_eq!(Period::Month3.to_string(), "3mo");
        assert_eq!(Period::Month6.to_string(), "6mo");
        assert_eq!(Period::Year1.to_string(), "1y");
        assert_eq!(Period::Year2.to_string(), "2y");
        assert_eq!(Period::Year5.to_string(), "5y");
        assert_eq!(Period::Max.to_string(), "max");
    }

    #[test]
    fn period_from_str_valid() {
        assert_eq!("1d".parse::<Period>().unwrap(), Period::Day1);
        assert_eq!("5d".parse::<Period>().unwrap(), Period::Day5);
        assert_eq!("1mo".parse::<Period>().unwrap(), Period::Month1);
        assert_eq!("3mo".parse::<Period>().unwrap(), Period::Month3);
        assert_eq!("6mo".parse::<Period>().unwrap(), Period::Month6);
        assert_eq!("1y".parse::<Period>().unwrap(), Period::Year1);
        assert_eq!("2y".parse::<Period>().unwrap(), Period::Year2);
        assert_eq!("5y".parse::<Period>().unwrap(), Period::Year5);
        assert_eq!("max".parse::<Period>().unwrap(), Period::Max);
    }

    #[test]
    fn period_from_str_invalid() {
        let err = "abc".parse::<Period>().unwrap_err();
        assert!(err.contains("invalid period 'abc'"));
        assert!(err.contains("1d"));
        assert!(err.contains("5d"));
        assert!(err.contains("1mo"));
        assert!(err.contains("max"));
    }

    #[test]
    fn period_roundtrip() {
        for &val in Period::all_values() {
            let parsed: Period = val.parse().unwrap();
            assert_eq!(parsed.to_string(), val);
        }
    }

    #[test]
    fn interval_display_all_variants() {
        assert_eq!(Interval::Min1.to_string(), "1m");
        assert_eq!(Interval::Min5.to_string(), "5m");
        assert_eq!(Interval::Min15.to_string(), "15m");
        assert_eq!(Interval::Hour1.to_string(), "1h");
        assert_eq!(Interval::Day1.to_string(), "1d");
        assert_eq!(Interval::Week1.to_string(), "1wk");
        assert_eq!(Interval::Month1.to_string(), "1mo");
    }

    #[test]
    fn interval_from_str_valid() {
        assert_eq!("1m".parse::<Interval>().unwrap(), Interval::Min1);
        assert_eq!("5m".parse::<Interval>().unwrap(), Interval::Min5);
        assert_eq!("15m".parse::<Interval>().unwrap(), Interval::Min15);
        assert_eq!("1h".parse::<Interval>().unwrap(), Interval::Hour1);
        assert_eq!("1d".parse::<Interval>().unwrap(), Interval::Day1);
        assert_eq!("1wk".parse::<Interval>().unwrap(), Interval::Week1);
        assert_eq!("1mo".parse::<Interval>().unwrap(), Interval::Month1);
    }

    #[test]
    fn interval_from_str_invalid() {
        let err = "2h".parse::<Interval>().unwrap_err();
        assert!(err.contains("invalid interval '2h'"));
        assert!(err.contains("1m"));
        assert!(err.contains("1h"));
        assert!(err.contains("1wk"));
        assert!(err.contains("1mo"));
    }

    #[test]
    fn interval_roundtrip() {
        for &val in Interval::all_values() {
            let parsed: Interval = val.parse().unwrap();
            assert_eq!(parsed.to_string(), val);
        }
    }

    #[test]
    fn interval_is_intraday() {
        assert!(Interval::Min1.is_intraday());
        assert!(Interval::Min5.is_intraday());
        assert!(Interval::Min15.is_intraday());
        assert!(Interval::Hour1.is_intraday());
        assert!(!Interval::Day1.is_intraday());
        assert!(!Interval::Week1.is_intraday());
        assert!(!Interval::Month1.is_intraday());
    }

    #[test]
    fn time_range_period_variant() {
        let tr = TimeRange::Period(Period::Year1);
        assert_eq!(tr, TimeRange::Period(Period::Year1));
    }

    #[test]
    fn time_range_date_range_variant() {
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
        let tr = TimeRange::DateRange { from, to };
        assert_eq!(tr, TimeRange::DateRange { from, to });
    }
}
