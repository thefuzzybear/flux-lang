use std::collections::HashMap;

use super::types::FluxType;

/// A scoped type environment for identifier resolution.
///
/// Scopes are stacked: innermost scope is searched first.
/// Scope levels: global (imports) → strategy (params + state) → handler → block
pub(crate) struct TypeEnvironment {
    scopes: Vec<HashMap<String, FluxType>>,
}

impl TypeEnvironment {
    /// Create a new type environment with a single global scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()], // Start with global scope
        }
    }

    /// Push a new empty scope (entering a block, handler, etc.)
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pop the innermost scope (leaving a block, handler, etc.)
    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Insert a binding into the current (innermost) scope.
    pub fn insert(&mut self, name: String, ty: FluxType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    /// Resolve an identifier by searching from innermost to outermost scope.
    /// Returns None if not found in any scope.
    pub fn resolve(&self, name: &str) -> Option<&FluxType> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    /// Check if a name exists in the current (innermost) scope only.
    pub fn exists_in_current_scope(&self, name: &str) -> bool {
        self.scopes.last().map_or(false, |s| s.contains_key(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::FluxType;

    #[test]
    fn test_new_environment() {
        let env = TypeEnvironment::new();
        // New environment has one scope (the global scope)
        assert_eq!(env.scopes.len(), 1);
        // Resolving an unknown name returns None
        assert_eq!(env.resolve("unknown"), None);
    }

    #[test]
    fn test_insert_and_resolve() {
        let mut env = TypeEnvironment::new();
        env.insert("x".to_string(), FluxType::Int);
        assert_eq!(env.resolve("x"), Some(&FluxType::Int));
    }

    #[test]
    fn test_push_pop_scope() {
        let mut env = TypeEnvironment::new();
        env.insert("outer".to_string(), FluxType::Float);

        env.push_scope();
        env.insert("inner".to_string(), FluxType::Bool);

        // Both are visible from the inner scope
        assert_eq!(env.resolve("inner"), Some(&FluxType::Bool));
        assert_eq!(env.resolve("outer"), Some(&FluxType::Float));

        env.pop_scope();

        // After pop, inner binding is gone
        assert_eq!(env.resolve("inner"), None);
        // Outer binding still accessible
        assert_eq!(env.resolve("outer"), Some(&FluxType::Float));
    }

    #[test]
    fn test_resolution_order() {
        let mut env = TypeEnvironment::new();
        // Define "x" in outer scope as Int
        env.insert("x".to_string(), FluxType::Int);

        env.push_scope();
        // Shadow "x" in inner scope as Float
        env.insert("x".to_string(), FluxType::Float);

        // Resolves to inner (shadowing)
        assert_eq!(env.resolve("x"), Some(&FluxType::Float));

        env.pop_scope();

        // After pop, resolves to outer
        assert_eq!(env.resolve("x"), Some(&FluxType::Int));
    }

    #[test]
    fn test_exists_in_current_scope() {
        let mut env = TypeEnvironment::new();
        env.insert("global_var".to_string(), FluxType::String);

        env.push_scope();
        env.insert("local_var".to_string(), FluxType::Int);

        // "local_var" exists in the current (innermost) scope
        assert!(env.exists_in_current_scope("local_var"));
        // "global_var" does NOT exist in the current scope (only in outer)
        assert!(!env.exists_in_current_scope("global_var"));
        // Unknown name doesn't exist
        assert!(!env.exists_in_current_scope("nope"));
    }

    #[test]
    fn test_nested_scope_isolation() {
        let mut env = TypeEnvironment::new();
        env.insert("persistent".to_string(), FluxType::Bool);

        env.push_scope();
        env.insert("temporary".to_string(), FluxType::Int);
        env.pop_scope();

        // "temporary" is no longer visible after its scope was popped
        assert_eq!(env.resolve("temporary"), None);
        // "persistent" remains
        assert_eq!(env.resolve("persistent"), Some(&FluxType::Bool));
    }

    #[test]
    fn test_multiple_nested_scopes() {
        let mut env = TypeEnvironment::new();
        // Level 0 (global)
        env.insert("a".to_string(), FluxType::Int);

        // Level 1
        env.push_scope();
        env.insert("b".to_string(), FluxType::Float);

        // Level 2
        env.push_scope();
        env.insert("c".to_string(), FluxType::String);

        // Level 3
        env.push_scope();
        env.insert("d".to_string(), FluxType::Bool);

        // All visible from deepest scope
        assert_eq!(env.resolve("a"), Some(&FluxType::Int));
        assert_eq!(env.resolve("b"), Some(&FluxType::Float));
        assert_eq!(env.resolve("c"), Some(&FluxType::String));
        assert_eq!(env.resolve("d"), Some(&FluxType::Bool));

        // Pop level 3 — "d" gone
        env.pop_scope();
        assert_eq!(env.resolve("d"), None);
        assert_eq!(env.resolve("c"), Some(&FluxType::String));

        // Pop level 2 — "c" gone
        env.pop_scope();
        assert_eq!(env.resolve("c"), None);
        assert_eq!(env.resolve("b"), Some(&FluxType::Float));

        // Pop level 1 — "b" gone
        env.pop_scope();
        assert_eq!(env.resolve("b"), None);
        assert_eq!(env.resolve("a"), Some(&FluxType::Int));
    }
}
