//! Multi-strategy loader with single-file and TOML config modes.
//!
//! Handles loading and compiling strategies from either a single `.flux`
//! file or a `.toml` configuration that references multiple strategies,
//! connectors, and risk constraints. Also provides `build_connectors()`
//! to instantiate the appropriate connector types from config entries
//! or single-file connector blocks.

use std::path::{Path, PathBuf};
use std::time::Duration;

use flux_runtime::BarContext;
use serde::Deserialize;

use crate::interpreter::Interpreter;

use super::connector::{Connector, ConnectorError};
use super::poll_connector::PollingConnector;
use super::replay_connector::ReplayConnector;
use super::ws_connector::WebSocketConnector;

/// A loaded and compiled strategy with its interpreter and metadata.
pub struct StrategyModule {
    /// Human-readable name (from strategy declaration)
    pub name: String,
    /// File path this strategy was loaded from
    pub source_path: PathBuf,
    /// Independent interpreter instance
    pub interpreter: Interpreter,
    /// Symbols this strategy is subscribed to
    pub subscribed_symbols: Vec<String>,
}

impl std::fmt::Debug for StrategyModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StrategyModule")
            .field("name", &self.name)
            .field("source_path", &self.source_path)
            .field("subscribed_symbols", &self.subscribed_symbols)
            .finish_non_exhaustive()
    }
}

/// TOML configuration for multi-strategy mode.
#[derive(Debug, Deserialize)]
pub struct LiveConfig {
    pub capital: Option<f64>,
    pub state_file: Option<String>,
    pub risk: Option<RiskConfig>,
    pub strategies: Vec<StrategyEntry>,
    pub connectors: Vec<ConnectorConfig>,
}

/// A single strategy entry in the TOML config.
#[derive(Debug, Deserialize)]
pub struct StrategyEntry {
    pub path: String,
    pub symbols: Vec<String>,
}

/// Risk constraint configuration from TOML.
#[derive(Debug, Deserialize)]
pub struct RiskConfig {
    pub max_position_size: Option<f64>,
    pub max_exposure: Option<f64>,
    pub max_positions: Option<usize>,
}

/// Connector configuration from TOML.
#[derive(Debug, Deserialize)]
pub struct ConnectorConfig {
    /// Connector type: "websocket", "poll", "replay"
    pub kind: String,
    pub url: Option<String>,
    pub file: Option<String>,
    pub symbols: Vec<String>,
    pub interval: Option<String>,
    pub playback_rate: Option<f64>,
}

/// Error that occurs when loading/compiling a strategy.
#[derive(Debug, Clone)]
pub struct LoadError {
    /// Path to the strategy file that failed
    pub path: PathBuf,
    /// Human-readable error message
    pub message: String,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// Load strategies from a TOML config file or single .flux file.
///
/// Single-strategy mode: `flux live strategy.flux`
/// Multi-strategy mode: `flux live config.toml`
///
/// # Errors
///
/// Returns a list of `LoadError`s when all strategies fail to compile.
/// Individual compile failures are logged to stderr but do not prevent
/// other strategies from loading (requirement 2.5).
pub fn load_strategies(path: &Path) -> Result<Vec<StrategyModule>, Vec<LoadError>> {
    if path.extension().map_or(false, |e| e == "toml") {
        load_multi_strategy_config(path)
    } else {
        load_single_strategy(path)
            .map(|s| vec![s])
            .map_err(|e| vec![e])
    }
}

/// Load a single `.flux` strategy file.
///
/// Reads the file, compiles it through lex → parse → typecheck, creates
/// an Interpreter, and extracts symbols from the connector_block or
/// data_block (if present).
fn load_single_strategy(path: &Path) -> Result<StrategyModule, LoadError> {
    let source = std::fs::read_to_string(path).map_err(|e| LoadError {
        path: path.to_path_buf(),
        message: format!("failed to read file: {}", e),
    })?;

    compile_strategy(&source, path, None)
}

/// Load multiple strategies from a TOML configuration file.
///
/// Parses the TOML config, resolves strategy paths relative to the config
/// file's directory, and compiles each referenced strategy. On per-strategy
/// compile failure, logs the error and continues loading others (requirement 2.5).
/// If ALL strategies fail to compile, returns an error listing all failures
/// (requirement 2.6).
fn load_multi_strategy_config(config_path: &Path) -> Result<Vec<StrategyModule>, Vec<LoadError>> {
    let config_content = std::fs::read_to_string(config_path).map_err(|e| {
        vec![LoadError {
            path: config_path.to_path_buf(),
            message: format!("failed to read config file: {}", e),
        }]
    })?;

    let config: LiveConfig = toml::from_str(&config_content).map_err(|e| {
        vec![LoadError {
            path: config_path.to_path_buf(),
            message: format!("failed to parse TOML config: {}", e),
        }]
    })?;

    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));

    let mut modules = Vec::new();
    let mut errors = Vec::new();

    for entry in &config.strategies {
        let strategy_path = config_dir.join(&entry.path);

        let source = match std::fs::read_to_string(&strategy_path) {
            Ok(s) => s,
            Err(e) => {
                let err = LoadError {
                    path: strategy_path.clone(),
                    message: format!("failed to read file: {}", e),
                };
                eprintln!("[loader] error: {}", err);
                errors.push(err);
                continue;
            }
        };

        match compile_strategy(&source, &strategy_path, Some(&entry.symbols)) {
            Ok(module) => modules.push(module),
            Err(err) => {
                eprintln!("[loader] error: {}", err);
                errors.push(err);
            }
        }
    }

    // Requirement 2.6: if ALL strategies failed, return error listing all failures
    if modules.is_empty() {
        return Err(errors);
    }

    Ok(modules)
}

/// Compile a strategy source through lex → parse → typecheck and create
/// a StrategyModule.
///
/// If `override_symbols` is provided (from TOML config), those symbols are used.
/// Otherwise, symbols are extracted from the connector_block or data_block.
fn compile_strategy(
    source: &str,
    path: &Path,
    override_symbols: Option<&Vec<String>>,
) -> Result<StrategyModule, LoadError> {
    // Lex
    let tokens = flux_compiler::lexer::lex_with_spans(source).map_err(|e| LoadError {
        path: path.to_path_buf(),
        message: format!("lexer error: {}", e),
    })?;

    // Parse
    let ast = flux_compiler::parser::parse(tokens).map_err(|e| LoadError {
        path: path.to_path_buf(),
        message: format!("parse error: {}", e),
    })?;

    // Typecheck
    let typed_program = flux_compiler::typeck::check(ast).map_err(|e| LoadError {
        path: path.to_path_buf(),
        message: format!("type error: {}", e),
    })?;

    // Extract strategy name
    let name = typed_program.strategy.name.clone();

    // Determine subscribed symbols
    let subscribed_symbols = if let Some(symbols) = override_symbols {
        symbols.clone()
    } else {
        extract_symbols_from_program(&typed_program)
    };

    // Create interpreter
    let interpreter = Interpreter::new(&typed_program);

    Ok(StrategyModule {
        name,
        source_path: path.to_path_buf(),
        interpreter,
        subscribed_symbols,
    })
}

/// Extract symbols from a typed program's connector_block or data_block.
///
/// Prefers connector_block symbols (for live mode). Falls back to data_block
/// symbols. Returns an empty vec if neither block declares symbols.
fn extract_symbols_from_program(
    program: &flux_compiler::typeck::typed_ast::TypedProgram,
) -> Vec<String> {
    // Prefer connector_block for live mode
    if let Some(ref cb) = program.connector_block {
        if let Some(ref symbols) = cb.symbols {
            return symbols.clone();
        }
    }

    // Fall back to data_block symbols
    if let Some(ref db) = program.data_block {
        if let Some(ref symbols) = db.symbols {
            return symbols.clone();
        }
    }

    Vec::new()
}

/// Error that occurs when building a connector from configuration.
#[derive(Debug, Clone)]
pub struct ConnectorBuildError {
    /// Connector kind that failed
    pub kind: String,
    /// Human-readable error message
    pub message: String,
}

impl std::fmt::Display for ConnectorBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "connector '{}': {}", self.kind, self.message)
    }
}

/// Build connector instances from a list of `ConnectorConfig` entries (TOML mode).
///
/// Maps each config entry to the appropriate connector type:
/// - `"replay"` → `ReplayConnector`
/// - `"poll"` → `PollingConnector`
/// - `"websocket"` → `WebSocketConnector`
///
/// Validates required fields per connector type and returns errors for
/// missing or invalid configuration.
pub fn build_connectors(
    configs: &[ConnectorConfig],
) -> Result<Vec<Box<dyn Connector>>, Vec<ConnectorBuildError>> {
    let mut connectors: Vec<Box<dyn Connector>> = Vec::new();
    let mut errors: Vec<ConnectorBuildError> = Vec::new();

    for (i, config) in configs.iter().enumerate() {
        let id = format!("{}-{}", config.kind, i);

        match config.kind.as_str() {
            "replay" => match build_replay_connector(&id, config) {
                Ok(c) => connectors.push(Box::new(c)),
                Err(e) => errors.push(e),
            },
            "poll" => match build_poll_connector(&id, config) {
                Ok(c) => connectors.push(Box::new(c)),
                Err(e) => errors.push(e),
            },
            "websocket" => match build_ws_connector(&id, config) {
                Ok(c) => connectors.push(Box::new(c)),
                Err(e) => errors.push(e),
            },
            other => {
                errors.push(ConnectorBuildError {
                    kind: other.to_string(),
                    message: format!(
                        "unknown connector kind '{}'; expected 'replay', 'poll', or 'websocket'",
                        other
                    ),
                });
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(connectors)
}

/// Build connector instances from a `TypedConnectorBlock` (single-file mode).
///
/// Extracts connector type, url, file, and interval from the typed AST node
/// and delegates to the same per-type builder logic used by TOML mode.
pub fn build_connectors_from_block(
    connector_block: &flux_compiler::typeck::typed_ast::TypedConnectorBlock,
) -> Result<Vec<Box<dyn Connector>>, Vec<ConnectorBuildError>> {
    let kind = connector_block
        .connector_type
        .clone()
        .unwrap_or_else(|| "websocket".to_string());

    let config = ConnectorConfig {
        kind: kind.clone(),
        url: connector_block.url.clone(),
        file: connector_block.file.clone(),
        symbols: connector_block.symbols.clone().unwrap_or_default(),
        interval: connector_block.interval.clone(),
        playback_rate: None,
    };

    build_connectors(&[config])
}

/// Build a `ReplayConnector` from config, validating required fields.
fn build_replay_connector(
    id: &str,
    config: &ConnectorConfig,
) -> Result<ReplayConnector, ConnectorBuildError> {
    let file = config.file.as_ref().ok_or_else(|| ConnectorBuildError {
        kind: "replay".to_string(),
        message: "missing required field 'file' for replay connector".to_string(),
    })?;

    let playback_rate = config.playback_rate.unwrap_or(0.0);
    Ok(ReplayConnector::new(id, PathBuf::from(file), playback_rate))
}

/// Build a `PollingConnector` from config, validating required fields.
fn build_poll_connector(
    id: &str,
    config: &ConnectorConfig,
) -> Result<PollingConnector, ConnectorBuildError> {
    let url = config.url.as_ref().ok_or_else(|| ConnectorBuildError {
        kind: "poll".to_string(),
        message: "missing required field 'url' for poll connector".to_string(),
    })?;

    let interval = parse_interval(config.interval.as_deref()).map_err(|e| ConnectorBuildError {
        kind: "poll".to_string(),
        message: e,
    })?;

    Ok(PollingConnector::new(id, url.clone(), interval))
}

/// Build a `WebSocketConnector` from config, validating required fields.
fn build_ws_connector(
    id: &str,
    config: &ConnectorConfig,
) -> Result<WebSocketConnector, ConnectorBuildError> {
    let url = config.url.as_ref().ok_or_else(|| ConnectorBuildError {
        kind: "websocket".to_string(),
        message: "missing required field 'url' for websocket connector".to_string(),
    })?;

    Ok(WebSocketConnector::new(id, url.clone(), Box::new(default_json_parser())))
}

/// Parse an interval string like "1m", "5m", "1h", "30s", "1d" into a `Duration`.
///
/// Supported suffixes:
/// - `s` — seconds
/// - `m` — minutes
/// - `h` — hours
/// - `d` — days
///
/// If no interval is provided, defaults to 1 minute.
pub fn parse_interval(interval: Option<&str>) -> Result<Duration, String> {
    let s = match interval {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(Duration::from_secs(60)), // default: 1 minute
    };

    let (value_str, multiplier) = if let Some(stripped) = s.strip_suffix('d') {
        (stripped, 86_400u64)
    } else if let Some(stripped) = s.strip_suffix('h') {
        (stripped, 3_600u64)
    } else if let Some(stripped) = s.strip_suffix('m') {
        (stripped, 60u64)
    } else if let Some(stripped) = s.strip_suffix('s') {
        (stripped, 1u64)
    } else {
        return Err(format!(
            "invalid interval '{}': expected a suffix of 's', 'm', 'h', or 'd' (e.g., '5m', '1h')",
            s
        ));
    };

    let value: u64 = value_str.parse().map_err(|_| {
        format!(
            "invalid interval '{}': numeric part '{}' is not a valid integer",
            s, value_str
        )
    })?;

    if value == 0 {
        return Err(format!("invalid interval '{}': value must be greater than 0", s));
    }

    Ok(Duration::from_secs(value * multiplier))
}

/// Default JSON parser for the WebSocket connector.
///
/// Expects messages in the format:
/// ```json
/// {"symbol": "AAPL", "open": 150.0, "high": 152.0, "low": 149.0, "close": 151.0, "volume": 1000000.0}
/// ```
///
/// Returns a closure that parses each WebSocket text message into a `BarContext`.
fn default_json_parser() -> impl Fn(&str) -> Result<BarContext, ConnectorError> + Send + Sync {
    #[derive(serde::Deserialize)]
    struct WsBarMessage {
        symbol: String,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    }

    move |msg: &str| {
        let parsed: WsBarMessage =
            serde_json::from_str(msg).map_err(|e| ConnectorError::ParseError(e.to_string()))?;

        Ok(BarContext {
            symbol: parsed.symbol,
            open: parsed.open,
            high: parsed.high,
            low: parsed.low,
            close: parsed.close,
            volume: parsed.volume,
            in_position: false, // Set by the harness before dispatch
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Helper: write content to a temp file with a given extension and return its path.
    fn write_temp_file(content: &str, extension: &str) -> (NamedTempFile, PathBuf) {
        let mut file = tempfile::Builder::new()
            .suffix(extension)
            .tempfile()
            .unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        (file, path)
    }

    #[test]
    fn test_load_single_flux_file() {
        let source = r#"strategy Simple {
    on bar {
        if close > open {
            OPEN(symbol, 100.0)
        }
    }
}"#;
        let (_file, path) = write_temp_file(source, ".flux");
        let result = load_strategies(&path);
        assert!(result.is_ok());
        let modules = result.unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "Simple");
    }

    #[test]
    fn test_load_single_flux_file_with_connector_block() {
        let source = r#"
connector {
    type = "websocket"
    url = "wss://stream.example.com"
    symbols = ["AAPL", "MSFT"]
}

strategy WithConnector {
    on bar {
        if close > open {
            OPEN(symbol, 100.0)
        }
    }
}"#;
        let (_file, path) = write_temp_file(source, ".flux");
        let result = load_strategies(&path);
        assert!(result.is_ok());
        let modules = result.unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "WithConnector");
        assert_eq!(modules[0].subscribed_symbols, vec!["AAPL", "MSFT"]);
    }

    #[test]
    fn test_load_single_flux_file_compile_error() {
        let source = "strategy {"; // syntax error
        let (_file, path) = write_temp_file(source, ".flux");
        let result = load_strategies(&path);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("parse error"));
    }

    #[test]
    fn test_load_toml_config() {
        // Create strategy files
        let strategy1 = r#"strategy Alpha {
    on bar {
        if close > open {
            OPEN(symbol, 50.0)
        }
    }
}"#;
        let strategy2 = r#"strategy Beta {
    on bar {
        if close < open {
            OPEN(symbol, 25.0)
        }
    }
}"#;
        let dir = tempfile::tempdir().unwrap();
        let s1_path = dir.path().join("alpha.flux");
        let s2_path = dir.path().join("beta.flux");
        std::fs::write(&s1_path, strategy1).unwrap();
        std::fs::write(&s2_path, strategy2).unwrap();

        let config = format!(
            r#"
[[strategies]]
path = "alpha.flux"
symbols = ["AAPL"]

[[strategies]]
path = "beta.flux"
symbols = ["MSFT", "GOOG"]

[[connectors]]
kind = "replay"
file = "data.csv"
symbols = ["AAPL", "MSFT", "GOOG"]
"#
        );
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, config).unwrap();

        let result = load_strategies(&config_path);
        assert!(result.is_ok());
        let modules = result.unwrap();
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].name, "Alpha");
        assert_eq!(modules[0].subscribed_symbols, vec!["AAPL"]);
        assert_eq!(modules[1].name, "Beta");
        assert_eq!(modules[1].subscribed_symbols, vec!["MSFT", "GOOG"]);
    }

    #[test]
    fn test_load_toml_partial_failure_continues() {
        // One strategy is valid, one has a syntax error
        let valid_strategy = r#"strategy Good {
    on bar {
        OPEN(symbol, 100.0)
    }
}"#;
        let dir = tempfile::tempdir().unwrap();
        let good_path = dir.path().join("good.flux");
        std::fs::write(&good_path, valid_strategy).unwrap();
        // bad.flux doesn't exist — will fail to read

        let config = r#"
[[strategies]]
path = "good.flux"
symbols = ["AAPL"]

[[strategies]]
path = "bad.flux"
symbols = ["MSFT"]

[[connectors]]
kind = "replay"
file = "data.csv"
symbols = ["AAPL", "MSFT"]
"#;
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, config).unwrap();

        let result = load_strategies(&config_path);
        // Should succeed with the valid strategy
        assert!(result.is_ok());
        let modules = result.unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "Good");
    }

    #[test]
    fn test_load_toml_all_fail_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        // No strategy files exist

        let config = r#"
[[strategies]]
path = "missing1.flux"
symbols = ["AAPL"]

[[strategies]]
path = "missing2.flux"
symbols = ["MSFT"]

[[connectors]]
kind = "replay"
file = "data.csv"
symbols = ["AAPL", "MSFT"]
"#;
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, config).unwrap();

        let result = load_strategies(&config_path);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let path = Path::new("/tmp/nonexistent_strategy_xyz.flux");
        let result = load_strategies(path);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("failed to read file"));
    }

    #[test]
    fn test_load_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("bad.toml");
        std::fs::write(&config_path, "this is not valid toml [[[").unwrap();

        let result = load_strategies(&config_path);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("failed to parse TOML config"));
    }

    // --- Tests for build_connectors and helpers ---

    #[test]
    fn test_parse_interval_minutes() {
        assert_eq!(parse_interval(Some("1m")).unwrap(), Duration::from_secs(60));
        assert_eq!(parse_interval(Some("5m")).unwrap(), Duration::from_secs(300));
        assert_eq!(parse_interval(Some("30m")).unwrap(), Duration::from_secs(1800));
    }

    #[test]
    fn test_parse_interval_seconds() {
        assert_eq!(parse_interval(Some("30s")).unwrap(), Duration::from_secs(30));
        assert_eq!(parse_interval(Some("1s")).unwrap(), Duration::from_secs(1));
    }

    #[test]
    fn test_parse_interval_hours() {
        assert_eq!(parse_interval(Some("1h")).unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_interval(Some("2h")).unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn test_parse_interval_days() {
        assert_eq!(parse_interval(Some("1d")).unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn test_parse_interval_default() {
        // No interval provided → defaults to 1 minute
        assert_eq!(parse_interval(None).unwrap(), Duration::from_secs(60));
        assert_eq!(parse_interval(Some("")).unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn test_parse_interval_invalid_suffix() {
        let result = parse_interval(Some("5x"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected a suffix"));
    }

    #[test]
    fn test_parse_interval_invalid_number() {
        let result = parse_interval(Some("abcm"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a valid integer"));
    }

    #[test]
    fn test_parse_interval_zero_value() {
        let result = parse_interval(Some("0m"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be greater than 0"));
    }

    #[test]
    fn test_build_connectors_replay() {
        let configs = vec![ConnectorConfig {
            kind: "replay".to_string(),
            url: None,
            file: Some("data.csv".to_string()),
            symbols: vec!["AAPL".to_string()],
            interval: None,
            playback_rate: Some(1.0),
        }];

        let result = build_connectors(&configs);
        assert!(result.is_ok());
        let connectors = result.unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0].id(), "replay-0");
    }

    #[test]
    fn test_build_connectors_replay_missing_file() {
        let configs = vec![ConnectorConfig {
            kind: "replay".to_string(),
            url: None,
            file: None,
            symbols: vec!["AAPL".to_string()],
            interval: None,
            playback_rate: None,
        }];

        let result = build_connectors(&configs);
        assert!(result.is_err());
        let errors = result.err().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("missing required field 'file'"));
    }

    #[test]
    fn test_build_connectors_poll() {
        let configs = vec![ConnectorConfig {
            kind: "poll".to_string(),
            url: Some("https://api.example.com/bars".to_string()),
            file: None,
            symbols: vec!["AAPL".to_string()],
            interval: Some("5m".to_string()),
            playback_rate: None,
        }];

        let result = build_connectors(&configs);
        assert!(result.is_ok());
        let connectors = result.unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0].id(), "poll-0");
    }

    #[test]
    fn test_build_connectors_poll_missing_url() {
        let configs = vec![ConnectorConfig {
            kind: "poll".to_string(),
            url: None,
            file: None,
            symbols: vec!["AAPL".to_string()],
            interval: Some("5m".to_string()),
            playback_rate: None,
        }];

        let result = build_connectors(&configs);
        assert!(result.is_err());
        let errors = result.err().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("missing required field 'url'"));
    }

    #[test]
    fn test_build_connectors_websocket() {
        let configs = vec![ConnectorConfig {
            kind: "websocket".to_string(),
            url: Some("wss://stream.example.com".to_string()),
            file: None,
            symbols: vec!["AAPL".to_string(), "MSFT".to_string()],
            interval: None,
            playback_rate: None,
        }];

        let result = build_connectors(&configs);
        assert!(result.is_ok());
        let connectors = result.unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0].id(), "websocket-0");
    }

    #[test]
    fn test_build_connectors_websocket_missing_url() {
        let configs = vec![ConnectorConfig {
            kind: "websocket".to_string(),
            url: None,
            file: None,
            symbols: vec!["AAPL".to_string()],
            interval: None,
            playback_rate: None,
        }];

        let result = build_connectors(&configs);
        assert!(result.is_err());
        let errors = result.err().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("missing required field 'url'"));
    }

    #[test]
    fn test_build_connectors_unknown_kind() {
        let configs = vec![ConnectorConfig {
            kind: "mqtt".to_string(),
            url: Some("tcp://broker.example.com".to_string()),
            file: None,
            symbols: vec!["AAPL".to_string()],
            interval: None,
            playback_rate: None,
        }];

        let result = build_connectors(&configs);
        assert!(result.is_err());
        let errors = result.err().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("unknown connector kind 'mqtt'"));
    }

    #[test]
    fn test_build_connectors_multiple() {
        let configs = vec![
            ConnectorConfig {
                kind: "replay".to_string(),
                url: None,
                file: Some("data.csv".to_string()),
                symbols: vec!["AAPL".to_string()],
                interval: None,
                playback_rate: None,
            },
            ConnectorConfig {
                kind: "websocket".to_string(),
                url: Some("wss://stream.example.com".to_string()),
                file: None,
                symbols: vec!["MSFT".to_string()],
                interval: None,
                playback_rate: None,
            },
        ];

        let result = build_connectors(&configs);
        assert!(result.is_ok());
        let connectors = result.unwrap();
        assert_eq!(connectors.len(), 2);
        assert_eq!(connectors[0].id(), "replay-0");
        assert_eq!(connectors[1].id(), "websocket-1");
    }

    #[test]
    fn test_build_connectors_from_block() {
        let block = flux_compiler::typeck::typed_ast::TypedConnectorBlock {
            connector_type: Some("websocket".to_string()),
            url: Some("wss://stream.example.com".to_string()),
            symbols: Some(vec!["AAPL".to_string()]),
            interval: Some("1m".to_string()),
            file: None,
            span: flux_compiler::lexer::Span { start: 0, end: 0 },
        };

        let result = build_connectors_from_block(&block);
        assert!(result.is_ok());
        let connectors = result.unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0].id(), "websocket-0");
    }

    #[test]
    fn test_build_connectors_from_block_replay() {
        let block = flux_compiler::typeck::typed_ast::TypedConnectorBlock {
            connector_type: Some("replay".to_string()),
            url: None,
            symbols: Some(vec!["AAPL".to_string()]),
            interval: None,
            file: Some("data.csv".to_string()),
            span: flux_compiler::lexer::Span { start: 0, end: 0 },
        };

        let result = build_connectors_from_block(&block);
        assert!(result.is_ok());
        let connectors = result.unwrap();
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0].id(), "replay-0");
    }

    #[test]
    fn test_default_json_parser_valid() {
        let parser = default_json_parser();
        let msg = r#"{"symbol": "AAPL", "open": 150.0, "high": 152.0, "low": 149.0, "close": 151.0, "volume": 1000000.0}"#;
        let bar = parser(msg).unwrap();
        assert_eq!(bar.symbol, "AAPL");
        assert_eq!(bar.open, 150.0);
        assert_eq!(bar.high, 152.0);
        assert_eq!(bar.low, 149.0);
        assert_eq!(bar.close, 151.0);
        assert_eq!(bar.volume, 1_000_000.0);
        assert!(!bar.in_position);
    }

    #[test]
    fn test_default_json_parser_invalid() {
        let parser = default_json_parser();
        let msg = "not valid json";
        let result = parser(msg);
        assert!(result.is_err());
    }
}
