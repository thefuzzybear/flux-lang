//! Property-based tests for account config extraction and validation.
//!
//! Feature: account-manifest
//!
//! Tests properties 4-8:
//! - Property 4: Env Resolution Correctness
//! - Property 5: Extraction Produces Correct Types
//! - Property 6: Validation Passes Iff Constraints Satisfied
//! - Property 7: Pretty-Printer Full Round-Trip
//! - Property 8: Pretty-Printer Preserves Env References

use std::sync::atomic::{AtomicU64, Ordering};

use flux_cli::live::account_config::*;
use flux_cli::live::manifest_format::format_manifest;
use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::parse_manifest;
use proptest::prelude::*;

// Global counter for unique env var names across test iterations.
static ENV_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique prefix for env vars to avoid cross-test conflicts.
fn unique_env_prefix() -> String {
    let id = ENV_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("FLUXPROP{}", id)
}

// ============================================================================
// Generators
// ============================================================================

/// Generate a valid env var name: [A-Z][A-Z0-9_]{1,15}
fn arb_env_var_name() -> impl Strategy<Value = String> {
    "[A-Z][A-Z0-9_]{1,15}"
}

/// Generate a simple string value (no quotes or backslashes to avoid escaping issues).
fn arb_simple_string() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_/.:-]{1,30}"
}

/// Generate a valid strategy/product name: [a-zA-Z][a-zA-Z0-9_]{0,15}
/// Note: in the manifest parser, entry keys are identifiers so no dashes allowed.
fn arb_valid_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_]{0,15}"
}

/// Generate a valid strategy path: non-empty, <= 512 chars.
fn arb_valid_path() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_/.-]{1,50}"
}

/// Generate a list of 1-4 unique symbols.
fn arb_symbols() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec("[A-Z]{2,4}", 1..=4)
        .prop_filter("unique symbols", |syms| {
            let mut seen = std::collections::HashSet::new();
            syms.iter().all(|s| seen.insert(s.clone()))
        })
}

/// Generate a valid AccountConfig that passes all validation constraints.
fn arb_valid_account_config() -> impl Strategy<Value = AccountConfig> {
    (
        // Group 1: account fields
        (
            arb_simple_string(),  // name
            arb_simple_string(),  // broker
            arb_simple_string(),  // account_id
            prop_oneof![Just("paper".to_string()), Just("live".to_string())], // mode
        ),
        // Group 2: gateway + data
        (
            arb_simple_string(),  // host
            1i64..=65535,         // port
            arb_simple_string(),  // data source
            arb_symbols(),        // symbols
            arb_simple_string(),  // interval
        ),
        // Group 3: database + risk part 1
        (
            arb_simple_string(),  // db url
            arb_simple_string(),  // db schema
            (-100000.0f64..-0.01), // max_daily_loss (negative)
            (-100000.0f64..-0.01), // max_weekly_loss (negative)
        ),
        // Group 4: risk part 2
        (
            1i64..=1000,           // max_position_per_product
            (1.0f64..10000000.0),  // max_total_notional
            (0.01f64..0.99),       // max_drawdown_pct
            1i64..=100,            // correlation_warning_threshold
            (1.0f64..10000000.0),  // initial_equity
        ),
    )
        .prop_flat_map(move |(acct, gw_data, db_risk1, risk2)| {
            let (name, broker, account_id, mode) = acct;
            let (host, port, data_source, symbols, interval) = gw_data;
            let (db_url, db_schema, mdl, mwl) = db_risk1;
            let (mpp, mtn, mdp, cwt, ie) = risk2;

            // Generate 1-3 products with valid fields and unique names
            let products_strat = proptest::collection::vec(
                (
                    (0.01f64..10000.0),  // multiplier
                    (0.001f64..100.0),   // tick_size
                    (0.01f64..100000.0), // margin
                ),
                1..=3,
            );

            // Generate 1-3 strategies with allocations that sum <= 1.0
            let num_strategies = 1usize..=3;

            (products_strat, num_strategies).prop_flat_map(move |(products_raw, n_strats)| {
                let name = name.clone();
                let broker = broker.clone();
                let account_id = account_id.clone();
                let mode = mode.clone();
                let host = host.clone();
                let data_source = data_source.clone();
                let symbols = symbols.clone();
                let interval = interval.clone();
                let db_url = db_url.clone();
                let db_schema = db_schema.clone();
                let products_raw = products_raw.clone();

                // Generate strategy fields with unique names
                let strats_strat = proptest::collection::vec(
                    (arb_valid_path(), 1i64..=10),
                    n_strats..=n_strats,
                );

                strats_strat.prop_map(move |strats_raw| {
                    // Use fixed unique product names to avoid duplicate key issues
                    let product_names = ["ProdA", "ProdB", "ProdC"];
                    let products: Vec<ProductEntry> = products_raw
                        .iter()
                        .enumerate()
                        .map(|(i, (mult, tick, margin))| ProductEntry {
                            name: product_names[i].to_string(),
                            multiplier: *mult,
                            tick_size: *tick,
                            margin: *margin,
                        })
                        .collect();

                    // Divide allocation evenly so sum <= 1.0
                    let n = strats_raw.len() as f64;
                    let alloc_each = (1.0 / n) * 0.9; // 90% of equal share

                    // Use fixed unique strategy names to avoid duplicate key issues
                    let strategy_names = ["strat_alpha", "strat_beta", "strat_gamma"];
                    let strategies: Vec<StrategyEntry> = strats_raw
                        .iter()
                        .enumerate()
                        .map(|(i, (spath, prio))| StrategyEntry {
                            name: strategy_names[i].to_string(),
                            path: spath.clone(),
                            allocation: alloc_each,
                            priority: *prio,
                            execution: None,
                            execution_offset_ticks: None,
                        })
                        .collect();

                    AccountConfig {
                        account: AccountSection {
                            name: name.clone(),
                            broker: broker.clone(),
                            account_id: account_id.clone(),
                            mode: mode.clone(),
                        },
                        gateway: GatewaySection {
                            host: host.clone(),
                            port,
                        },
                        data: DataSection {
                            source: data_source.clone(),
                            symbols: symbols.clone(),
                            interval: interval.clone(),
                        },
                        database: DatabaseSection {
                            url: db_url.clone(),
                            schema: db_schema.clone(),
                        },
                        risk: RiskSection {
                            max_daily_loss: mdl,
                            max_weekly_loss: mwl,
                            max_position_per_product: mpp,
                            max_total_notional: mtn,
                            max_drawdown_pct: mdp,
                            correlation_warning_threshold: cwt,
                            initial_equity: ie,
                        },
                        products,
                        strategies,
                        execution_default: None,
                    }
                })
            })
        })
}

/// Generate an unconstrained AccountConfig (may or may not pass validation).
fn arb_unconstrained_account_config() -> impl Strategy<Value = AccountConfig> {
    (
        // Group 1: account fields (mode can be invalid)
        (
            arb_simple_string(),
            arb_simple_string(),
            arb_simple_string(),
            prop_oneof![
                Just("paper".to_string()),
                Just("live".to_string()),
                Just("invalid".to_string()),
                Just("test".to_string()),
            ],
        ),
        // Group 2: gateway + data (port can be invalid)
        (
            arb_simple_string(),
            prop_oneof![
                (-10i64..0),
                (0i64..1),
                (1i64..=65535),
                (65536i64..70000),
            ],
            arb_simple_string(),
            arb_symbols(),
            arb_simple_string(),
        ),
        // Group 3: database + risk part 1 (values can be positive or negative)
        (
            arb_simple_string(),
            arb_simple_string(),
            (-50000.0f64..50000.0), // max_daily_loss
            (-50000.0f64..50000.0), // max_weekly_loss
        ),
        // Group 4: risk part 2
        (
            (-5i64..100),            // max_position_per_product
            (-1000.0f64..10000000.0), // max_total_notional
            (-0.5f64..1.5),          // max_drawdown_pct
            (-5i64..100),            // correlation_warning_threshold
            (-1000.0f64..10000000.0), // initial_equity
        ),
    )
        .prop_flat_map(move |(acct, gw_data, db_risk1, risk2)| {
            let (name, broker, account_id, mode) = acct;
            let (host, port, data_source, symbols, interval) = gw_data;
            let (db_url, db_schema, mdl, mwl) = db_risk1;
            let (mpp, mtn, mdp, cwt, ie) = risk2;

            // Generate 1-3 products (fields may be invalid)
            let products_strat = proptest::collection::vec(
                (
                    arb_valid_name(),
                    (-100.0f64..10000.0),  // multiplier (can be <= 0)
                    (-10.0f64..100.0),     // tick_size (can be <= 0)
                    (-1000.0f64..100000.0), // margin (can be <= 0)
                ),
                1..=3,
            );

            // Generate 1-3 strategies (allocations can be invalid)
            let strats_strat = proptest::collection::vec(
                (
                    // name: sometimes valid, sometimes invalid
                    prop_oneof![
                        arb_valid_name(),
                        Just("bad name!".to_string()),
                        Just("".to_string()),
                    ],
                    // path: sometimes valid, sometimes invalid
                    prop_oneof![
                        arb_valid_path(),
                        Just("".to_string()),
                    ],
                    (-1.0f64..2.0),  // allocation (can be <= 0 or > 1)
                    1i64..=10,       // priority
                ),
                1..=3,
            );

            (products_strat, strats_strat).prop_map(move |(products_raw, strats_raw)| {
                let products: Vec<ProductEntry> = products_raw
                    .iter()
                    .map(|(pname, mult, tick, margin)| ProductEntry {
                        name: pname.clone(),
                        multiplier: *mult,
                        tick_size: *tick,
                        margin: *margin,
                    })
                    .collect();

                let strategies: Vec<StrategyEntry> = strats_raw
                    .iter()
                    .map(|(sname, spath, alloc, prio)| StrategyEntry {
                        name: sname.clone(),
                        path: spath.clone(),
                        allocation: *alloc,
                        priority: *prio,
                        execution: None,
                        execution_offset_ticks: None,
                    })
                    .collect();

                AccountConfig {
                    account: AccountSection {
                        name: name.clone(),
                        broker: broker.clone(),
                        account_id: account_id.clone(),
                        mode: mode.clone(),
                    },
                    gateway: GatewaySection {
                        host: host.clone(),
                        port,
                    },
                    data: DataSection {
                        source: data_source.clone(),
                        symbols: symbols.clone(),
                        interval: interval.clone(),
                    },
                    database: DatabaseSection {
                        url: db_url.clone(),
                        schema: db_schema.clone(),
                    },
                    risk: RiskSection {
                        max_daily_loss: mdl,
                        max_weekly_loss: mwl,
                        max_position_per_product: mpp,
                        max_total_notional: mtn,
                        max_drawdown_pct: mdp,
                        correlation_warning_threshold: cwt,
                        initial_equity: ie,
                    },
                    products,
                    strategies,
                    execution_default: None,
                }
            })
        })
}

// ============================================================================
// Helpers
// ============================================================================

/// Format a float value for source emission, ensuring it always has a decimal point.
fn format_float_for_source(f: f64) -> String {
    let s = format!("{}", f);
    if s.contains('.') {
        s
    } else {
        format!("{}.0", s)
    }
}

/// Compute expected validation errors for an AccountConfig.
/// Returns the count of expected errors based on the constraints.
fn count_expected_validation_errors(config: &AccountConfig) -> usize {
    let mut count = 0;

    // Risk field checks
    if config.risk.max_daily_loss >= 0.0 { count += 1; }
    if config.risk.max_weekly_loss >= 0.0 { count += 1; }
    if config.risk.max_drawdown_pct <= 0.0 || config.risk.max_drawdown_pct >= 1.0 { count += 1; }
    if config.risk.max_position_per_product <= 0 { count += 1; }
    if config.risk.max_total_notional <= 0.0 { count += 1; }
    if config.risk.initial_equity <= 0.0 { count += 1; }

    // Strategy checks
    let mut allocation_sum = 0.0f64;
    for strategy in &config.strategies {
        if strategy.allocation <= 0.0 { count += 1; }
        allocation_sum += strategy.allocation;
        if strategy.path.is_empty() { count += 1; }
        else if strategy.path.len() > 512 { count += 1; }
        if !is_valid_strategy_name_check(&strategy.name) { count += 1; }
    }
    if allocation_sum > 1.0 { count += 1; }

    // Product checks
    for product in &config.products {
        if product.multiplier <= 0.0 { count += 1; }
        if product.tick_size <= 0.0 { count += 1; }
        if product.margin <= 0.0 { count += 1; }
    }

    // Mode check
    if config.account.mode != "paper" && config.account.mode != "live" { count += 1; }

    // Port check
    if config.gateway.port < 1 || config.gateway.port > 65535 { count += 1; }

    count
}

/// Mirror of the strategy name validation from account_config.rs.
fn is_valid_strategy_name_check(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Build a manifest source string from an AccountConfig (without env vars).
fn build_manifest_source(config: &AccountConfig) -> String {
    let symbols_str = config.data.symbols
        .iter()
        .map(|s| format!("\"{}\"", s))
        .collect::<Vec<_>>()
        .join(", ");

    let mut source = format!(
        r#"account {{
    name = "{}"
    broker = "{}"
    account_id = "{}"
    mode = "{}"
}}

gateway {{
    host = "{}"
    port = {}
}}

data {{
    source = "{}"
    symbols = [{}]
    interval = "{}"
}}

database {{
    url = "{}"
    schema = "{}"
}}

risk {{
    max_daily_loss = {}
    max_weekly_loss = {}
    max_position_per_product = {}
    max_total_notional = {}
    max_drawdown_pct = {}
    correlation_warning_threshold = {}
    initial_equity = {}
}}

"#,
        config.account.name, config.account.broker,
        config.account.account_id, config.account.mode,
        config.gateway.host, config.gateway.port,
        config.data.source, symbols_str, config.data.interval,
        config.database.url, config.database.schema,
        format_float_for_source(config.risk.max_daily_loss),
        format_float_for_source(config.risk.max_weekly_loss),
        config.risk.max_position_per_product,
        format_float_for_source(config.risk.max_total_notional),
        format_float_for_source(config.risk.max_drawdown_pct),
        config.risk.correlation_warning_threshold,
        format_float_for_source(config.risk.initial_equity),
    );

    // Products block
    source.push_str("products {\n");
    for product in &config.products {
        source.push_str(&format!(
            "    {} = {{ multiplier = {}, tick_size = {}, margin = {} }}\n",
            product.name,
            format_float_for_source(product.multiplier),
            format_float_for_source(product.tick_size),
            format_float_for_source(product.margin),
        ));
    }
    source.push_str("}\n\n");

    // Strategies block
    source.push_str("strategies {\n");
    for strategy in &config.strategies {
        source.push_str(&format!(
            "    {} = {{ path = \"{}\", allocation = {}, priority = {} }}\n",
            strategy.name,
            strategy.path,
            format_float_for_source(strategy.allocation),
            strategy.priority,
        ));
    }
    source.push_str("}\n");

    source
}

/// Parse and extract a manifest source, returning the AccountConfig.
fn parse_and_extract(source: &str) -> Result<(AccountConfig, EnvSources), String> {
    let tokens = lex_with_spans(source).map_err(|e| format!("lex error: {}", e))?;
    let ast = parse_manifest(tokens).map_err(|e| format!("parse error: {}", e))?;
    extract_config(&ast).map_err(|errs| {
        format!("extraction errors: {:?}", errs)
    })
}

// ============================================================================
// Property 4: Env Resolution Correctness (Task 10.1)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 2.1, 2.2, 2.5, 4.3, 4.4**
    ///
    /// For any env variable name matching [A-Za-z0-9_]+ that is set in the process
    /// environment, extracting a manifest containing env("VAR_NAME") SHALL produce
    /// the exact string value of that environment variable. For any name not set in
    /// the environment, extraction SHALL return an error identifying the variable name.
    #[test]
    fn prop_env_resolution_correctness(
        var_suffix in arb_env_var_name(),
        env_value in arb_simple_string(),
    ) {
        let prefix = unique_env_prefix();
        let set_var = format!("{}_{}", prefix, var_suffix);
        // Use a different name guaranteed to not be set
        let unset_var = format!("{}_UNSET_{}", prefix, var_suffix);

        // Set the env var
        unsafe { std::env::set_var(&set_var, &env_value); }

        // Ensure unset var is actually unset
        unsafe { std::env::remove_var(&unset_var); }

        // Test 1: env var that IS set resolves correctly
        let source_set = format!(
            r#"account {{
    name = "test"
    broker = "ibkr"
    account_id = env("{set_var}")
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
    url = "postgres://localhost/flux"
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
"#);

        let result = parse_and_extract(&source_set);
        prop_assert!(result.is_ok(), "Extraction should succeed with set env var, got: {:?}", result.err());
        let (config, env_sources) = result.unwrap();
        prop_assert_eq!(&config.account.account_id, &env_value,
            "Resolved env value should match: expected '{}', got '{}'", env_value, config.account.account_id);
        prop_assert!(
            env_sources.sources.contains_key(&("account".to_string(), "account_id".to_string())),
            "EnvSources should track the resolved field"
        );

        // Test 2: env var that is NOT set produces an error
        let source_unset = format!(
            r#"account {{
    name = "test"
    broker = "ibkr"
    account_id = env("{unset_var}")
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
    url = "postgres://localhost/flux"
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
"#);

        let tokens = lex_with_spans(&source_unset).unwrap();
        let ast = parse_manifest(tokens).unwrap();
        let result_unset = extract_config(&ast);
        prop_assert!(result_unset.is_err(), "Extraction should fail with unset env var");
        let errs = result_unset.unwrap_err();
        let has_env_error = errs.iter().any(|e| matches!(e, ExtractionError::EnvNotSet { var_name, .. } if var_name == &unset_var));
        prop_assert!(has_env_error, "Error should identify the unset var '{}', got: {:?}", unset_var, errs);

        // Cleanup
        unsafe { std::env::remove_var(&set_var); }
    }
}

// ============================================================================
// Property 5: Extraction Produces Correct Types (Task 10.2)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 4.1, 4.2, 4.5, 4.6, 4.7**
    ///
    /// For any valid ManifestProgram AST containing all required blocks with all
    /// required fields of correct types, the Config Extractor SHALL produce an
    /// AccountConfig where every field matches the corresponding AST literal value.
    /// For any AST missing a required block or field, the extractor SHALL return
    /// an error identifying the missing element.
    #[test]
    fn prop_extraction_type_correctness(config in arb_valid_account_config()) {
        // Build a manifest source from the generated config
        let source = build_manifest_source(&config);

        // Parse and extract
        let result = parse_and_extract(&source);
        prop_assert!(result.is_ok(),
            "Extraction should succeed for valid config, got error: {:?}\nSource:\n{}",
            result.err(), source);
        let (extracted, _) = result.unwrap();

        // Assert field-by-field equivalence
        prop_assert_eq!(&extracted.account.name, &config.account.name);
        prop_assert_eq!(&extracted.account.broker, &config.account.broker);
        prop_assert_eq!(&extracted.account.account_id, &config.account.account_id);
        prop_assert_eq!(&extracted.account.mode, &config.account.mode);
        prop_assert_eq!(&extracted.gateway.host, &config.gateway.host);
        prop_assert_eq!(extracted.gateway.port, config.gateway.port);
        prop_assert_eq!(&extracted.data.source, &config.data.source);
        prop_assert_eq!(&extracted.data.symbols, &config.data.symbols);
        prop_assert_eq!(&extracted.data.interval, &config.data.interval);
        prop_assert_eq!(&extracted.database.url, &config.database.url);
        prop_assert_eq!(&extracted.database.schema, &config.database.schema);

        // Risk section - use float tolerance for values that go through formatting
        prop_assert!((extracted.risk.max_daily_loss - config.risk.max_daily_loss).abs() < 1e-10,
            "max_daily_loss mismatch: {} vs {}", extracted.risk.max_daily_loss, config.risk.max_daily_loss);
        prop_assert!((extracted.risk.max_weekly_loss - config.risk.max_weekly_loss).abs() < 1e-10,
            "max_weekly_loss mismatch: {} vs {}", extracted.risk.max_weekly_loss, config.risk.max_weekly_loss);
        prop_assert_eq!(extracted.risk.max_position_per_product, config.risk.max_position_per_product);
        prop_assert!((extracted.risk.max_total_notional - config.risk.max_total_notional).abs() < 1e-10,
            "max_total_notional mismatch");
        prop_assert!((extracted.risk.max_drawdown_pct - config.risk.max_drawdown_pct).abs() < 1e-10,
            "max_drawdown_pct mismatch");
        prop_assert_eq!(extracted.risk.correlation_warning_threshold, config.risk.correlation_warning_threshold);
        prop_assert!((extracted.risk.initial_equity - config.risk.initial_equity).abs() < 1e-10,
            "initial_equity mismatch");

        // Products
        prop_assert_eq!(extracted.products.len(), config.products.len());
        for (ep, cp) in extracted.products.iter().zip(config.products.iter()) {
            prop_assert_eq!(&ep.name, &cp.name);
            prop_assert!((ep.multiplier - cp.multiplier).abs() < 1e-10);
            prop_assert!((ep.tick_size - cp.tick_size).abs() < 1e-10);
            prop_assert!((ep.margin - cp.margin).abs() < 1e-10);
        }

        // Strategies
        prop_assert_eq!(extracted.strategies.len(), config.strategies.len());
        for (es, cs) in extracted.strategies.iter().zip(config.strategies.iter()) {
            prop_assert_eq!(&es.name, &cs.name);
            prop_assert_eq!(&es.path, &cs.path);
            prop_assert!((es.allocation - cs.allocation).abs() < 1e-10);
            prop_assert_eq!(es.priority, cs.priority);
        }
    }

    /// **Validates: Requirements 4.5, 4.6, 4.7**
    ///
    /// For any AST missing a required block, the extractor SHALL return an error
    /// identifying the missing block.
    #[test]
    fn prop_extraction_missing_block_error(
        block_idx in 0usize..7,
        config in arb_valid_account_config(),
    ) {
        let block_names = ["account", "gateway", "data", "database", "risk", "products", "strategies"];
        let missing_block = block_names[block_idx];

        // Build source but omit one block
        let full_source = build_manifest_source(&config);
        // Remove the block by filtering lines
        let filtered = remove_block_from_source(&full_source, missing_block);

        let tokens = lex_with_spans(&filtered).unwrap();
        let ast = parse_manifest(tokens).unwrap();
        let result = extract_config(&ast);

        prop_assert!(result.is_err(),
            "Extraction should fail when '{}' block is missing", missing_block);
        let errs = result.unwrap_err();
        let has_missing = errs.iter().any(|e| matches!(e, ExtractionError::MissingBlock { block_name } if block_name == missing_block));
        prop_assert!(has_missing,
            "Error should identify missing block '{}', got: {:?}", missing_block, errs);
    }
}

/// Remove a named block from manifest source text.
fn remove_block_from_source(source: &str, block_name: &str) -> String {
    let mut result = String::new();
    let mut skip = false;
    let mut brace_depth = 0;

    for line in source.lines() {
        if !skip {
            let trimmed = line.trim();
            // Detect block start: line starts with "blockname" followed by whitespace and "{"
            if trimmed.starts_with(block_name) {
                let rest = trimmed[block_name.len()..].trim();
                if rest == "{" || rest.starts_with("{ ") || rest.starts_with("{") {
                    skip = true;
                    // Count braces on this opening line
                    brace_depth = 0;
                    for ch in line.chars() {
                        match ch {
                            '{' => brace_depth += 1,
                            '}' => brace_depth -= 1,
                            _ => {}
                        }
                    }
                    if brace_depth == 0 {
                        skip = false;
                    }
                    continue;
                }
            }
            result.push_str(line);
            result.push('\n');
        } else {
            // Count braces to find the end of the block
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            skip = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    result
}

// ============================================================================
// Property 6: Validation Passes Iff Constraints Satisfied (Task 10.3)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 5.1, 5.4, 5.5, 5.6, 5.7, 5.8, 5.9, 5.10**
    ///
    /// For any AccountConfig, the validator SHALL pass if and only if all constraints
    /// are satisfied. The validator SHALL return all violations as a collection,
    /// not just the first.
    #[test]
    fn prop_validation_correctness(config in arb_unconstrained_account_config()) {
        let expected_errors = count_expected_validation_errors(&config);
        let result = validate_config(&config);

        if expected_errors == 0 {
            // All constraints satisfied → validator should pass
            prop_assert!(result.is_ok(),
                "Validator should pass when all constraints are satisfied, but got errors: {:?}",
                result.err());
        } else {
            // Some constraints violated → validator should fail
            prop_assert!(result.is_err(),
                "Validator should fail when {} constraint(s) violated, but it passed.\nConfig: {:?}",
                expected_errors, config);
            let errors = result.unwrap_err();
            // All violations should be collected (not just first)
            prop_assert_eq!(errors.len(), expected_errors,
                "Expected {} validation errors, got {}.\nErrors: {:?}\nConfig: {:?}",
                expected_errors, errors.len(), errors, config);
        }
    }
}

// ============================================================================
// Property 7: Pretty-Printer Full Round-Trip (Task 10.4)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 8.1, 8.3, 8.4**
    ///
    /// For any valid AccountConfig and corresponding EnvSources, pretty-printing
    /// the config to source text, then parsing that text with the manifest parser,
    /// then extracting an AccountConfig from the resulting AST (with the referenced
    /// env vars set), SHALL produce a field-by-field equivalent AccountConfig.
    #[test]
    fn prop_pretty_printer_full_roundtrip(config in arb_valid_account_config()) {
        let prefix = unique_env_prefix();

        // Create EnvSources for account_id and database url
        let mut env_sources = EnvSources::default();
        let acct_var = format!("{}_ACCT_ID", prefix);
        let db_var = format!("{}_DB_URL", prefix);

        env_sources.sources.insert(
            ("account".to_string(), "account_id".to_string()),
            acct_var.clone(),
        );
        env_sources.sources.insert(
            ("database".to_string(), "url".to_string()),
            db_var.clone(),
        );

        // Set env vars to the actual config values so extraction resolves them
        unsafe {
            std::env::set_var(&acct_var, &config.account.account_id);
            std::env::set_var(&db_var, &config.database.url);
        }

        // Pretty-print
        let formatted = format_manifest(&config, &env_sources);

        // Parse the output
        let tokens = lex_with_spans(&formatted)
            .map_err(|e| TestCaseError::Fail(format!("Lex failed: {}\nSource:\n{}", e, formatted).into()))?;
        let ast = parse_manifest(tokens)
            .map_err(|e| TestCaseError::Fail(format!("Parse failed: {}\nSource:\n{}", e, formatted).into()))?;

        // Extract (with env vars set)
        let (extracted, _) = extract_config(&ast)
            .map_err(|e| TestCaseError::Fail(format!("Extract failed: {:?}\nSource:\n{}", e, formatted).into()))?;

        // Assert field-by-field equivalence
        prop_assert_eq!(&extracted.account.name, &config.account.name);
        prop_assert_eq!(&extracted.account.broker, &config.account.broker);
        prop_assert_eq!(&extracted.account.account_id, &config.account.account_id);
        prop_assert_eq!(&extracted.account.mode, &config.account.mode);
        prop_assert_eq!(&extracted.gateway.host, &config.gateway.host);
        prop_assert_eq!(extracted.gateway.port, config.gateway.port);
        prop_assert_eq!(&extracted.data.source, &config.data.source);
        prop_assert_eq!(&extracted.data.symbols, &config.data.symbols);
        prop_assert_eq!(&extracted.data.interval, &config.data.interval);
        prop_assert_eq!(&extracted.database.url, &config.database.url);
        prop_assert_eq!(&extracted.database.schema, &config.database.schema);

        // Risk section
        prop_assert!((extracted.risk.max_daily_loss - config.risk.max_daily_loss).abs() < 1e-10);
        prop_assert!((extracted.risk.max_weekly_loss - config.risk.max_weekly_loss).abs() < 1e-10);
        prop_assert_eq!(extracted.risk.max_position_per_product, config.risk.max_position_per_product);
        prop_assert!((extracted.risk.max_total_notional - config.risk.max_total_notional).abs() < 1e-10);
        prop_assert!((extracted.risk.max_drawdown_pct - config.risk.max_drawdown_pct).abs() < 1e-10);
        prop_assert_eq!(extracted.risk.correlation_warning_threshold, config.risk.correlation_warning_threshold);
        prop_assert!((extracted.risk.initial_equity - config.risk.initial_equity).abs() < 1e-10);

        // Products
        prop_assert_eq!(extracted.products.len(), config.products.len());
        for (ep, cp) in extracted.products.iter().zip(config.products.iter()) {
            prop_assert_eq!(&ep.name, &cp.name);
            prop_assert!((ep.multiplier - cp.multiplier).abs() < 1e-10);
            prop_assert!((ep.tick_size - cp.tick_size).abs() < 1e-10);
            prop_assert!((ep.margin - cp.margin).abs() < 1e-10);
        }

        // Strategies
        prop_assert_eq!(extracted.strategies.len(), config.strategies.len());
        for (es, cs) in extracted.strategies.iter().zip(config.strategies.iter()) {
            prop_assert_eq!(&es.name, &cs.name);
            prop_assert_eq!(&es.path, &cs.path);
            prop_assert!((es.allocation - cs.allocation).abs() < 1e-10);
            prop_assert_eq!(es.priority, cs.priority);
        }

        // Cleanup env vars
        unsafe {
            std::env::remove_var(&acct_var);
            std::env::remove_var(&db_var);
        }
    }
}

// ============================================================================
// Property 8: Pretty-Printer Preserves Env References (Task 10.5)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 8.2**
    ///
    /// For any AccountConfig field that is recorded in EnvSources as originating
    /// from an env("VAR_NAME") call, the pretty-printed output SHALL contain the
    /// string env("VAR_NAME") for that field rather than the resolved value.
    #[test]
    fn prop_pretty_printer_env_preservation(
        config in arb_valid_account_config(),
        env_acct_id in arb_env_var_name(),
        env_db_url in arb_env_var_name(),
        env_gw_host in arb_env_var_name(),
        use_acct_env in proptest::bool::ANY,
        use_db_env in proptest::bool::ANY,
        use_gw_env in proptest::bool::ANY,
    ) {
        let prefix = unique_env_prefix();
        let mut env_sources = EnvSources::default();

        let acct_var = format!("{}_{}", prefix, env_acct_id);
        let db_var = format!("{}_{}", prefix, env_db_url);
        let gw_var = format!("{}_{}", prefix, env_gw_host);

        // Conditionally mark some fields as env-sourced
        if use_acct_env {
            env_sources.sources.insert(
                ("account".to_string(), "account_id".to_string()),
                acct_var.clone(),
            );
        }
        if use_db_env {
            env_sources.sources.insert(
                ("database".to_string(), "url".to_string()),
                db_var.clone(),
            );
        }
        if use_gw_env {
            env_sources.sources.insert(
                ("gateway".to_string(), "host".to_string()),
                gw_var.clone(),
            );
        }

        // Pretty-print
        let output = format_manifest(&config, &env_sources);

        // Assert: env-sourced fields show env("VAR") in output
        if use_acct_env {
            let expected = format!("account_id = env(\"{}\")", acct_var);
            prop_assert!(output.contains(&expected),
                "Output should contain '{}' for env-sourced account_id, got:\n{}",
                expected, output);
            // Should NOT contain the literal resolved value for that field
            let literal = format!("account_id = \"{}\"", config.account.account_id);
            prop_assert!(!output.contains(&literal),
                "Output should NOT contain literal '{}' when field is env-sourced",
                literal);
        } else {
            // Non-env field should show actual value
            let literal = format!("account_id = \"{}\"", config.account.account_id);
            prop_assert!(output.contains(&literal),
                "Output should contain literal '{}' for non-env account_id, got:\n{}",
                literal, output);
        }

        if use_db_env {
            let expected = format!("url = env(\"{}\")", db_var);
            prop_assert!(output.contains(&expected),
                "Output should contain '{}' for env-sourced url, got:\n{}",
                expected, output);
        } else {
            let literal = format!("url = \"{}\"", config.database.url);
            prop_assert!(output.contains(&literal),
                "Output should contain literal '{}' for non-env url, got:\n{}",
                literal, output);
        }

        if use_gw_env {
            let expected = format!("host = env(\"{}\")", gw_var);
            prop_assert!(output.contains(&expected),
                "Output should contain '{}' for env-sourced host, got:\n{}",
                expected, output);
        } else {
            let literal = format!("host = \"{}\"", config.gateway.host);
            prop_assert!(output.contains(&literal),
                "Output should contain literal '{}' for non-env host, got:\n{}",
                literal, output);
        }
    }
}
