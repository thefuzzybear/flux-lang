use std::collections::HashMap;

use super::{yahoo::YahooProvider, DataFetcher};

/// Build the provider registry mapping names to implementations.
///
/// Returns a HashMap where keys are provider name strings (e.g., "yahoo")
/// and values are boxed trait objects implementing `DataFetcher`.
pub fn build_registry() -> HashMap<String, Box<dyn DataFetcher>> {
    let mut registry: HashMap<String, Box<dyn DataFetcher>> = HashMap::new();
    registry.insert("yahoo".to_string(), Box::new(YahooProvider::new()));
    registry
}

/// Look up a provider by name, returning a helpful error if not found.
///
/// On failure, the error message lists all available provider names
/// so users can easily correct their `--source` flag.
pub fn get_provider<'a>(
    registry: &'a HashMap<String, Box<dyn DataFetcher>>,
    name: &str,
) -> Result<&'a dyn DataFetcher, String> {
    registry.get(name).map(|p| p.as_ref()).ok_or_else(|| {
        let mut available: Vec<&str> = registry.keys().map(|k| k.as_str()).collect();
        available.sort();
        format!(
            "unknown provider '{}'. Available: {}",
            name,
            available.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_registry_contains_yahoo() {
        let registry = build_registry();
        assert!(registry.contains_key("yahoo"));
        assert_eq!(registry.get("yahoo").unwrap().name(), "yahoo");
    }

    #[test]
    fn get_provider_found() {
        let registry = build_registry();
        let provider = get_provider(&registry, "yahoo").unwrap();
        assert_eq!(provider.name(), "yahoo");
    }

    #[test]
    fn get_provider_not_found_lists_available() {
        let registry = build_registry();
        let result = get_provider(&registry, "alpaca");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("unknown provider 'alpaca'"));
        assert!(err.contains("yahoo"));
    }

    #[test]
    fn get_provider_empty_name_not_found() {
        let registry = build_registry();
        let result = get_provider(&registry, "");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("unknown provider ''"));
        assert!(err.contains("Available:"));
    }
}
