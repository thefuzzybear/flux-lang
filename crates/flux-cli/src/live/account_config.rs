//! Account manifest configuration extraction and validation.
//!
//! Extracts a typed `AccountConfig` from a parsed `ManifestProgram` AST,
//! validates all fields, and tracks which fields originated from `env()` calls.

use std::collections::HashMap;

use flux_compiler::lexer::Span;
use flux_compiler::parser::ast::{
    ManifestBlockKind, ManifestEntry, ManifestField, ManifestProgram, ManifestValue,
};

/// Top-level typed configuration extracted from an account.flux manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountConfig {
    pub account: AccountSection,
    pub gateway: GatewaySection,
    pub data: DataSection,
    pub database: DatabaseSection,
    pub risk: RiskSection,
    pub products: Vec<ProductEntry>,
    pub strategies: Vec<StrategyEntry>,
    /// Account-level default execution policy (None = Market).
    pub execution_default: Option<String>,
}

/// The `account` block: identity and mode.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountSection {
    pub name: String,
    pub broker: String,
    pub account_id: String,
    pub mode: String,
}

/// The `gateway` block: connection endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct GatewaySection {
    pub host: String,
    pub port: i64,
}

/// The `data` block: market data source configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct DataSection {
    pub source: String,
    pub symbols: Vec<String>,
    pub interval: String,
    /// Optional path to a CSV file for replay mode (relative to account directory).
    pub replay_file: Option<String>,
}

/// The `database` block: persistence layer.
#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseSection {
    pub url: String,
    pub schema: String,
}

/// The `risk` block: risk management parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskSection {
    pub max_daily_loss: f64,
    pub max_weekly_loss: f64,
    pub max_position_per_product: i64,
    pub max_total_notional: f64,
    pub max_drawdown_pct: f64,
    pub correlation_warning_threshold: i64,
    pub initial_equity: f64,
}

/// A single product entry from the `products` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductEntry {
    pub name: String,
    pub multiplier: f64,
    pub tick_size: f64,
    pub margin: f64,
}

/// A single strategy entry from the `strategies` block.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyEntry {
    pub name: String,
    pub path: String,
    pub allocation: f64,
    pub priority: i64,
    /// Execution policy override (None = use account default).
    pub execution: Option<String>,
    /// Offset ticks for AggressiveLimit policy.
    pub execution_offset_ticks: Option<i32>,
}

/// Tracks which fields in AccountConfig were resolved from `env()` calls.
///
/// Maps `(block_name, field_name)` → original env var name.
#[derive(Debug, Clone, Default)]
pub struct EnvSources {
    pub sources: HashMap<(String, String), String>,
}

/// Errors that can occur during config extraction from AST.
#[derive(Debug, Clone)]
pub enum ExtractionError {
    /// A required top-level block is missing from the manifest.
    MissingBlock { block_name: String },
    /// A required field is missing from a block.
    MissingField { block_name: String, field_name: String },
    /// A field value has the wrong type.
    TypeMismatch {
        block_name: String,
        field_name: String,
        expected: &'static str,
        actual: String,
    },
    /// An `env()` call references a variable that is not set.
    EnvNotSet {
        var_name: String,
        block_name: String,
        field_name: String,
        span: Span,
    },
    /// An `env()` call has an invalid variable name (non-alphanumeric/underscore chars).
    InvalidEnvName { var_name: String, span: Span },
}

/// Check if a strategy name contains only valid characters: `[a-zA-Z0-9_-]+`
fn is_valid_strategy_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Validate an extracted AccountConfig for semantic correctness.
///
/// Returns all validation errors as a collection rather than stopping at the first.
pub fn validate_config(config: &AccountConfig) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    // 1. Risk field validation
    if config.risk.max_daily_loss >= 0.0 {
        errors.push(ValidationError::InvalidRiskField {
            field: "max_daily_loss".to_string(),
            reason: format!("must be negative, got {}", config.risk.max_daily_loss),
        });
    }
    if config.risk.max_weekly_loss >= 0.0 {
        errors.push(ValidationError::InvalidRiskField {
            field: "max_weekly_loss".to_string(),
            reason: format!("must be negative, got {}", config.risk.max_weekly_loss),
        });
    }
    if config.risk.max_drawdown_pct <= 0.0 || config.risk.max_drawdown_pct >= 1.0 {
        errors.push(ValidationError::InvalidRiskField {
            field: "max_drawdown_pct".to_string(),
            reason: format!(
                "must be between 0.0 and 1.0 exclusive, got {}",
                config.risk.max_drawdown_pct
            ),
        });
    }
    if config.risk.max_position_per_product <= 0 {
        errors.push(ValidationError::InvalidRiskField {
            field: "max_position_per_product".to_string(),
            reason: format!("must be positive, got {}", config.risk.max_position_per_product),
        });
    }
    if config.risk.max_total_notional <= 0.0 {
        errors.push(ValidationError::InvalidRiskField {
            field: "max_total_notional".to_string(),
            reason: format!("must be positive, got {}", config.risk.max_total_notional),
        });
    }
    if config.risk.initial_equity <= 0.0 {
        errors.push(ValidationError::InvalidRiskField {
            field: "initial_equity".to_string(),
            reason: format!("must be positive, got {}", config.risk.initial_equity),
        });
    }

    // 2. Strategy validation
    let mut allocation_sum = 0.0_f64;
    for strategy in &config.strategies {
        // Validate allocation > 0.0
        if strategy.allocation <= 0.0 {
            errors.push(ValidationError::InvalidAllocation {
                strategy: strategy.name.clone(),
                reason: format!("allocation must be > 0.0, got {}", strategy.allocation),
            });
        }
        allocation_sum += strategy.allocation;

        // Validate path: non-empty and <= 512 chars
        if strategy.path.is_empty() {
            errors.push(ValidationError::InvalidStrategyPath {
                strategy: strategy.name.clone(),
                reason: "path must not be empty".to_string(),
            });
        } else if strategy.path.len() > 512 {
            errors.push(ValidationError::InvalidStrategyPath {
                strategy: strategy.name.clone(),
                reason: format!(
                    "path exceeds 512 characters (got {})",
                    strategy.path.len()
                ),
            });
        }

        // Validate name matches [a-zA-Z0-9_-]+
        if !is_valid_strategy_name(&strategy.name) {
            errors.push(ValidationError::InvalidStrategyName {
                strategy: strategy.name.clone(),
                reason: "name must be non-empty and contain only alphanumeric characters, hyphens, and underscores".to_string(),
            });
        }
    }

    // Check allocation sum after all individual checks
    if allocation_sum > 1.0 {
        errors.push(ValidationError::AllocationSumExceeded {
            total: allocation_sum,
        });
    }

    // 3. Product validation
    for product in &config.products {
        if product.multiplier <= 0.0 {
            errors.push(ValidationError::InvalidProductField {
                product: product.name.clone(),
                field: "multiplier".to_string(),
                reason: format!("must be > 0.0, got {}", product.multiplier),
            });
        }
        if product.tick_size <= 0.0 {
            errors.push(ValidationError::InvalidProductField {
                product: product.name.clone(),
                field: "tick_size".to_string(),
                reason: format!("must be > 0.0, got {}", product.tick_size),
            });
        }
        if product.margin <= 0.0 {
            errors.push(ValidationError::InvalidProductField {
                product: product.name.clone(),
                field: "margin".to_string(),
                reason: format!("must be > 0.0, got {}", product.margin),
            });
        }
    }

    // 4. Account mode validation
    if config.account.mode != "paper" && config.account.mode != "live" {
        errors.push(ValidationError::InvalidMode {
            value: config.account.mode.clone(),
        });
    }

    // 5. Gateway port validation
    if config.gateway.port < 1 || config.gateway.port > 65535 {
        errors.push(ValidationError::InvalidPort {
            value: config.gateway.port,
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Errors that can occur during config validation.
#[derive(Debug, Clone)]
pub enum ValidationError {
    /// A risk field has an invalid value (e.g. positive when it should be negative).
    InvalidRiskField { field: String, reason: String },
    /// A strategy allocation is invalid (e.g. <= 0).
    InvalidAllocation { strategy: String, reason: String },
    /// The sum of all strategy allocations exceeds 1.0.
    AllocationSumExceeded { total: f64 },
    /// A strategy path is invalid (empty, too long, etc.).
    InvalidStrategyPath { strategy: String, reason: String },
    /// A strategy name is invalid (contains disallowed characters).
    InvalidStrategyName { strategy: String, reason: String },
    /// A product field has an invalid value (e.g. <= 0).
    InvalidProductField {
        product: String,
        field: String,
        reason: String,
    },
    /// The account mode is not "paper" or "live".
    InvalidMode { value: String },
    /// The gateway port is outside the valid range (1–65535).
    InvalidPort { value: i64 },
}

// ─── Config Extraction ───────────────────────────────────────────────────────

/// Extract a typed AccountConfig from a parsed ManifestProgram AST.
///
/// Resolves env() calls by reading from the process environment.
/// Returns errors for missing blocks, missing fields, or type mismatches.
pub fn extract_config(
    ast: &ManifestProgram,
) -> Result<(AccountConfig, EnvSources), Vec<ExtractionError>> {
    let mut errors: Vec<ExtractionError> = Vec::new();
    let mut env_sources = EnvSources::default();

    // Collect each block kind into Option variables
    let mut account_block: Option<&Vec<ManifestField>> = None;
    let mut gateway_block: Option<&Vec<ManifestField>> = None;
    let mut data_block: Option<&Vec<ManifestField>> = None;
    let mut database_block: Option<&Vec<ManifestField>> = None;
    let mut risk_block: Option<&Vec<ManifestField>> = None;
    let mut products_block: Option<&Vec<ManifestEntry>> = None;
    let mut strategies_block: Option<&Vec<ManifestEntry>> = None;

    for block in &ast.blocks {
        match &block.kind {
            ManifestBlockKind::Account(fields) => account_block = Some(fields),
            ManifestBlockKind::Gateway(fields) => gateway_block = Some(fields),
            ManifestBlockKind::Data(fields) => data_block = Some(fields),
            ManifestBlockKind::Database(fields) => database_block = Some(fields),
            ManifestBlockKind::Risk(fields) => risk_block = Some(fields),
            ManifestBlockKind::Products(entries) => products_block = Some(entries),
            ManifestBlockKind::Strategies(entries) => strategies_block = Some(entries),
        }
    }

    // Check for missing required blocks
    if account_block.is_none() {
        errors.push(ExtractionError::MissingBlock {
            block_name: "account".to_string(),
        });
    }
    if gateway_block.is_none() {
        errors.push(ExtractionError::MissingBlock {
            block_name: "gateway".to_string(),
        });
    }
    if data_block.is_none() {
        errors.push(ExtractionError::MissingBlock {
            block_name: "data".to_string(),
        });
    }
    if database_block.is_none() {
        errors.push(ExtractionError::MissingBlock {
            block_name: "database".to_string(),
        });
    }
    if risk_block.is_none() {
        errors.push(ExtractionError::MissingBlock {
            block_name: "risk".to_string(),
        });
    }
    if products_block.is_none() {
        errors.push(ExtractionError::MissingBlock {
            block_name: "products".to_string(),
        });
    }
    if strategies_block.is_none() {
        errors.push(ExtractionError::MissingBlock {
            block_name: "strategies".to_string(),
        });
    }

    // Extract each block's fields (if block is present)
    let account = account_block.map(|fields| {
        extract_account_section(fields, &mut errors, &mut env_sources)
    });
    let gateway = gateway_block.map(|fields| {
        extract_gateway_section(fields, &mut errors, &mut env_sources)
    });
    let data = data_block.map(|fields| {
        extract_data_section(fields, &mut errors, &mut env_sources)
    });
    let database = database_block.map(|fields| {
        extract_database_section(fields, &mut errors, &mut env_sources)
    });
    let risk = risk_block.map(|fields| {
        extract_risk_section(fields, &mut errors, &mut env_sources)
    });
    let products = products_block.map(|entries| {
        extract_products(entries, &mut errors, &mut env_sources)
    });
    let strategies = strategies_block.map(|entries| {
        extract_strategies(entries, &mut errors, &mut env_sources)
    });

    if !errors.is_empty() {
        return Err(errors);
    }

    // All blocks are present and fields extracted successfully
    let config = AccountConfig {
        account: account.unwrap(),
        gateway: gateway.unwrap(),
        data: data.unwrap(),
        database: database.unwrap(),
        risk: risk.unwrap(),
        products: products.unwrap(),
        strategies: strategies.unwrap(),
        execution_default: None, // parsed when execution block support is added to manifest parser
    };

    Ok((config, env_sources))
}

// ─── Section Extractors ──────────────────────────────────────────────────────

fn extract_account_section(
    fields: &[ManifestField],
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> AccountSection {
    let name = extract_string(fields, "name", "account", errors, env_sources)
        .unwrap_or_default();
    let broker = extract_string(fields, "broker", "account", errors, env_sources)
        .unwrap_or_default();
    let account_id = extract_string(fields, "account_id", "account", errors, env_sources)
        .unwrap_or_default();
    let mode = extract_string(fields, "mode", "account", errors, env_sources)
        .unwrap_or_default();

    AccountSection {
        name,
        broker,
        account_id,
        mode,
    }
}

fn extract_gateway_section(
    fields: &[ManifestField],
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> GatewaySection {
    let host = extract_string(fields, "host", "gateway", errors, env_sources)
        .unwrap_or_default();
    let port = extract_int(fields, "port", "gateway", errors).unwrap_or_default();

    GatewaySection { host, port }
}

fn extract_data_section(
    fields: &[ManifestField],
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> DataSection {
    let source = extract_string(fields, "source", "data", errors, env_sources)
        .unwrap_or_default();
    let symbols = extract_string_list(fields, "symbols", "data", errors).unwrap_or_default();
    let interval = extract_string(fields, "interval", "data", errors, env_sources)
        .unwrap_or_default();
    // Optional: replay_file for replay mode (not an error if missing)
    let replay_file = extract_string_optional(fields, "replay_file", env_sources);

    DataSection {
        source,
        symbols,
        interval,
        replay_file,
    }
}

fn extract_database_section(
    fields: &[ManifestField],
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> DatabaseSection {
    let url = extract_string(fields, "url", "database", errors, env_sources)
        .unwrap_or_default();
    let schema = extract_string(fields, "schema", "database", errors, env_sources)
        .unwrap_or_default();

    DatabaseSection { url, schema }
}

fn extract_risk_section(
    fields: &[ManifestField],
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> RiskSection {
    let max_daily_loss =
        extract_float(fields, "max_daily_loss", "risk", errors).unwrap_or_default();
    let max_weekly_loss =
        extract_float(fields, "max_weekly_loss", "risk", errors).unwrap_or_default();
    let max_position_per_product =
        extract_int(fields, "max_position_per_product", "risk", errors).unwrap_or_default();
    let max_total_notional =
        extract_float(fields, "max_total_notional", "risk", errors).unwrap_or_default();
    let max_drawdown_pct =
        extract_float(fields, "max_drawdown_pct", "risk", errors).unwrap_or_default();
    let correlation_warning_threshold =
        extract_int(fields, "correlation_warning_threshold", "risk", errors).unwrap_or_default();
    let initial_equity =
        extract_float(fields, "initial_equity", "risk", errors).unwrap_or_default();
    let _ = env_sources; // env_sources passed for consistency but risk fields are numeric

    RiskSection {
        max_daily_loss,
        max_weekly_loss,
        max_position_per_product,
        max_total_notional,
        max_drawdown_pct,
        correlation_warning_threshold,
        initial_equity,
    }
}

fn extract_products(
    entries: &[ManifestEntry],
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> Vec<ProductEntry> {
    let mut products = Vec::new();
    for entry in entries {
        let name = entry.name.clone();
        let multiplier =
            extract_float(&entry.fields, "multiplier", "products", errors).unwrap_or_default();
        let tick_size =
            extract_float(&entry.fields, "tick_size", "products", errors).unwrap_or_default();
        let margin =
            extract_float(&entry.fields, "margin", "products", errors).unwrap_or_default();
        let _ = env_sources;
        products.push(ProductEntry {
            name,
            multiplier,
            tick_size,
            margin,
        });
    }
    products
}

fn extract_strategies(
    entries: &[ManifestEntry],
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> Vec<StrategyEntry> {
    let mut strategies = Vec::new();
    for entry in entries {
        let name = entry.name.clone();
        let path = extract_string(&entry.fields, "path", "strategies", errors, env_sources)
            .unwrap_or_default();
        let allocation =
            extract_float(&entry.fields, "allocation", "strategies", errors).unwrap_or_default();
        let priority =
            extract_int(&entry.fields, "priority", "strategies", errors).unwrap_or_default();
        let execution = extract_optional_string(&entry.fields, "execution");
        let execution_offset_ticks = extract_optional_int(&entry.fields, "execution_offset_ticks");
        strategies.push(StrategyEntry {
            name,
            path,
            allocation,
            priority,
            execution,
            execution_offset_ticks,
        });
    }
    strategies
}

// ─── Field Extraction Helpers ────────────────────────────────────────────────

/// Extract an optional string field from a list of ManifestFields.
/// Returns None if the field is not present (does NOT push an error).
fn extract_optional_string(fields: &[ManifestField], field_name: &str) -> Option<String> {
    fields.iter().find(|f| f.name == field_name).and_then(|f| match &f.value {
        ManifestValue::String(s) => Some(s.clone()),
        _ => None,
    })
}

/// Extract an optional integer field from a list of ManifestFields.
/// Returns None if the field is not present (does NOT push an error).
fn extract_optional_int(fields: &[ManifestField], field_name: &str) -> Option<i32> {
    fields.iter().find(|f| f.name == field_name).and_then(|f| match &f.value {
        ManifestValue::Int(i) => Some(*i as i32),
        _ => None,
    })
}

/// Extract a string field from a list of ManifestFields.
/// Resolves env() calls and records them in EnvSources.
fn extract_string(
    fields: &[ManifestField],
    field_name: &str,
    block_name: &str,
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> Option<String> {
    let field = fields.iter().find(|f| f.name == field_name);
    match field {
        None => {
            errors.push(ExtractionError::MissingField {
                block_name: block_name.to_string(),
                field_name: field_name.to_string(),
            });
            None
        }
        Some(f) => match &f.value {
            ManifestValue::String(s) => Some(s.clone()),
            ManifestValue::EnvCall(var) => {
                resolve_env(var, &f.span, block_name, field_name, errors, env_sources)
            }
            other => {
                errors.push(ExtractionError::TypeMismatch {
                    block_name: block_name.to_string(),
                    field_name: field_name.to_string(),
                    expected: "string",
                    actual: manifest_value_type_name(other),
                });
                None
            }
        },
    }
}

/// Extract an optional string field — returns None if the field is missing (no error).
/// Used for fields that are not required.
fn extract_string_optional(
    fields: &[ManifestField],
    field_name: &str,
    _env_sources: &mut EnvSources,
) -> Option<String> {
    let field = fields.iter().find(|f| f.name == field_name)?;
    match &field.value {
        ManifestValue::String(s) => Some(s.clone()),
        ManifestValue::EnvCall(var) => std::env::var(var).ok(),
        _ => None,
    }
}

/// Extract an integer field from a list of ManifestFields.
fn extract_int(
    fields: &[ManifestField],
    field_name: &str,
    block_name: &str,
    errors: &mut Vec<ExtractionError>,
) -> Option<i64> {
    let field = fields.iter().find(|f| f.name == field_name);
    match field {
        None => {
            errors.push(ExtractionError::MissingField {
                block_name: block_name.to_string(),
                field_name: field_name.to_string(),
            });
            None
        }
        Some(f) => match &f.value {
            ManifestValue::Int(i) => Some(*i),
            other => {
                errors.push(ExtractionError::TypeMismatch {
                    block_name: block_name.to_string(),
                    field_name: field_name.to_string(),
                    expected: "integer",
                    actual: manifest_value_type_name(other),
                });
                None
            }
        },
    }
}

/// Extract a float field from a list of ManifestFields.
/// Also accepts integer values, promoting them to f64.
fn extract_float(
    fields: &[ManifestField],
    field_name: &str,
    block_name: &str,
    errors: &mut Vec<ExtractionError>,
) -> Option<f64> {
    let field = fields.iter().find(|f| f.name == field_name);
    match field {
        None => {
            errors.push(ExtractionError::MissingField {
                block_name: block_name.to_string(),
                field_name: field_name.to_string(),
            });
            None
        }
        Some(f) => match &f.value {
            ManifestValue::Float(v) => Some(*v),
            ManifestValue::Int(i) => Some(*i as f64),
            other => {
                errors.push(ExtractionError::TypeMismatch {
                    block_name: block_name.to_string(),
                    field_name: field_name.to_string(),
                    expected: "float",
                    actual: manifest_value_type_name(other),
                });
                None
            }
        },
    }
}

/// Extract a string list field from a list of ManifestFields.
fn extract_string_list(
    fields: &[ManifestField],
    field_name: &str,
    block_name: &str,
    errors: &mut Vec<ExtractionError>,
) -> Option<Vec<String>> {
    let field = fields.iter().find(|f| f.name == field_name);
    match field {
        None => {
            errors.push(ExtractionError::MissingField {
                block_name: block_name.to_string(),
                field_name: field_name.to_string(),
            });
            None
        }
        Some(f) => match &f.value {
            ManifestValue::StringList(list) => Some(list.clone()),
            other => {
                errors.push(ExtractionError::TypeMismatch {
                    block_name: block_name.to_string(),
                    field_name: field_name.to_string(),
                    expected: "string list",
                    actual: manifest_value_type_name(other),
                });
                None
            }
        },
    }
}

// ─── Env Resolution ──────────────────────────────────────────────────────────

/// Resolve an env() call: validate the variable name, read from environment,
/// and record in EnvSources.
fn resolve_env(
    var: &str,
    span: &Span,
    block_name: &str,
    field_name: &str,
    errors: &mut Vec<ExtractionError>,
    env_sources: &mut EnvSources,
) -> Option<String> {
    // Validate env var name: only [A-Za-z0-9_] allowed
    if !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || var.is_empty() {
        errors.push(ExtractionError::InvalidEnvName {
            var_name: var.to_string(),
            span: *span,
        });
        return None;
    }

    match std::env::var(var) {
        Ok(value) => {
            env_sources.sources.insert(
                (block_name.to_string(), field_name.to_string()),
                var.to_string(),
            );
            Some(value)
        }
        Err(_) => {
            errors.push(ExtractionError::EnvNotSet {
                var_name: var.to_string(),
                block_name: block_name.to_string(),
                field_name: field_name.to_string(),
                span: *span,
            });
            None
        }
    }
}

// ─── Utilities ───────────────────────────────────────────────────────────────

/// Return a human-readable type name for a ManifestValue variant.
fn manifest_value_type_name(value: &ManifestValue) -> String {
    match value {
        ManifestValue::String(_) => "string".to_string(),
        ManifestValue::Int(_) => "integer".to_string(),
        ManifestValue::Float(_) => "float".to_string(),
        ManifestValue::StringList(_) => "string list".to_string(),
        ManifestValue::EnvCall(_) => "env()".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_compiler::lexer::lex_with_spans;
    use flux_compiler::parser::parse_manifest;

    /// Helper: parse a manifest source string and extract the config.
    fn parse_and_extract(source: &str) -> Result<(AccountConfig, EnvSources), Vec<ExtractionError>> {
        let tokens = lex_with_spans(source).expect("lex failed");
        let ast = parse_manifest(tokens).expect("parse failed");
        extract_config(&ast)
    }

    /// Full valid manifest source for test cases.
    fn full_valid_manifest() -> &'static str {
        r#"account {
    name = "swing"
    broker = "ibkr"
    account_id = "U12345"
    mode = "paper"
}

gateway {
    host = "127.0.0.1"
    port = 4002
}

data {
    source = "ibkr"
    symbols = ["ES", "NQ", "YM"]
    interval = "1d"
}

database {
    url = "postgres://localhost/flux"
    schema = "swing"
}

risk {
    max_daily_loss = -15000.0
    max_weekly_loss = -30000.0
    max_position_per_product = 10
    max_total_notional = 3000000.0
    max_drawdown_pct = 0.08
    correlation_warning_threshold = 4
    initial_equity = 500000.0
}

products {
    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }
    NQ = { multiplier = 20.0, tick_size = 0.25, margin = 21120.0 }
}

strategies {
    aether = { path = "aether/strategy.flux", allocation = 0.6, priority = 1 }
    kairos = { path = "kairos/strategy.flux", allocation = 0.4, priority = 2 }
}
"#
    }

    // ─── Extraction Tests ────────────────────────────────────────────────────

    #[test]
    fn extract_complete_valid_manifest() {
        let (config, _env_sources) = parse_and_extract(full_valid_manifest()).unwrap();

        // Account section
        assert_eq!(config.account.name, "swing");
        assert_eq!(config.account.broker, "ibkr");
        assert_eq!(config.account.account_id, "U12345");
        assert_eq!(config.account.mode, "paper");

        // Gateway section
        assert_eq!(config.gateway.host, "127.0.0.1");
        assert_eq!(config.gateway.port, 4002);

        // Data section
        assert_eq!(config.data.source, "ibkr");
        assert_eq!(config.data.symbols, vec!["ES", "NQ", "YM"]);
        assert_eq!(config.data.interval, "1d");

        // Database section
        assert_eq!(config.database.url, "postgres://localhost/flux");
        assert_eq!(config.database.schema, "swing");

        // Risk section
        assert_eq!(config.risk.max_daily_loss, -15000.0);
        assert_eq!(config.risk.max_weekly_loss, -30000.0);
        assert_eq!(config.risk.max_position_per_product, 10);
        assert_eq!(config.risk.max_total_notional, 3000000.0);
        assert_eq!(config.risk.max_drawdown_pct, 0.08);
        assert_eq!(config.risk.correlation_warning_threshold, 4);
        assert_eq!(config.risk.initial_equity, 500000.0);

        // Products
        assert_eq!(config.products.len(), 2);
        assert_eq!(config.products[0].name, "ES");
        assert_eq!(config.products[0].multiplier, 50.0);
        assert_eq!(config.products[0].tick_size, 0.25);
        assert_eq!(config.products[0].margin, 15840.0);
        assert_eq!(config.products[1].name, "NQ");
        assert_eq!(config.products[1].multiplier, 20.0);

        // Strategies
        assert_eq!(config.strategies.len(), 2);
        assert_eq!(config.strategies[0].name, "aether");
        assert_eq!(config.strategies[0].path, "aether/strategy.flux");
        assert_eq!(config.strategies[0].allocation, 0.6);
        assert_eq!(config.strategies[0].priority, 1);
        assert_eq!(config.strategies[1].name, "kairos");
        assert_eq!(config.strategies[1].path, "kairos/strategy.flux");
        assert_eq!(config.strategies[1].allocation, 0.4);
        assert_eq!(config.strategies[1].priority, 2);
    }

    #[test]
    fn extract_with_env_calls() {
        // Use unique env var names to avoid conflicts with parallel tests
        let var_name_id = "FLUX_TEST_ACCOUNT_ID_44";
        let var_name_url = "FLUX_TEST_DB_URL_44";

        unsafe {
            std::env::set_var(var_name_id, "ENV_ACC_789");
            std::env::set_var(var_name_url, "postgres://envhost/db");
        }

        let source = format!(
            r#"account {{
    name = "test"
    broker = "ibkr"
    account_id = env("{var_name_id}")
    mode = "live"
}}

gateway {{
    host = "localhost"
    port = 4001
}}

data {{
    source = "ibkr"
    symbols = ["ES"]
    interval = "1d"
}}

database {{
    url = env("{var_name_url}")
    schema = "test"
}}

risk {{
    max_daily_loss = -5000.0
    max_weekly_loss = -10000.0
    max_position_per_product = 5
    max_total_notional = 1000000.0
    max_drawdown_pct = 0.05
    correlation_warning_threshold = 3
    initial_equity = 100000.0
}}

products {{
    ES = {{ multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }}
}}

strategies {{
    alpha = {{ path = "alpha/strat.flux", allocation = 1.0, priority = 1 }}
}}
"#
        );

        let (config, env_sources) = parse_and_extract(&source).unwrap();

        // Verify resolved values
        assert_eq!(config.account.account_id, "ENV_ACC_789");
        assert_eq!(config.database.url, "postgres://envhost/db");

        // Verify env_sources tracking
        assert_eq!(
            env_sources.sources.get(&("account".to_string(), "account_id".to_string())),
            Some(&var_name_id.to_string())
        );
        assert_eq!(
            env_sources.sources.get(&("database".to_string(), "url".to_string())),
            Some(&var_name_url.to_string())
        );

        // Clean up
        unsafe {
            std::env::remove_var(var_name_id);
            std::env::remove_var(var_name_url);
        }
    }

    #[test]
    fn error_missing_required_block() {
        // Source missing the `risk` block entirely
        let source = r#"account {
    name = "test"
    broker = "ibkr"
    account_id = "U1"
    mode = "paper"
}

gateway {
    host = "localhost"
    port = 4001
}

data {
    source = "ibkr"
    symbols = ["ES"]
    interval = "1d"
}

database {
    url = "postgres://localhost/db"
    schema = "test"
}

products {
    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }
}

strategies {
    alpha = { path = "alpha.flux", allocation = 1.0, priority = 1 }
}
"#;

        let result = parse_and_extract(source);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        let has_missing_risk = errors.iter().any(|e| matches!(e,
            ExtractionError::MissingBlock { block_name } if block_name == "risk"
        ));
        assert!(has_missing_risk, "Expected MissingBlock error for 'risk', got: {:?}", errors);
    }

    #[test]
    fn error_missing_required_field() {
        // Account block missing broker, account_id, and mode
        let source = r#"account {
    name = "test"
}

gateway {
    host = "localhost"
    port = 4001
}

data {
    source = "ibkr"
    symbols = ["ES"]
    interval = "1d"
}

database {
    url = "postgres://localhost/db"
    schema = "test"
}

risk {
    max_daily_loss = -5000.0
    max_weekly_loss = -10000.0
    max_position_per_product = 5
    max_total_notional = 1000000.0
    max_drawdown_pct = 0.05
    correlation_warning_threshold = 3
    initial_equity = 100000.0
}

products {
    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }
}

strategies {
    alpha = { path = "alpha.flux", allocation = 1.0, priority = 1 }
}
"#;

        let result = parse_and_extract(source);
        assert!(result.is_err());
        let errors = result.unwrap_err();

        // Should have MissingField errors for broker, account_id, mode
        let missing_fields: Vec<&str> = errors
            .iter()
            .filter_map(|e| match e {
                ExtractionError::MissingField { block_name, field_name }
                    if block_name == "account" =>
                {
                    Some(field_name.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(missing_fields.contains(&"broker"), "Expected MissingField for 'broker'");
        assert!(missing_fields.contains(&"account_id"), "Expected MissingField for 'account_id'");
        assert!(missing_fields.contains(&"mode"), "Expected MissingField for 'mode'");
    }

    #[test]
    fn error_type_mismatch() {
        // gateway block: host should be string but we pass int, port should be int but we pass string
        let source = r#"account {
    name = "test"
    broker = "ibkr"
    account_id = "U1"
    mode = "paper"
}

gateway {
    host = 42
    port = "wrong"
}

data {
    source = "ibkr"
    symbols = ["ES"]
    interval = "1d"
}

database {
    url = "postgres://localhost/db"
    schema = "test"
}

risk {
    max_daily_loss = -5000.0
    max_weekly_loss = -10000.0
    max_position_per_product = 5
    max_total_notional = 1000000.0
    max_drawdown_pct = 0.05
    correlation_warning_threshold = 3
    initial_equity = 100000.0
}

products {
    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }
}

strategies {
    alpha = { path = "alpha.flux", allocation = 1.0, priority = 1 }
}
"#;

        let result = parse_and_extract(source);
        assert!(result.is_err());
        let errors = result.unwrap_err();

        let type_mismatches: Vec<(&str, &str)> = errors
            .iter()
            .filter_map(|e| match e {
                ExtractionError::TypeMismatch {
                    block_name,
                    field_name,
                    ..
                } if block_name == "gateway" => Some((block_name.as_str(), field_name.as_str())),
                _ => None,
            })
            .collect();

        assert!(
            type_mismatches.iter().any(|(_, f)| *f == "host"),
            "Expected TypeMismatch for gateway.host"
        );
        assert!(
            type_mismatches.iter().any(|(_, f)| *f == "port"),
            "Expected TypeMismatch for gateway.port"
        );
    }

    #[test]
    fn error_env_var_not_set() {
        // Ensure the env var is definitely not set
        let var_name = "FLUX_TEST_NONEXISTENT_VAR_12345";
        unsafe {
            std::env::remove_var(var_name);
        }

        let source = format!(
            r#"account {{
    name = "test"
    broker = "ibkr"
    account_id = env("{var_name}")
    mode = "paper"
}}

gateway {{
    host = "localhost"
    port = 4001
}}

data {{
    source = "ibkr"
    symbols = ["ES"]
    interval = "1d"
}}

database {{
    url = "postgres://localhost/db"
    schema = "test"
}}

risk {{
    max_daily_loss = -5000.0
    max_weekly_loss = -10000.0
    max_position_per_product = 5
    max_total_notional = 1000000.0
    max_drawdown_pct = 0.05
    correlation_warning_threshold = 3
    initial_equity = 100000.0
}}

products {{
    ES = {{ multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }}
}}

strategies {{
    alpha = {{ path = "alpha.flux", allocation = 1.0, priority = 1 }}
}}
"#
        );

        let result = parse_and_extract(&source);
        assert!(result.is_err());
        let errors = result.unwrap_err();

        let has_env_not_set = errors.iter().any(|e| matches!(e,
            ExtractionError::EnvNotSet { var_name: v, .. } if v == var_name
        ));
        assert!(has_env_not_set, "Expected EnvNotSet error for '{var_name}', got: {:?}", errors);
    }

    // ─── Validation Tests ────────────────────────────────────────────────────

    /// Helper to build a valid AccountConfig for validation tests.
    fn valid_config() -> AccountConfig {
        AccountConfig {
            account: AccountSection {
                name: "test".to_string(),
                broker: "ibkr".to_string(),
                account_id: "U12345".to_string(),
                mode: "paper".to_string(),
            },
            gateway: GatewaySection {
                host: "127.0.0.1".to_string(),
                port: 4002,
            },
            data: DataSection {
                source: "ibkr".to_string(),
                symbols: vec!["ES".to_string()],
                interval: "1d".to_string(),
                replay_file: None,
            },
            database: DatabaseSection {
                url: "postgres://localhost/flux".to_string(),
                schema: "test".to_string(),
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
            products: vec![ProductEntry {
                name: "ES".to_string(),
                multiplier: 50.0,
                tick_size: 0.25,
                margin: 15840.0,
            }],
            strategies: vec![StrategyEntry {
                name: "aether".to_string(),
                path: "aether/strategy.flux".to_string(),
                allocation: 0.6,
                priority: 1,
                execution: None,
                execution_offset_ticks: None,
            }],
            execution_default: None,
        }
    }

    #[test]
    fn valid_config_passes_validator() {
        let config = valid_config();
        let result = validate_config(&config);
        assert!(result.is_ok(), "Expected valid config to pass validation, got: {:?}", result);
    }

    #[test]
    fn validator_each_constraint_violation() {
        // 1. Positive max_daily_loss
        {
            let mut config = valid_config();
            config.risk.max_daily_loss = 5000.0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidRiskField { field, .. } if field == "max_daily_loss"
            )));
        }

        // 2. Positive max_weekly_loss
        {
            let mut config = valid_config();
            config.risk.max_weekly_loss = 1000.0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidRiskField { field, .. } if field == "max_weekly_loss"
            )));
        }

        // 3. max_drawdown_pct out of range (>= 1.0)
        {
            let mut config = valid_config();
            config.risk.max_drawdown_pct = 1.5;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidRiskField { field, .. } if field == "max_drawdown_pct"
            )));
        }

        // 4. max_position_per_product <= 0
        {
            let mut config = valid_config();
            config.risk.max_position_per_product = 0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidRiskField { field, .. } if field == "max_position_per_product"
            )));
        }

        // 5. max_total_notional <= 0
        {
            let mut config = valid_config();
            config.risk.max_total_notional = -100.0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidRiskField { field, .. } if field == "max_total_notional"
            )));
        }

        // 6. initial_equity <= 0
        {
            let mut config = valid_config();
            config.risk.initial_equity = 0.0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidRiskField { field, .. } if field == "initial_equity"
            )));
        }

        // 7. Invalid mode
        {
            let mut config = valid_config();
            config.account.mode = "invalid_mode".to_string();
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidMode { value } if value == "invalid_mode"
            )));
        }

        // 8. Invalid port (0)
        {
            let mut config = valid_config();
            config.gateway.port = 0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidPort { value } if *value == 0
            )));
        }

        // 9. Invalid port (> 65535)
        {
            let mut config = valid_config();
            config.gateway.port = 70000;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidPort { value } if *value == 70000
            )));
        }

        // 10. Invalid allocation (<= 0)
        {
            let mut config = valid_config();
            config.strategies[0].allocation = -0.5;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidAllocation { .. }
            )));
        }

        // 11. Allocation sum > 1.0
        {
            let mut config = valid_config();
            config.strategies = vec![
                StrategyEntry {
                    name: "a".to_string(),
                    path: "a.flux".to_string(),
                    allocation: 0.7,
                    priority: 1,
                    execution: None,
                    execution_offset_ticks: None,
                },
                StrategyEntry {
                    name: "b".to_string(),
                    path: "b.flux".to_string(),
                    allocation: 0.5,
                    priority: 2,
                    execution: None,
                    execution_offset_ticks: None,
                },
            ];
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::AllocationSumExceeded { .. }
            )));
        }

        // 12. Empty strategy path
        {
            let mut config = valid_config();
            config.strategies[0].path = "".to_string();
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidStrategyPath { .. }
            )));
        }

        // 13. Strategy path > 512 chars
        {
            let mut config = valid_config();
            config.strategies[0].path = "x".repeat(513);
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidStrategyPath { .. }
            )));
        }

        // 14. Invalid strategy name (contains spaces)
        {
            let mut config = valid_config();
            config.strategies[0].name = "bad name!".to_string();
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidStrategyName { .. }
            )));
        }

        // 15. Product multiplier <= 0
        {
            let mut config = valid_config();
            config.products[0].multiplier = -1.0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidProductField { field, .. } if field == "multiplier"
            )));
        }

        // 16. Product tick_size <= 0
        {
            let mut config = valid_config();
            config.products[0].tick_size = 0.0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidProductField { field, .. } if field == "tick_size"
            )));
        }

        // 17. Product margin <= 0
        {
            let mut config = valid_config();
            config.products[0].margin = -100.0;
            let errors = validate_config(&config).unwrap_err();
            assert!(errors.iter().any(|e| matches!(e,
                ValidationError::InvalidProductField { field, .. } if field == "margin"
            )));
        }
    }

    #[test]
    fn validator_multiple_violations_all_reported() {
        let mut config = valid_config();
        // Introduce multiple simultaneous violations
        config.risk.max_daily_loss = 5000.0; // should be negative
        config.account.mode = "invalid".to_string(); // should be paper or live
        config.gateway.port = 0; // should be 1-65535

        let errors = validate_config(&config).unwrap_err();

        // Should have at least 3 errors (one for each violation)
        assert!(
            errors.len() >= 3,
            "Expected at least 3 validation errors, got {}: {:?}",
            errors.len(),
            errors
        );

        // Verify each violation type is present
        assert!(errors.iter().any(|e| matches!(e,
            ValidationError::InvalidRiskField { field, .. } if field == "max_daily_loss"
        )), "Missing max_daily_loss violation");

        assert!(errors.iter().any(|e| matches!(e,
            ValidationError::InvalidMode { .. }
        )), "Missing InvalidMode violation");

        assert!(errors.iter().any(|e| matches!(e,
            ValidationError::InvalidPort { .. }
        )), "Missing InvalidPort violation");
    }
}
