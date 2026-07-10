//! Exhaustiveness checking for match expressions.
//!
//! Verifies that a match expression covers all variants of the scrutinee enum,
//! or that a wildcard pattern is present as a catch-all.

use super::enum_info::EnumInfo;
use crate::parser::ast::Pattern;

/// Result of an exhaustiveness check.
#[derive(Debug, Clone, PartialEq)]
pub enum ExhaustivenessResult {
    /// All variants are covered (either explicitly or via wildcard).
    Exhaustive,
    /// Some variants are not covered and no wildcard is present.
    NonExhaustive {
        /// The names of uncovered variants.
        missing_variants: Vec<String>,
    },
}

/// Check whether a set of match arm patterns exhaustively covers all variants
/// of the given enum.
///
/// A match is exhaustive if:
/// 1. Every variant of the enum appears in at least one arm pattern, OR
/// 2. A wildcard `_` pattern is present.
///
/// # Arguments
///
/// * `enum_info` - The enum type being matched on
/// * `patterns` - The patterns from all match arms
///
/// # Returns
///
/// `ExhaustivenessResult::Exhaustive` if all variants are covered,
/// otherwise `ExhaustivenessResult::NonExhaustive` with the list of missing variants.
pub fn check_exhaustiveness(enum_info: &EnumInfo, patterns: &[Pattern]) -> ExhaustivenessResult {
    // If any pattern is a wildcard, the match is exhaustive
    for pattern in patterns {
        if matches!(pattern, Pattern::Wildcard { .. }) {
            return ExhaustivenessResult::Exhaustive;
        }
    }

    // Collect all variant names that are covered by explicit patterns
    let mut covered: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for pattern in patterns {
        if let Pattern::Variant { variant_name, .. } = pattern {
            covered.insert(variant_name.as_str());
        }
    }

    // Find all variants that are NOT covered
    let missing: Vec<String> = enum_info
        .variants
        .iter()
        .filter(|v| !covered.contains(v.name.as_str()))
        .map(|v| v.name.clone())
        .collect();

    if missing.is_empty() {
        ExhaustivenessResult::Exhaustive
    } else {
        ExhaustivenessResult::NonExhaustive {
            missing_variants: missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;
    use crate::typeck::enum_info::VariantInfo;
    use crate::typeck::types::FluxType;

    fn make_span(start: usize, end: usize) -> Span {
        Span::new(start, end)
    }

    fn order_type_enum() -> EnumInfo {
        EnumInfo::new(
            "OrderType".to_string(),
            vec![
                VariantInfo::unit("Market".to_string(), make_span(0, 6)),
                VariantInfo::with_fields(
                    "Limit".to_string(),
                    vec![("price".to_string(), FluxType::Float)],
                    make_span(10, 30),
                ),
                VariantInfo::unit("Stop".to_string(), make_span(35, 39)),
            ],
            make_span(0, 50),
        )
    }

    #[test]
    fn test_all_variants_covered() {
        let enum_info = order_type_enum();
        let patterns = vec![
            Pattern::Variant {
                enum_name: "OrderType".to_string(),
                variant_name: "Market".to_string(),
                bindings: vec![],
                span: make_span(0, 10),
            },
            Pattern::Variant {
                enum_name: "OrderType".to_string(),
                variant_name: "Limit".to_string(),
                bindings: vec!["p".to_string()],
                span: make_span(20, 30),
            },
            Pattern::Variant {
                enum_name: "OrderType".to_string(),
                variant_name: "Stop".to_string(),
                bindings: vec![],
                span: make_span(40, 50),
            },
        ];

        assert_eq!(
            check_exhaustiveness(&enum_info, &patterns),
            ExhaustivenessResult::Exhaustive
        );
    }

    #[test]
    fn test_wildcard_makes_exhaustive() {
        let enum_info = order_type_enum();
        let patterns = vec![
            Pattern::Variant {
                enum_name: "OrderType".to_string(),
                variant_name: "Market".to_string(),
                bindings: vec![],
                span: make_span(0, 10),
            },
            Pattern::Wildcard {
                span: make_span(20, 21),
            },
        ];

        assert_eq!(
            check_exhaustiveness(&enum_info, &patterns),
            ExhaustivenessResult::Exhaustive
        );
    }

    #[test]
    fn test_missing_variant_detected() {
        let enum_info = order_type_enum();
        let patterns = vec![
            Pattern::Variant {
                enum_name: "OrderType".to_string(),
                variant_name: "Market".to_string(),
                bindings: vec![],
                span: make_span(0, 10),
            },
            Pattern::Variant {
                enum_name: "OrderType".to_string(),
                variant_name: "Limit".to_string(),
                bindings: vec!["p".to_string()],
                span: make_span(20, 30),
            },
            // Missing: Stop
        ];

        assert_eq!(
            check_exhaustiveness(&enum_info, &patterns),
            ExhaustivenessResult::NonExhaustive {
                missing_variants: vec!["Stop".to_string()],
            }
        );
    }

    #[test]
    fn test_multiple_missing_variants() {
        let enum_info = order_type_enum();
        let patterns = vec![Pattern::Variant {
            enum_name: "OrderType".to_string(),
            variant_name: "Market".to_string(),
            bindings: vec![],
            span: make_span(0, 10),
        }];

        let result = check_exhaustiveness(&enum_info, &patterns);
        match result {
            ExhaustivenessResult::NonExhaustive { missing_variants } => {
                assert_eq!(missing_variants.len(), 2);
                assert!(missing_variants.contains(&"Limit".to_string()));
                assert!(missing_variants.contains(&"Stop".to_string()));
            }
            _ => panic!("expected NonExhaustive"),
        }
    }

    #[test]
    fn test_empty_patterns_all_missing() {
        let enum_info = order_type_enum();
        let patterns: Vec<Pattern> = vec![];

        let result = check_exhaustiveness(&enum_info, &patterns);
        match result {
            ExhaustivenessResult::NonExhaustive { missing_variants } => {
                assert_eq!(missing_variants.len(), 3);
            }
            _ => panic!("expected NonExhaustive"),
        }
    }

    #[test]
    fn test_single_variant_enum_covered() {
        let enum_info = EnumInfo::new(
            "Status".to_string(),
            vec![VariantInfo::unit("Active".to_string(), make_span(0, 6))],
            make_span(0, 10),
        );
        let patterns = vec![Pattern::Variant {
            enum_name: "Status".to_string(),
            variant_name: "Active".to_string(),
            bindings: vec![],
            span: make_span(0, 10),
        }];

        assert_eq!(
            check_exhaustiveness(&enum_info, &patterns),
            ExhaustivenessResult::Exhaustive
        );
    }

    #[test]
    fn test_wildcard_only() {
        let enum_info = order_type_enum();
        let patterns = vec![Pattern::Wildcard {
            span: make_span(0, 1),
        }];

        assert_eq!(
            check_exhaustiveness(&enum_info, &patterns),
            ExhaustivenessResult::Exhaustive
        );
    }
}
