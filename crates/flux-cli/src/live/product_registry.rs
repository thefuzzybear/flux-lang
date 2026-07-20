use std::collections::HashMap;

use crate::live::account_config::ProductEntry;

/// Product specification for a single tradeable instrument.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductSpec {
    /// Contract multiplier (e.g., 50.0 for ES, 20.0 for NQ, 1.0 for equities).
    pub multiplier: f64,
    /// Minimum price increment.
    pub tick_size: f64,
    /// Per-contract margin required to open a new position.
    pub margin_initial: f64,
    /// Per-contract margin required to maintain an existing position.
    pub margin_maintenance: f64,
}

/// Registry mapping symbol names to product specifications.
#[derive(Debug, Clone)]
pub struct ProductRegistry {
    products: HashMap<String, ProductSpec>,
}

impl ProductRegistry {
    /// Build a registry from the account manifest's product entries.
    ///
    /// Maps the single `margin` field from ProductEntry to both
    /// `margin_initial` and `margin_maintenance` in ProductSpec.
    pub fn from_entries(entries: &[ProductEntry]) -> Self {
        let products = entries
            .iter()
            .map(|entry| {
                let spec = ProductSpec {
                    multiplier: entry.multiplier,
                    tick_size: entry.tick_size,
                    margin_initial: entry.margin,
                    margin_maintenance: entry.margin,
                };
                (entry.name.clone(), spec)
            })
            .collect();
        Self { products }
    }

    /// Look up a product specification by symbol name.
    ///
    /// Returns `None` if the symbol is not registered.
    pub fn get(&self, symbol: &str) -> Option<&ProductSpec> {
        self.products.get(symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::account_config::ProductEntry;

    #[test]
    fn test_from_entries_and_lookups() {
        let entries = vec![
            ProductEntry {
                name: "ES".to_string(),
                multiplier: 50.0,
                tick_size: 0.25,
                margin: 15000.0,
            },
            ProductEntry {
                name: "NQ".to_string(),
                multiplier: 20.0,
                tick_size: 0.25,
                margin: 18000.0,
            },
        ];
        let registry = ProductRegistry::from_entries(&entries);

        let es = registry.get("ES").unwrap();
        assert_eq!(es.multiplier, 50.0);
        assert_eq!(es.tick_size, 0.25);
        assert_eq!(es.margin_initial, 15000.0);
        assert_eq!(es.margin_maintenance, 15000.0);

        let nq = registry.get("NQ").unwrap();
        assert_eq!(nq.multiplier, 20.0);
        assert_eq!(nq.tick_size, 0.25);
        assert_eq!(nq.margin_initial, 18000.0);
        assert_eq!(nq.margin_maintenance, 18000.0);
    }

    #[test]
    fn test_get_returns_none_for_unknown() {
        let registry = ProductRegistry::from_entries(&[]);
        assert!(registry.get("UNKNOWN").is_none());
    }

    #[test]
    fn test_get_returns_none_for_unknown_with_populated_registry() {
        let entries = vec![ProductEntry {
            name: "ES".to_string(),
            multiplier: 50.0,
            tick_size: 0.25,
            margin: 15000.0,
        }];
        let registry = ProductRegistry::from_entries(&entries);
        assert!(registry.get("NQ").is_none());
    }

    #[test]
    fn test_margin_maps_to_both_initial_and_maintenance() {
        let entries = vec![ProductEntry {
            name: "CL".to_string(),
            multiplier: 1000.0,
            tick_size: 0.01,
            margin: 9500.0,
        }];
        let registry = ProductRegistry::from_entries(&entries);

        let cl = registry.get("CL").unwrap();
        assert_eq!(cl.margin_initial, 9500.0);
        assert_eq!(cl.margin_maintenance, 9500.0);
        // Both fields come from the single ProductEntry.margin value
        assert_eq!(cl.margin_initial, cl.margin_maintenance);
    }
}
