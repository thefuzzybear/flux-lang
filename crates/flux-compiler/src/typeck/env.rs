use std::collections::HashMap;

use super::enum_info::{EnumInfo, MethodInfo};
use super::types::FluxType;

/// A scoped type environment for identifier resolution.
///
/// Scopes are stacked: innermost scope is searched first.
/// Scope levels: global (imports) → strategy (params + state) → handler → block
pub(crate) struct TypeEnvironment {
    scopes: Vec<HashMap<String, FluxType>>,
    /// Registry of enum definitions: enum name → enum info
    enums: HashMap<String, EnumInfo>,
    /// Registry of impl block methods: type name → method name → method info
    impl_methods: HashMap<String, HashMap<String, MethodInfo>>,
}

impl TypeEnvironment {
    /// Create a new type environment with a single global scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()], // Start with global scope
            enums: HashMap::new(),
            impl_methods: HashMap::new(),
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

    /// Register an enum definition in the type environment.
    /// Returns `Ok(())` if successful, `Err(())` if an enum with that name already exists.
    pub fn register_enum(&mut self, info: EnumInfo) -> Result<(), ()> {
        if self.enums.contains_key(&info.name) {
            return Err(());
        }
        self.enums.insert(info.name.clone(), info);
        Ok(())
    }

    /// Look up an enum by name.
    pub fn get_enum(&self, name: &str) -> Option<&EnumInfo> {
        self.enums.get(name)
    }

    /// Check if an enum exists with the given name.
    pub fn has_enum(&self, name: &str) -> bool {
        self.enums.contains_key(name)
    }

    /// Returns an iterator over all registered enum names.
    pub fn enum_names(&self) -> impl Iterator<Item = &str> {
        self.enums.keys().map(|s| s.as_str())
    }

    // --- Impl method registry ---

    /// Register a method for a type. Returns `Err(())` if a method with that name
    /// already exists on the given type.
    pub fn register_method(&mut self, type_name: &str, info: MethodInfo) -> Result<(), ()> {
        let methods = self.impl_methods.entry(type_name.to_string()).or_default();
        if methods.contains_key(&info.name) {
            return Err(());
        }
        methods.insert(info.name.clone(), info);
        Ok(())
    }

    /// Look up a method on a type by name.
    pub fn get_method(&self, type_name: &str, method_name: &str) -> Option<&MethodInfo> {
        self.impl_methods.get(type_name)?.get(method_name)
    }

    /// Check if a type has any registered impl methods.
    pub fn has_methods(&self, type_name: &str) -> bool {
        self.impl_methods.contains_key(type_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::FluxType;
    use super::super::enum_info::{EnumInfo, VariantInfo};
    use crate::lexer::Span;

    fn make_span(start: usize, end: usize) -> Span {
        Span::new(start, end)
    }

    #[test]
    fn test_new_environment() {
        let env = TypeEnvironment::new();
        // New environment has one scope (the global scope)
        assert_eq!(env.scopes.len(), 1);
        // Resolving an unknown name returns None
        assert_eq!(env.resolve("unknown"), None);
        // New environment has no enums
        assert!(env.enums.is_empty());
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

    // ===== Enum registry tests =====

    #[test]
    fn test_register_and_lookup_enum() {
        let mut env = TypeEnvironment::new();
        let enum_info = EnumInfo::new(
            "OrderType".to_string(),
            vec![
                VariantInfo::unit("Market".to_string(), make_span(0, 6)),
                VariantInfo::with_fields(
                    "Limit".to_string(),
                    vec![("price".to_string(), FluxType::Float)],
                    make_span(10, 30),
                ),
            ],
            make_span(0, 40),
        );

        assert!(env.register_enum(enum_info).is_ok());
        assert!(env.has_enum("OrderType"));

        let retrieved = env.get_enum("OrderType").unwrap();
        assert_eq!(retrieved.name, "OrderType");
        assert_eq!(retrieved.variants.len(), 2);
    }

    #[test]
    fn test_register_duplicate_enum_fails() {
        let mut env = TypeEnvironment::new();
        let enum1 = EnumInfo::new(
            "Status".to_string(),
            vec![VariantInfo::unit("Ok".to_string(), make_span(0, 2))],
            make_span(0, 10),
        );
        let enum2 = EnumInfo::new(
            "Status".to_string(),
            vec![VariantInfo::unit("Error".to_string(), make_span(0, 5))],
            make_span(0, 15),
        );

        assert!(env.register_enum(enum1).is_ok());
        assert!(env.register_enum(enum2).is_err()); // Duplicate name
    }

    #[test]
    fn test_enum_not_found() {
        let env = TypeEnvironment::new();
        assert!(env.get_enum("NonExistent").is_none());
        assert!(!env.has_enum("NonExistent"));
    }

    #[test]
    fn test_enum_names_iterator() {
        let mut env = TypeEnvironment::new();
        let enum1 = EnumInfo::new("A".to_string(), vec![], make_span(0, 10));
        let enum2 = EnumInfo::new("B".to_string(), vec![], make_span(0, 10));
        let enum3 = EnumInfo::new("C".to_string(), vec![], make_span(0, 10));

        env.register_enum(enum1).unwrap();
        env.register_enum(enum2).unwrap();
        env.register_enum(enum3).unwrap();

        let mut names: Vec<&str> = env.enum_names().collect();
        names.sort();
        assert_eq!(names, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_variant_lookup_from_env() {
        let mut env = TypeEnvironment::new();
        let enum_info = EnumInfo::new(
            "Color".to_string(),
            vec![
                VariantInfo::unit("Red".to_string(), make_span(0, 3)),
                VariantInfo::unit("Green".to_string(), make_span(5, 10)),
                VariantInfo::unit("Blue".to_string(), make_span(12, 16)),
            ],
            make_span(0, 20),
        );
        env.register_enum(enum_info).unwrap();

        let retrieved = env.get_enum("Color").unwrap();
        assert!(retrieved.find_variant("Red").is_some());
        assert!(retrieved.find_variant("Green").is_some());
        assert!(retrieved.find_variant("Blue").is_some());
        assert!(retrieved.find_variant("Yellow").is_none());
    }
}
