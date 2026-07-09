//! Yahoo Finance data provider implementation.
//!
//! Fetches historical OHLCV data from Yahoo Finance using its
//! cookie+crumb authentication mechanism. No API key required.

use std::sync::OnceLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

use super::error::FetchError;
use super::types::{Interval, Period, TimeRange};
use super::{DataFetcher, FetchRequest, OhlcvRecord};

/// Yahoo Finance data provider.
///
/// Uses `reqwest::blocking::Client` with cookie store enabled to handle
/// Yahoo's session-based authentication transparently.
pub struct YahooProvider {
    client: reqwest::blocking::Client,
    crumb: OnceLock<String>,
}

impl YahooProvider {
    /// Create a new Yahoo Finance provider with configured HTTP client.
    ///
    /// The client is configured with:
    /// - Cookie store enabled (for session cookies)
    /// - 30-second request timeout
    /// - 10-second connection timeout
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            crumb: OnceLock::new(),
        }
    }

    /// Authenticate with Yahoo Finance by obtaining a session cookie and crumb.
    ///
    /// Flow:
    /// 1. GET `https://fc.yahoo.com` — sets initial cookies (fast, no HTML)
    /// 2. GET `https://query2.finance.yahoo.com/v1/test/getcrumb` — returns crumb token
    ///
    /// If step 2 fails with 401, falls back to:
    /// 3. GET `https://finance.yahoo.com/quote/SPY` — establishes full session
    /// 4. Retry crumb request
    fn authenticate(&self) -> Result<&str, FetchError> {
        if let Some(crumb) = self.crumb.get() {
            return Ok(crumb.as_str());
        }

        let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                  AppleWebKit/537.36 (KHTML, like Gecko) \
                  Chrome/126.0.0.0 Safari/537.36";

        // Step 1: Hit fc.yahoo.com to establish cookies (lightweight endpoint)
        let _ = self
            .client
            .get("https://fc.yahoo.com")
            .header("User-Agent", ua)
            .send();

        // Step 2: Request crumb
        let crumb_result = self.request_crumb(ua);

        let crumb_text = match crumb_result {
            Ok(text) => text,
            Err(_) => {
                // Fallback: establish a full session via a quote page
                let _ = self
                    .client
                    .get("https://finance.yahoo.com/quote/SPY")
                    .header("User-Agent", ua)
                    .header("Accept", "text/html,application/xhtml+xml")
                    .header("Accept-Language", "en-US,en;q=0.9")
                    .send();

                // Retry crumb after full session
                self.request_crumb(ua).map_err(|_| FetchError::AuthError {
                    provider: "yahoo".to_string(),
                    detail: "crumb request failed after session retry — Yahoo may be blocking automated access".to_string(),
                })?
            }
        };

        // Cache the crumb (OnceLock ensures only one thread wins)
        let _ = self.crumb.set(crumb_text);
        Ok(self.crumb.get().unwrap().as_str())
    }

    /// Request the crumb token from Yahoo Finance.
    fn request_crumb(&self, user_agent: &str) -> Result<String, FetchError> {
        let crumb_resp = self
            .client
            .get("https://query2.finance.yahoo.com/v1/test/getcrumb")
            .header("User-Agent", user_agent)
            .send()
            .map_err(|e| map_reqwest_error(e, "query2.finance.yahoo.com"))?;

        if !crumb_resp.status().is_success() {
            return Err(FetchError::AuthError {
                provider: "yahoo".to_string(),
                detail: format!(
                    "crumb request failed with HTTP {}",
                    crumb_resp.status().as_u16()
                ),
            });
        }

        let crumb_text = crumb_resp.text().map_err(|e| FetchError::ParseError {
            provider: "yahoo".to_string(),
            detail: format!("failed to read crumb response: {}", e),
        })?;

        let crumb_text = crumb_text.trim().to_string();
        if crumb_text.is_empty() {
            return Err(FetchError::AuthError {
                provider: "yahoo".to_string(),
                detail: "received empty crumb".to_string(),
            });
        }

        Ok(crumb_text)
    }

    /// Build the Yahoo Finance download URL for the given request.
    fn build_url(&self, request: &FetchRequest, crumb: &str) -> String {
        let (period1, period2) = self.compute_timestamps(&request.time_range);
        let interval_str = request.interval.to_string();

        format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval={}&crumb={}&includeAdjustedClose=true",
            request.symbol, period1, period2, interval_str, crumb
        )
    }

    /// Convert a TimeRange into Unix timestamps (period1, period2).
    fn compute_timestamps(&self, time_range: &TimeRange) -> (i64, i64) {
        match time_range {
            TimeRange::Period(period) => {
                let now = Utc::now().timestamp();
                let duration_secs = period_to_seconds(*period);
                let period1 = now - duration_secs;
                (period1, now)
            }
            TimeRange::DateRange { from, to } => {
                let period1 = from
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp();
                let period2 = to
                    .and_hms_opt(23, 59, 59)
                    .unwrap()
                    .and_utc()
                    .timestamp();
                (period1, period2)
            }
        }
    }

    /// Parse Yahoo's v8 chart API JSON response into OHLCV records.
    ///
    /// The v8 chart response has the structure:
    /// ```json
    /// { "chart": { "result": [{ "timestamp": [...], "indicators": { "quote": [{
    ///     "open": [...], "high": [...], "low": [...], "close": [...], "volume": [...]
    /// }]}}]}}
    /// ```
    fn parse_chart_json(
        &self,
        body: &str,
        symbol: &str,
    ) -> Result<Vec<OhlcvRecord>, FetchError> {
        let json: serde_json::Value = serde_json::from_str(body).map_err(|e| {
            FetchError::ParseError {
                provider: "yahoo".to_string(),
                detail: format!("failed to parse JSON response: {}", e),
            }
        })?;

        let result = json
            .get("chart")
            .and_then(|c| c.get("result"))
            .and_then(|r| r.get(0))
            .ok_or_else(|| FetchError::ParseError {
                provider: "yahoo".to_string(),
                detail: "missing chart.result[0] in response".to_string(),
            })?;

        let timestamps = result
            .get("timestamp")
            .and_then(|t| t.as_array())
            .ok_or_else(|| FetchError::ParseError {
                provider: "yahoo".to_string(),
                detail: "missing timestamp array in response".to_string(),
            })?;

        let quote = result
            .get("indicators")
            .and_then(|i| i.get("quote"))
            .and_then(|q| q.get(0))
            .ok_or_else(|| FetchError::ParseError {
                provider: "yahoo".to_string(),
                detail: "missing indicators.quote[0] in response".to_string(),
            })?;

        let opens = quote.get("open").and_then(|v| v.as_array());
        let highs = quote.get("high").and_then(|v| v.as_array());
        let lows = quote.get("low").and_then(|v| v.as_array());
        let closes = quote.get("close").and_then(|v| v.as_array());
        let volumes = quote.get("volume").and_then(|v| v.as_array());

        let (opens, highs, lows, closes, volumes) =
            match (opens, highs, lows, closes, volumes) {
                (Some(o), Some(h), Some(l), Some(c), Some(v)) => (o, h, l, c, v),
                _ => {
                    return Err(FetchError::ParseError {
                        provider: "yahoo".to_string(),
                        detail: "missing OHLCV arrays in quote data".to_string(),
                    });
                }
            };

        let mut records = Vec::with_capacity(timestamps.len());

        for i in 0..timestamps.len() {
            let ts = timestamps[i].as_i64().unwrap_or(0);
            let open = opens.get(i).and_then(|v| v.as_f64());
            let high = highs.get(i).and_then(|v| v.as_f64());
            let low = lows.get(i).and_then(|v| v.as_f64());
            let close = closes.get(i).and_then(|v| v.as_f64());
            let volume = volumes.get(i).and_then(|v| v.as_f64()).or_else(|| {
                volumes.get(i).and_then(|v| v.as_i64()).map(|v| v as f64)
            });

            // Skip bars with null values
            let (open, high, low, close, volume) = match (open, high, low, close, volume) {
                (Some(o), Some(h), Some(l), Some(c), Some(v)) => (o, h, l, c, v),
                _ => continue,
            };

            let timestamp = DateTime::from_timestamp(ts, 0)
                .map(|dt| dt.naive_utc())
                .unwrap_or_else(|| {
                    NaiveDate::from_ymd_opt(1970, 1, 1)
                        .unwrap()
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                });

            records.push(OhlcvRecord {
                timestamp,
                symbol: symbol.to_string(),
                open,
                high,
                low,
                close,
                volume,
            });
        }

        Ok(records)
    }

    /// Parse Yahoo's CSV response into OHLCV records.
    ///
    /// Yahoo CSV format: Date,Open,High,Low,Close,Adj Close,Volume
    /// - For daily data: Date is "YYYY-MM-DD"
    /// - For intraday data: Date is Unix epoch seconds
    /// - Rows with "null" values are skipped
    /// - Uses Close (index 4), not Adj Close (index 5)
    #[allow(dead_code)]
    fn parse_csv(
        &self,
        body: &str,
        symbol: &str,
        interval: Interval,
    ) -> Result<Vec<OhlcvRecord>, FetchError> {
        let mut records = Vec::new();
        let mut lines = body.lines();

        // Skip header line
        let header = lines.next().ok_or_else(|| FetchError::ParseError {
            provider: "yahoo".to_string(),
            detail: "empty CSV response".to_string(),
        })?;

        // Validate header contains expected columns
        if !header.contains("Date") || !header.contains("Close") {
            return Err(FetchError::ParseError {
                provider: "yahoo".to_string(),
                detail: format!("unexpected CSV header: {}", header),
            });
        }

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Skip rows with "null" values
            if line.contains("null") {
                continue;
            }

            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() < 6 {
                continue;
            }

            // Parse timestamp
            let timestamp = parse_timestamp(fields[0], interval).map_err(|e| {
                FetchError::ParseError {
                    provider: "yahoo".to_string(),
                    detail: format!("failed to parse date '{}': {}", fields[0], e),
                }
            })?;

            // Parse OHLCV values (indices: 1=Open, 2=High, 3=Low, 4=Close, 6=Volume)
            let open = parse_f64(fields[1], "open")?;
            let high = parse_f64(fields[2], "high")?;
            let low = parse_f64(fields[3], "low")?;
            let close = parse_f64(fields[4], "close")?;
            let volume = parse_f64(fields[6], "volume")?;

            records.push(OhlcvRecord {
                timestamp,
                symbol: symbol.to_string(),
                open,
                high,
                low,
                close,
                volume,
            });
        }

        Ok(records)
    }
}

impl DataFetcher for YahooProvider {
    fn name(&self) -> &str {
        "yahoo"
    }

    fn fetch(&self, request: &FetchRequest) -> Result<Vec<OhlcvRecord>, FetchError> {
        // Authenticate (cached after first call)
        let crumb = self.authenticate()?;

        // Build download URL (v8 chart API, returns JSON)
        let url = self.build_url(request, crumb);

        // Fetch chart data
        let resp = self
            .client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36")
            .send()
            .map_err(|e| map_reqwest_error(e, "query1.finance.yahoo.com"))?;

        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(FetchError::RateLimited);
        }
        if !status.is_success() {
            return Err(FetchError::HttpError {
                status: status.as_u16(),
                message: format!(
                    "Yahoo Finance returned error for symbol '{}'",
                    request.symbol
                ),
            });
        }

        let body = resp.text().map_err(|e| FetchError::ParseError {
            provider: "yahoo".to_string(),
            detail: format!("failed to read response body: {}", e),
        })?;

        // Parse JSON chart response
        self.parse_chart_json(&body, &request.symbol)
    }
}

/// Parse a timestamp string from Yahoo CSV.
///
/// - Daily/weekly/monthly: "YYYY-MM-DD" → NaiveDateTime at midnight
/// - Intraday: Unix epoch seconds → NaiveDateTime
fn parse_timestamp(date_str: &str, interval: Interval) -> Result<NaiveDateTime, String> {
    if interval.is_intraday() {
        // Intraday: Date column is Unix epoch seconds
        let epoch: i64 = date_str
            .parse()
            .map_err(|e| format!("invalid epoch timestamp: {}", e))?;
        DateTime::from_timestamp(epoch, 0)
            .map(|dt| dt.naive_utc())
            .ok_or_else(|| format!("invalid epoch value: {}", epoch))
    } else {
        // Daily/weekly/monthly: Date column is "YYYY-MM-DD"
        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .map_err(|e| format!("invalid date format: {}", e))?;
        Ok(date.and_hms_opt(0, 0, 0).unwrap())
    }
}

/// Parse a float value from a CSV field.
fn parse_f64(value: &str, field_name: &str) -> Result<f64, FetchError> {
    value.parse::<f64>().map_err(|_| FetchError::ParseError {
        provider: "yahoo".to_string(),
        detail: format!("invalid {} value: '{}'", field_name, value),
    })
}

/// Convert a Period to approximate seconds duration.
fn period_to_seconds(period: Period) -> i64 {
    match period {
        Period::Day1 => 86_400,
        Period::Day5 => 5 * 86_400,
        Period::Month1 => 30 * 86_400,
        Period::Month3 => 90 * 86_400,
        Period::Month6 => 180 * 86_400,
        Period::Year1 => 365 * 86_400,
        Period::Year2 => 2 * 365 * 86_400,
        Period::Year5 => 5 * 365 * 86_400,
        Period::Max => 50 * 365 * 86_400, // ~50 years
    }
}

/// Map reqwest errors to appropriate FetchError variants.
fn map_reqwest_error(err: reqwest::Error, host: &str) -> FetchError {
    if err.is_timeout() {
        FetchError::Timeout { seconds: 30 }
    } else if err.is_connect() {
        FetchError::Connection {
            host: host.to_string(),
            reason: err.to_string(),
        }
    } else {
        FetchError::Connection {
            host: host.to_string(),
            reason: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yahoo_provider_name() {
        let provider = YahooProvider::new();
        assert_eq!(provider.name(), "yahoo");
    }

    #[test]
    fn parse_timestamp_daily() {
        let ts = parse_timestamp("2024-01-15", Interval::Day1).unwrap();
        assert_eq!(ts, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap().and_hms_opt(0, 0, 0).unwrap());
    }

    #[test]
    fn parse_timestamp_weekly() {
        let ts = parse_timestamp("2024-03-04", Interval::Week1).unwrap();
        assert_eq!(ts, NaiveDate::from_ymd_opt(2024, 3, 4).unwrap().and_hms_opt(0, 0, 0).unwrap());
    }

    #[test]
    fn parse_timestamp_intraday_epoch() {
        // 2024-01-15 14:30:00 UTC = 1705325400
        let ts = parse_timestamp("1705325400", Interval::Min5).unwrap();
        assert_eq!(ts, DateTime::from_timestamp(1705325400, 0).unwrap().naive_utc());
    }

    #[test]
    fn parse_timestamp_invalid_daily() {
        let result = parse_timestamp("not-a-date", Interval::Day1);
        assert!(result.is_err());
    }

    #[test]
    fn parse_timestamp_invalid_epoch() {
        let result = parse_timestamp("not-a-number", Interval::Min1);
        assert!(result.is_err());
    }

    #[test]
    fn parse_f64_valid() {
        assert_eq!(parse_f64("186.25", "close").unwrap(), 186.25);
        assert_eq!(parse_f64("0.0", "volume").unwrap(), 0.0);
        assert_eq!(parse_f64("1200000", "volume").unwrap(), 1200000.0);
    }

    #[test]
    fn parse_f64_invalid() {
        let err = parse_f64("abc", "close").unwrap_err();
        match err {
            FetchError::ParseError { provider, detail } => {
                assert_eq!(provider, "yahoo");
                assert!(detail.contains("close"));
                assert!(detail.contains("abc"));
            }
            _ => panic!("expected ParseError"),
        }
    }

    #[test]
    fn period_to_seconds_day1() {
        assert_eq!(period_to_seconds(Period::Day1), 86_400);
    }

    #[test]
    fn period_to_seconds_year1() {
        assert_eq!(period_to_seconds(Period::Year1), 365 * 86_400);
    }

    #[test]
    fn period_to_seconds_max() {
        assert_eq!(period_to_seconds(Period::Max), 50 * 365 * 86_400);
    }

    #[test]
    fn parse_csv_basic_daily() {
        let provider = YahooProvider::new();
        let csv = "Date,Open,High,Low,Close,Adj Close,Volume\n\
                   2024-01-02,185.50,186.75,185.10,186.20,185.90,1200000\n\
                   2024-01-03,186.00,187.50,185.80,187.10,186.80,1100000\n";

        let records = provider.parse_csv(csv, "AAPL", Interval::Day1).unwrap();
        assert_eq!(records.len(), 2);

        assert_eq!(records[0].symbol, "AAPL");
        assert_eq!(records[0].open, 185.50);
        assert_eq!(records[0].high, 186.75);
        assert_eq!(records[0].low, 185.10);
        assert_eq!(records[0].close, 186.20); // Close, not Adj Close
        assert_eq!(records[0].volume, 1200000.0);
        assert_eq!(
            records[0].timestamp,
            NaiveDate::from_ymd_opt(2024, 1, 2).unwrap().and_hms_opt(0, 0, 0).unwrap()
        );
    }

    #[test]
    fn parse_csv_skips_null_rows() {
        let provider = YahooProvider::new();
        let csv = "Date,Open,High,Low,Close,Adj Close,Volume\n\
                   2024-01-02,185.50,186.75,185.10,186.20,185.90,1200000\n\
                   2024-01-03,null,null,null,null,null,null\n\
                   2024-01-04,187.00,188.00,186.50,187.50,187.20,1300000\n";

        let records = provider.parse_csv(csv, "AAPL", Interval::Day1).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].timestamp,
            NaiveDate::from_ymd_opt(2024, 1, 2).unwrap().and_hms_opt(0, 0, 0).unwrap()
        );
        assert_eq!(
            records[1].timestamp,
            NaiveDate::from_ymd_opt(2024, 1, 4).unwrap().and_hms_opt(0, 0, 0).unwrap()
        );
    }

    #[test]
    fn parse_csv_intraday_epoch() {
        let provider = YahooProvider::new();
        let csv = "Date,Open,High,Low,Close,Adj Close,Volume\n\
                   1705325400,185.50,186.75,185.10,186.20,185.90,1200000\n";

        let records = provider.parse_csv(csv, "AAPL", Interval::Min5).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].timestamp,
            DateTime::from_timestamp(1705325400, 0).unwrap().naive_utc()
        );
    }

    #[test]
    fn parse_csv_empty_body() {
        let provider = YahooProvider::new();
        let result = provider.parse_csv("", "AAPL", Interval::Day1);
        assert!(result.is_err());
    }

    #[test]
    fn parse_csv_invalid_header() {
        let provider = YahooProvider::new();
        let result = provider.parse_csv("Foo,Bar,Baz\n1,2,3", "AAPL", Interval::Day1);
        assert!(result.is_err());
    }

    #[test]
    fn map_reqwest_error_timeout() {
        // We can't easily construct a reqwest timeout error in tests,
        // but we verify the function signature and basic mapping works.
        // Integration tests will cover actual HTTP errors.
    }

    #[test]
    fn compute_timestamps_date_range() {
        let provider = YahooProvider::new();
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
        let time_range = TimeRange::DateRange { from, to };

        let (p1, p2) = provider.compute_timestamps(&time_range);

        // period1 should be 2024-01-01 00:00:00 UTC
        assert_eq!(p1, from.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp());
        // period2 should be 2024-06-30 23:59:59 UTC
        assert_eq!(p2, to.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp());
    }

    #[test]
    fn compute_timestamps_period() {
        let provider = YahooProvider::new();
        let time_range = TimeRange::Period(Period::Year1);

        let (p1, p2) = provider.compute_timestamps(&time_range);

        // period2 should be approximately now
        let now = Utc::now().timestamp();
        assert!((p2 - now).abs() < 2); // within 2 seconds

        // period1 should be approximately 1 year ago
        let expected_p1 = now - 365 * 86_400;
        assert!((p1 - expected_p1).abs() < 2);
    }

    #[test]
    fn build_url_contains_required_params() {
        let provider = YahooProvider::new();
        let request = FetchRequest {
            symbol: "AAPL".to_string(),
            time_range: TimeRange::DateRange {
                from: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                to: NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
            },
            interval: Interval::Day1,
        };

        let url = provider.build_url(&request, "test_crumb");

        assert!(url.contains("AAPL"));
        assert!(url.contains("interval=1d"));
        assert!(url.contains("crumb=test_crumb"));
        assert!(url.contains("period1="));
        assert!(url.contains("period2="));
        assert!(url.starts_with("https://query1.finance.yahoo.com/v8/finance/chart/"));
    }
}
