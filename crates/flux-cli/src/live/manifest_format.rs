//! Pretty-printer for account manifest files.
//!
//! Formats an `AccountConfig` back into valid `account.flux` manifest syntax.
//! Uses `EnvSources` to emit `env("VAR")` for fields that were originally
//! sourced from environment variables.

use super::account_config::{AccountConfig, EnvSources};

/// Format an `AccountConfig` back into valid `account.flux` source.
///
/// Uses `EnvSources` to emit `env("VAR_NAME")` for fields that were originally
/// env-sourced. Uses 4-space indentation, one field per line.
pub fn format_manifest(config: &AccountConfig, env_sources: &EnvSources) -> String {
    let mut out = String::new();

    // account block
    format_account_block(&mut out, config, env_sources);
    out.push('\n');

    // gateway block
    format_gateway_block(&mut out, config, env_sources);
    out.push('\n');

    // data block
    format_data_block(&mut out, config, env_sources);
    out.push('\n');

    // database block
    format_database_block(&mut out, config, env_sources);
    out.push('\n');

    // risk block
    format_risk_block(&mut out, config, env_sources);
    out.push('\n');

    // products block
    format_products_block(&mut out, config, env_sources);
    out.push('\n');

    // strategies block
    format_strategies_block(&mut out, config, env_sources);

    out
}

/// Escape a string value for manifest output: escape `\` and `"`.
fn escape_string(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Format a float value, ensuring it always contains a decimal point.
fn format_float(f: f64) -> String {
    let s = format!("{}", f);
    if s.contains('.') {
        s
    } else {
        format!("{}.0", s)
    }
}

/// Format an integer value (no underscores).
fn format_int(i: i64) -> String {
    format!("{}", i)
}

/// Emit a string field value, checking if it's env-sourced.
fn emit_string_field(
    out: &mut String,
    block_name: &str,
    field_name: &str,
    value: &str,
    env_sources: &EnvSources,
) {
    if let Some(var_name) = env_sources.sources.get(&(block_name.to_string(), field_name.to_string())) {
        out.push_str(&format!("    {} = env(\"{}\")\n", field_name, var_name));
    } else {
        out.push_str(&format!("    {} = \"{}\"\n", field_name, escape_string(value)));
    }
}

fn format_account_block(out: &mut String, config: &AccountConfig, env_sources: &EnvSources) {
    out.push_str("account {\n");
    emit_string_field(out, "account", "name", &config.account.name, env_sources);
    emit_string_field(out, "account", "broker", &config.account.broker, env_sources);
    emit_string_field(out, "account", "account_id", &config.account.account_id, env_sources);
    emit_string_field(out, "account", "mode", &config.account.mode, env_sources);
    out.push_str("}\n");
}

fn format_gateway_block(out: &mut String, config: &AccountConfig, env_sources: &EnvSources) {
    out.push_str("gateway {\n");
    emit_string_field(out, "gateway", "host", &config.gateway.host, env_sources);
    out.push_str(&format!("    port = {}\n", format_int(config.gateway.port)));
    out.push_str("}\n");
}

fn format_data_block(out: &mut String, config: &AccountConfig, env_sources: &EnvSources) {
    out.push_str("data {\n");
    emit_string_field(out, "data", "source", &config.data.source, env_sources);
    // String list: ["a", "b", "c"] on one line
    let items: Vec<String> = config.data.symbols.iter()
        .map(|s| format!("\"{}\"", escape_string(s)))
        .collect();
    out.push_str(&format!("    symbols = [{}]\n", items.join(", ")));
    emit_string_field(out, "data", "interval", &config.data.interval, env_sources);
    out.push_str("}\n");
}

fn format_database_block(out: &mut String, config: &AccountConfig, env_sources: &EnvSources) {
    out.push_str("database {\n");
    emit_string_field(out, "database", "url", &config.database.url, env_sources);
    emit_string_field(out, "database", "schema", &config.database.schema, env_sources);
    out.push_str("}\n");
}

fn format_risk_block(out: &mut String, config: &AccountConfig, _env_sources: &EnvSources) {
    out.push_str("risk {\n");
    out.push_str(&format!("    max_daily_loss = {}\n", format_float(config.risk.max_daily_loss)));
    out.push_str(&format!("    max_weekly_loss = {}\n", format_float(config.risk.max_weekly_loss)));
    out.push_str(&format!("    max_position_per_product = {}\n", format_int(config.risk.max_position_per_product)));
    out.push_str(&format!("    max_total_notional = {}\n", format_float(config.risk.max_total_notional)));
    out.push_str(&format!("    max_drawdown_pct = {}\n", format_float(config.risk.max_drawdown_pct)));
    out.push_str(&format!("    correlation_warning_threshold = {}\n", format_int(config.risk.correlation_warning_threshold)));
    out.push_str(&format!("    initial_equity = {}\n", format_float(config.risk.initial_equity)));
    out.push_str("}\n");
}

fn format_products_block(out: &mut String, config: &AccountConfig, _env_sources: &EnvSources) {
    out.push_str("products {\n");
    for product in &config.products {
        out.push_str(&format!(
            "    {} = {{ multiplier = {}, tick_size = {}, margin = {} }}\n",
            product.name,
            format_float(product.multiplier),
            format_float(product.tick_size),
            format_float(product.margin),
        ));
    }
    out.push_str("}\n");
}

fn format_strategies_block(out: &mut String, config: &AccountConfig, _env_sources: &EnvSources) {
    out.push_str("strategies {\n");
    for strategy in &config.strategies {
        out.push_str(&format!(
            "    {} = {{ path = \"{}\", allocation = {}, priority = {} }}\n",
            strategy.name,
            escape_string(&strategy.path),
            format_float(strategy.allocation),
            format_int(strategy.priority),
        ));
    }
    out.push_str("}\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::account_config::*;

    fn sample_config() -> AccountConfig {
        AccountConfig {
            account: AccountSection {
                name: "swing".to_string(),
                broker: "ibkr".to_string(),
                account_id: "DU12345".to_string(),
                mode: "paper".to_string(),
            },
            gateway: GatewaySection {
                host: "127.0.0.1".to_string(),
                port: 4002,
            },
            data: DataSection {
                source: "ibkr".to_string(),
                symbols: vec!["ES".to_string(), "NQ".to_string(), "YM".to_string(), "RTY".to_string()],
                interval: "1d".to_string(),
            },
            database: DatabaseSection {
                url: "postgres://localhost/flux".to_string(),
                schema: "swing".to_string(),
            },
            risk: RiskSection {
                max_daily_loss: -15000.0,
                max_weekly_loss: -30000.0,
                max_position_per_product: 10,
                max_total_notional: 3000000.0,
                max_drawdown_pct: 0.08,
                correlation_warning_threshold: 4,
                initial_equity: 500000.0,
            },
            products: vec![
                ProductEntry {
                    name: "ES".to_string(),
                    multiplier: 50.0,
                    tick_size: 0.25,
                    margin: 15840.0,
                },
                ProductEntry {
                    name: "NQ".to_string(),
                    multiplier: 20.0,
                    tick_size: 0.25,
                    margin: 21120.0,
                },
            ],
            strategies: vec![
                StrategyEntry {
                    name: "aether".to_string(),
                    path: "aether/strategy/strategy.flux".to_string(),
                    allocation: 0.6,
                    priority: 1,
                },
                StrategyEntry {
                    name: "kairos".to_string(),
                    path: "kairos/strategy/strategy.flux".to_string(),
                    allocation: 0.4,
                    priority: 2,
                },
            ],
        }
    }

    #[test]
    fn test_format_basic_manifest() {
        let config = sample_config();
        let env_sources = EnvSources::default();
        let output = format_manifest(&config, &env_sources);

        // Check block structure
        assert!(output.contains("account {\n"));
        assert!(output.contains("gateway {\n"));
        assert!(output.contains("data {\n"));
        assert!(output.contains("database {\n"));
        assert!(output.contains("risk {\n"));
        assert!(output.contains("products {\n"));
        assert!(output.contains("strategies {\n"));

        // Check 4-space indentation
        assert!(output.contains("    name = \"swing\"\n"));
        assert!(output.contains("    port = 4002\n"));

        // Check float formatting
        assert!(output.contains("    max_daily_loss = -15000.0\n"));
        assert!(output.contains("    max_drawdown_pct = 0.08\n"));

        // Check string list
        assert!(output.contains("    symbols = [\"ES\", \"NQ\", \"YM\", \"RTY\"]\n"));

        // Check products inline struct
        assert!(output.contains("    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }\n"));
        assert!(output.contains("    NQ = { multiplier = 20.0, tick_size = 0.25, margin = 21120.0 }\n"));

        // Check strategies inline struct
        assert!(output.contains("    aether = { path = \"aether/strategy/strategy.flux\", allocation = 0.6, priority = 1 }\n"));
        assert!(output.contains("    kairos = { path = \"kairos/strategy/strategy.flux\", allocation = 0.4, priority = 2 }\n"));

        // Check trailing newline
        assert!(output.ends_with("}\n"));
    }

    #[test]
    fn test_format_with_env_sources() {
        let config = sample_config();
        let mut env_sources = EnvSources::default();
        env_sources.sources.insert(
            ("account".to_string(), "account_id".to_string()),
            "IBKR_SWING_ACCOUNT".to_string(),
        );
        env_sources.sources.insert(
            ("database".to_string(), "url".to_string()),
            "FLUX_DB_URL".to_string(),
        );

        let output = format_manifest(&config, &env_sources);

        // Env-sourced fields should emit env() instead of resolved value
        assert!(output.contains("    account_id = env(\"IBKR_SWING_ACCOUNT\")\n"));
        assert!(output.contains("    url = env(\"FLUX_DB_URL\")\n"));

        // Non-env fields still emit normal values
        assert!(output.contains("    name = \"swing\"\n"));
        assert!(output.contains("    schema = \"swing\"\n"));
    }

    #[test]
    fn test_format_string_escaping() {
        let mut config = sample_config();
        config.account.name = "test \"quoted\" name".to_string();
        config.database.url = "path\\to\\db".to_string();

        let env_sources = EnvSources::default();
        let output = format_manifest(&config, &env_sources);

        assert!(output.contains("    name = \"test \\\"quoted\\\" name\"\n"));
        assert!(output.contains("    url = \"path\\\\to\\\\db\"\n"));
    }

    #[test]
    fn test_format_blocks_separated_by_blank_lines() {
        let config = sample_config();
        let env_sources = EnvSources::default();
        let output = format_manifest(&config, &env_sources);

        // Blocks should be separated by blank lines
        assert!(output.contains("}\n\n"));
    }

    #[test]
    fn test_format_float_whole_numbers_have_decimal() {
        let mut config = sample_config();
        config.risk.max_total_notional = 3000000.0;
        config.risk.initial_equity = 500000.0;

        let env_sources = EnvSources::default();
        let output = format_manifest(&config, &env_sources);

        assert!(output.contains("    max_total_notional = 3000000.0\n"));
        assert!(output.contains("    initial_equity = 500000.0\n"));
    }

    #[test]
    fn test_round_trip_format_parse_extract() {
        // Build a config, format it, parse the output, extract config, compare
        let original_config = sample_config();
        let env_sources = EnvSources::default();
        let source = format_manifest(&original_config, &env_sources);

        // Parse the formatted output
        let tokens = flux_compiler::lexer::lex_with_spans(&source).expect("lex failed");
        let ast = flux_compiler::parser::parse_manifest(tokens).expect("parse failed");

        // Extract config from parsed AST
        let (extracted_config, _) =
            crate::live::account_config::extract_config(&ast).expect("extract failed");

        // Compare field-by-field
        assert_eq!(original_config, extracted_config);
    }
}
