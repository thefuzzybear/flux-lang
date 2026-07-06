//! Validation tests for the TextMate grammar and VS Code extension manifest.
//!
//! These integration tests read the static JSON artifacts from disk and validate
//! their structure and regex patterns against expected values.
//!
//! **Validates: Requirements 6.1, 6.2, 7.1, 7.6**

use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;

/// Resolve the project root (two levels up from CARGO_MANIFEST_DIR for flux-cli).
fn project_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn load_grammar() -> Value {
    let path = project_root().join("editors/vscode/syntaxes/flux.tmLanguage.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read grammar file at {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("Grammar file is not valid JSON")
}

fn load_extension_manifest() -> Value {
    let path = project_root().join("editors/vscode/package.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read extension manifest at {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("Extension manifest is not valid JSON")
}

fn load_language_configuration() -> Value {
    let path = project_root().join("editors/vscode/language-configuration.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read language config at {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("Language configuration is not valid JSON")
}

// ============================================================================
// Grammar Structure Validation
// ============================================================================

#[test]
fn grammar_has_correct_scope_name() {
    let grammar = load_grammar();
    assert_eq!(
        grammar["scopeName"].as_str().unwrap(),
        "source.flux",
        "scopeName must be 'source.flux'"
    );
}

#[test]
fn grammar_has_patterns_array() {
    let grammar = load_grammar();
    assert!(
        grammar["patterns"].is_array(),
        "Grammar must have a top-level 'patterns' array"
    );
    assert!(
        !grammar["patterns"].as_array().unwrap().is_empty(),
        "patterns array must not be empty"
    );
}

#[test]
fn grammar_has_repository_object() {
    let grammar = load_grammar();
    assert!(
        grammar["repository"].is_object(),
        "Grammar must have a top-level 'repository' object"
    );
}

#[test]
fn grammar_repository_contains_all_expected_rules() {
    let grammar = load_grammar();
    let repository = grammar["repository"].as_object().unwrap();

    let expected_rules = [
        "comments",
        "keywords-control",
        "keywords-declaration",
        "constants",
        "numbers",
        "strings",
        "signal-functions",
        "builtin-functions",
        "strategy-name",
        "operators",
        "identifiers",
        "punctuation",
    ];

    for rule in &expected_rules {
        assert!(
            repository.contains_key(*rule),
            "Repository is missing expected rule: '{}'",
            rule
        );
    }
}

// ============================================================================
// Extension Manifest Validation
// ============================================================================

#[test]
fn extension_has_correct_name() {
    let manifest = load_extension_manifest();
    assert_eq!(
        manifest["name"].as_str().unwrap(),
        "flux-lang",
        "Extension name must be 'flux-lang'"
    );
}

#[test]
fn extension_has_vscode_engine() {
    let manifest = load_extension_manifest();
    let engines = &manifest["engines"];
    assert!(
        engines.is_object(),
        "Extension must have an 'engines' object"
    );
    assert!(
        engines["vscode"].is_string(),
        "Extension must declare 'engines.vscode' version"
    );
}

#[test]
fn extension_has_contributes_languages() {
    let manifest = load_extension_manifest();
    let languages = &manifest["contributes"]["languages"];
    assert!(
        languages.is_array(),
        "Extension must have 'contributes.languages' array"
    );

    let languages_arr = languages.as_array().unwrap();
    let has_flux = languages_arr
        .iter()
        .any(|lang| lang["id"].as_str() == Some("flux"));
    assert!(
        has_flux,
        "contributes.languages must contain a language with id 'flux'"
    );
}

#[test]
fn extension_has_contributes_grammars() {
    let manifest = load_extension_manifest();
    let grammars = &manifest["contributes"]["grammars"];
    assert!(
        grammars.is_array(),
        "Extension must have 'contributes.grammars' array"
    );

    let grammars_arr = grammars.as_array().unwrap();
    let has_flux_grammar = grammars_arr.iter().any(|g| {
        g["language"].as_str() == Some("flux") && g["scopeName"].as_str() == Some("source.flux")
    });
    assert!(
        has_flux_grammar,
        "contributes.grammars must link 'flux' language to 'source.flux' scope"
    );
}

#[test]
fn extension_has_activation_events() {
    let manifest = load_extension_manifest();
    let events = &manifest["activationEvents"];
    assert!(
        events.is_array(),
        "Extension must have 'activationEvents' array"
    );

    let events_arr = events.as_array().unwrap();
    let has_on_language = events_arr
        .iter()
        .any(|e| e.as_str() == Some("onLanguage:flux"));
    assert!(
        has_on_language,
        "activationEvents must contain 'onLanguage:flux'"
    );
}

// ============================================================================
// Language Configuration Validation
// ============================================================================

#[test]
fn language_config_has_line_comment() {
    let config = load_language_configuration();
    assert_eq!(
        config["comments"]["lineComment"].as_str().unwrap(),
        "#",
        "lineComment must be '#'"
    );
}

#[test]
fn language_config_has_brackets() {
    let config = load_language_configuration();
    assert!(
        config["brackets"].is_array(),
        "Language config must have 'brackets' array"
    );
    assert!(
        !config["brackets"].as_array().unwrap().is_empty(),
        "brackets array must not be empty"
    );
}

#[test]
fn language_config_has_auto_closing_pairs() {
    let config = load_language_configuration();
    assert!(
        config["autoClosingPairs"].is_array(),
        "Language config must have 'autoClosingPairs' array"
    );
    assert!(
        !config["autoClosingPairs"].as_array().unwrap().is_empty(),
        "autoClosingPairs array must not be empty"
    );
}

// ============================================================================
// Grammar Regex Validation (Sample-Based)
// ============================================================================

/// Helper: extract the match regex from a repository rule's first pattern.
fn get_rule_match(grammar: &Value, rule_name: &str) -> String {
    let rule = &grammar["repository"][rule_name];
    let patterns = rule["patterns"].as_array().expect("Rule must have patterns");
    patterns[0]["match"]
        .as_str()
        .expect("First pattern must have a 'match' field")
        .to_string()
}

/// Helper: extract the match regex from a specific pattern index in a rule.
fn get_rule_match_at(grammar: &Value, rule_name: &str, index: usize) -> String {
    let rule = &grammar["repository"][rule_name];
    let patterns = rule["patterns"].as_array().expect("Rule must have patterns");
    patterns[index]["match"]
        .as_str()
        .expect("Pattern must have a 'match' field")
        .to_string()
}

#[test]
fn keywords_control_matches_expected_words() {
    let grammar = load_grammar();
    let pattern = get_rule_match(&grammar, "keywords-control");
    let re = Regex::new(&pattern).expect("keywords-control regex must be valid");

    for keyword in &["if", "else", "for", "while", "return"] {
        assert!(
            re.is_match(keyword),
            "keywords-control pattern must match '{}'",
            keyword
        );
    }

    // Should not match non-keywords
    assert!(
        !re.is_match("iffy"),
        "keywords-control pattern must not match 'iffy' (word boundary)"
    );
}

#[test]
fn signal_functions_matches_expected_words() {
    let grammar = load_grammar();
    let pattern = get_rule_match(&grammar, "signal-functions");
    let re = Regex::new(&pattern).expect("signal-functions regex must be valid");

    for func in &["OPEN", "CLOSE", "CLOSE_QTY"] {
        assert!(
            re.is_match(func),
            "signal-functions pattern must match '{}'",
            func
        );
    }

    // Should not match lowercase variants
    assert!(
        !re.is_match("open"),
        "signal-functions pattern must not match 'open'"
    );
}

#[test]
fn numbers_float_pattern_matches() {
    let grammar = load_grammar();
    // Float pattern is the first in the numbers rule (higher priority)
    let pattern = get_rule_match_at(&grammar, "numbers", 0);
    let re = Regex::new(&pattern).expect("numbers float regex must be valid");

    assert!(re.is_match("3.14"), "Float pattern must match '3.14'");
    assert!(re.is_match("0.5"), "Float pattern must match '0.5'");
    assert!(
        re.is_match("123.456"),
        "Float pattern must match '123.456'"
    );
}

#[test]
fn numbers_integer_pattern_matches() {
    let grammar = load_grammar();
    // Integer pattern is the second in the numbers rule
    let pattern = get_rule_match_at(&grammar, "numbers", 1);
    let re = Regex::new(&pattern).expect("numbers integer regex must be valid");

    assert!(re.is_match("42"), "Integer pattern must match '42'");
    assert!(re.is_match("0"), "Integer pattern must match '0'");
    assert!(re.is_match("12345"), "Integer pattern must match '12345'");
}

#[test]
fn comments_pattern_matches() {
    let grammar = load_grammar();
    let pattern = get_rule_match(&grammar, "comments");
    let re = Regex::new(&pattern).expect("comments regex must be valid");

    assert!(
        re.is_match("# hello"),
        "Comment pattern must match '# hello'"
    );
    assert!(
        re.is_match("# this is a comment"),
        "Comment pattern must match '# this is a comment'"
    );
    assert!(re.is_match("#"), "Comment pattern must match bare '#'");
}

#[test]
fn constants_pattern_matches() {
    let grammar = load_grammar();
    let pattern = get_rule_match(&grammar, "constants");
    let re = Regex::new(&pattern).expect("constants regex must be valid");

    for constant in &["true", "false", "null"] {
        assert!(
            re.is_match(constant),
            "Constants pattern must match '{}'",
            constant
        );
    }

    // Should not match partial matches due to word boundaries
    assert!(
        !re.is_match("trueish"),
        "Constants pattern must not match 'trueish'"
    );
}

#[test]
fn keywords_declaration_matches_expected_words() {
    let grammar = load_grammar();
    let pattern = get_rule_match(&grammar, "keywords-declaration");
    let re = Regex::new(&pattern).expect("keywords-declaration regex must be valid");

    for keyword in &["strategy", "params", "state", "on", "from", "import"] {
        assert!(
            re.is_match(keyword),
            "keywords-declaration pattern must match '{}'",
            keyword
        );
    }
}

#[test]
fn builtin_functions_matches_expected_words() {
    let grammar = load_grammar();
    let pattern = get_rule_match(&grammar, "builtin-functions");
    let re = Regex::new(&pattern).expect("builtin-functions regex must be valid");

    for func in &["sma", "ema", "stddev", "rsi", "atr", "abs", "sqrt", "min", "max"] {
        assert!(
            re.is_match(func),
            "builtin-functions pattern must match '{}'",
            func
        );
    }
}

#[test]
fn operators_logical_matches() {
    let grammar = load_grammar();
    // Logical operators are the first pattern in the operators rule
    let pattern = get_rule_match_at(&grammar, "operators", 0);
    let re = Regex::new(&pattern).expect("operators logical regex must be valid");

    for op in &["and", "or", "not"] {
        assert!(
            re.is_match(op),
            "Logical operators pattern must match '{}'",
            op
        );
    }
}

#[test]
fn operators_comparison_matches() {
    let grammar = load_grammar();
    // Comparison operators are the second pattern in the operators rule
    let pattern = get_rule_match_at(&grammar, "operators", 1);
    let re = Regex::new(&pattern).expect("operators comparison regex must be valid");

    for op in &["==", "!=", "<=", ">=", "<", ">"] {
        assert!(
            re.is_match(op),
            "Comparison operators pattern must match '{}'",
            op
        );
    }
}

#[test]
fn operators_arithmetic_matches() {
    let grammar = load_grammar();
    // Arithmetic operators are the third pattern in the operators rule
    let pattern = get_rule_match_at(&grammar, "operators", 2);
    let re = Regex::new(&pattern).expect("operators arithmetic regex must be valid");

    for op in &["+", "-", "*", "/", "%"] {
        assert!(
            re.is_match(op),
            "Arithmetic operators pattern must match '{}'",
            op
        );
    }
}
