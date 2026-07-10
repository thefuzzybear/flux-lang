//! Type information for the Flux type system.
//!
//! These structures store metadata about enum definitions and impl block methods
//! for use by the typechecker during enum construction validation, match
//! exhaustiveness checking, and method resolution.

use std::collections::HashMap;

use super::typed_ast::TypedStmt;
use super::types::FluxType;
use crate::lexer::Span;

/// Information about an enum definition.
///
/// Stores the enum name, type parameters (for generics), and all variant
/// definitions. This is registered in the type environment when an enum
/// is type-checked.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumInfo {
    /// The enum name (e.g., "OrderType")
    pub name: String,
    /// Type parameters for generic enums (e.g., ["T"] for `enum Result[T]`)
    /// Empty for non-generic enums.
    pub type_params: Vec<String>,
    /// All variants of this enum.
    pub variants: Vec<VariantInfo>,
    /// Source span of the enum definition.
    pub span: Span,
}

impl EnumInfo {
    /// Create a new EnumInfo with no type parameters.
    pub fn new(name: String, variants: Vec<VariantInfo>, span: Span) -> Self {
        Self {
            name,
            type_params: Vec::new(),
            variants,
            span,
        }
    }

    /// Create a new generic EnumInfo with type parameters.
    pub fn with_type_params(
        name: String,
        type_params: Vec<String>,
        variants: Vec<VariantInfo>,
        span: Span,
    ) -> Self {
        Self {
            name,
            type_params,
            variants,
            span,
        }
    }

    /// Look up a variant by name.
    pub fn find_variant(&self, name: &str) -> Option<&VariantInfo> {
        self.variants.iter().find(|v| v.name == name)
    }

    /// Returns true if this is a generic enum (has type parameters).
    pub fn is_generic(&self) -> bool {
        !self.type_params.is_empty()
    }

    /// Returns an iterator over variant names.
    pub fn variant_names(&self) -> impl Iterator<Item = &str> {
        self.variants.iter().map(|v| v.name.as_str())
    }
}

/// Information about an enum variant.
///
/// A variant may be a unit variant (no fields) or a data variant (with named fields).
#[derive(Debug, Clone, PartialEq)]
pub struct VariantInfo {
    /// The variant name (e.g., "Market" or "Limit")
    pub name: String,
    /// Fields of the variant, as (field_name, field_type) pairs.
    /// Empty for unit variants.
    pub fields: Vec<(String, FluxType)>,
    /// Source span of the variant definition.
    pub span: Span,
}

impl VariantInfo {
    /// Create a unit variant (no fields).
    pub fn unit(name: String, span: Span) -> Self {
        Self {
            name,
            fields: Vec::new(),
            span,
        }
    }

    /// Create a data variant with named fields.
    pub fn with_fields(name: String, fields: Vec<(String, FluxType)>, span: Span) -> Self {
        Self {
            name,
            fields,
            span,
        }
    }

    /// Returns true if this is a unit variant (no fields).
    pub fn is_unit(&self) -> bool {
        self.fields.is_empty()
    }

    /// Returns the number of fields.
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Look up a field by name.
    pub fn find_field(&self, name: &str) -> Option<&FluxType> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }

    /// Returns an iterator over field names.
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(|(n, _)| n.as_str())
    }

    /// Returns an iterator over field types.
    pub fn field_types(&self) -> impl Iterator<Item = &FluxType> {
        self.fields.iter().map(|(_, t)| t)
    }
}

/// Registry for enum types, providing lookup by name.
#[derive(Debug, Clone, Default)]
pub struct EnumRegistry {
    /// Maps enum name → enum info
    enums: HashMap<String, EnumInfo>,
}

impl EnumRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            enums: HashMap::new(),
        }
    }

    /// Register an enum definition.
    ///
    /// Returns `Ok(())` if the enum was registered successfully.
    /// Returns `Err(())` if an enum with the same name already exists.
    pub fn register(&mut self, info: EnumInfo) -> Result<(), ()> {
        if self.enums.contains_key(&info.name) {
            return Err(());
        }
        self.enums.insert(info.name.clone(), info);
        Ok(())
    }

    /// Look up an enum by name.
    pub fn get(&self, name: &str) -> Option<&EnumInfo> {
        self.enums.get(name)
    }

    /// Check if an enum exists.
    pub fn contains(&self, name: &str) -> bool {
        self.enums.contains_key(name)
    }

    /// Returns an iterator over all registered enum names.
    pub fn enum_names(&self) -> impl Iterator<Item = &str> {
        self.enums.keys().map(|s| s.as_str())
    }

    /// Returns the number of registered enums.
    pub fn len(&self) -> usize {
        self.enums.len()
    }

    /// Returns true if no enums are registered.
    pub fn is_empty(&self) -> bool {
        self.enums.is_empty()
    }
}

// --- MethodInfo for impl block method registration ---

/// Information about a method defined in an impl block.
///
/// Stores the method signature (parameter types excluding `self`, return type),
/// whether it is static (no `self` parameter), and the typed body for codegen/interpreter.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodInfo {
    /// The method name (e.g., "best_bid")
    pub name: String,
    /// Parameter types excluding `self`.
    pub param_types: Vec<FluxType>,
    /// The return type of the method.
    pub return_type: FluxType,
    /// True if this is a static/associated method (no `self` parameter).
    pub is_static: bool,
    /// The typed method body statements.
    pub body: Vec<TypedStmt>,
    /// Source span of the method definition.
    pub span: Span,
}

// --- TraitInfo for trait definition registration ---

/// Information about a trait definition.
///
/// Stores the trait name, required method signatures, and the source span.
/// Registered in the type environment when a trait is type-checked.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitInfo {
    /// The trait name (e.g., "DataFeed")
    pub name: String,
    /// Required method signatures for this trait.
    pub methods: Vec<TraitMethodInfo>,
    /// Source span of the trait definition.
    pub span: Span,
}

/// Information about a method signature within a trait definition.
///
/// Describes the expected parameter types (excluding `self`), return type,
/// and whether the method takes a `self` parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethodInfo {
    /// The method name (e.g., "next")
    pub name: String,
    /// Parameter types excluding `self`.
    pub param_types: Vec<FluxType>,
    /// The return type of the method.
    pub return_type: FluxType,
    /// Whether this method has a `self` parameter.
    pub has_self: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_span(start: usize, end: usize) -> Span {
        Span::new(start, end)
    }

    #[test]
    fn test_variant_info_unit() {
        let variant = VariantInfo::unit("Market".to_string(), make_span(10, 16));
        assert!(variant.is_unit());
        assert_eq!(variant.field_count(), 0);
        assert_eq!(variant.name, "Market");
    }

    #[test]
    fn test_variant_info_with_fields() {
        let variant = VariantInfo::with_fields(
            "Limit".to_string(),
            vec![
                ("price".to_string(), FluxType::Float),
                ("qty".to_string(), FluxType::Int),
            ],
            make_span(20, 50),
        );
        assert!(!variant.is_unit());
        assert_eq!(variant.field_count(), 2);
        assert_eq!(variant.find_field("price"), Some(&FluxType::Float));
        assert_eq!(variant.find_field("qty"), Some(&FluxType::Int));
        assert_eq!(variant.find_field("missing"), None);
    }

    #[test]
    fn test_enum_info_find_variant() {
        let variants = vec![
            VariantInfo::unit("Market".to_string(), make_span(0, 6)),
            VariantInfo::with_fields(
                "Limit".to_string(),
                vec![("price".to_string(), FluxType::Float)],
                make_span(10, 30)),
        ];
        let enum_info = EnumInfo::new("OrderType".to_string(), variants, make_span(0, 40));

        assert!(enum_info.find_variant("Market").is_some());
        assert!(enum_info.find_variant("Limit").is_some());
        assert!(enum_info.find_variant("Stop").is_none());
        assert!(!enum_info.is_generic());
    }

    #[test]
    fn test_enum_info_generic() {
        let variants = vec![VariantInfo::unit("Some".to_string(), make_span(0, 4))];
        let enum_info = EnumInfo::with_type_params(
            "Option".to_string(),
            vec!["T".to_string()],
            variants,
            make_span(0, 20),
        );

        assert!(enum_info.is_generic());
        assert_eq!(enum_info.type_params, vec!["T"]);
    }

    #[test]
    fn test_enum_registry_register_and_lookup() {
        let mut registry = EnumRegistry::new();
        let enum_info = EnumInfo::new(
            "OrderType".to_string(),
            vec![VariantInfo::unit("Market".to_string(), make_span(0, 6))],
            make_span(0, 20),
        );

        assert!(registry.register(enum_info).is_ok());
        assert!(registry.contains("OrderType"));
        assert!(registry.get("OrderType").is_some());
    }

    #[test]
    fn test_enum_registry_duplicate_name() {
        let mut registry = EnumRegistry::new();
        let enum_info1 = EnumInfo::new(
            "Status".to_string(),
            vec![VariantInfo::unit("Ok".to_string(), make_span(0, 2))],
            make_span(0, 10),
        );
        let enum_info2 = EnumInfo::new(
            "Status".to_string(),
            vec![VariantInfo::unit("Error".to_string(), make_span(0, 5))],
            make_span(0, 15),
        );

        assert!(registry.register(enum_info1).is_ok());
        assert!(registry.register(enum_info2).is_err()); // Duplicate name
    }

    #[test]
    fn test_variant_field_iteration() {
        let variant = VariantInfo::with_fields(
            "Order".to_string(),
            vec![
                ("symbol".to_string(), FluxType::String),
                ("qty".to_string(), FluxType::Int),
                ("price".to_string(), FluxType::Float),
            ],
            make_span(0, 30),
        );

        let names: Vec<&str> = variant.field_names().collect();
        assert_eq!(names, vec!["symbol", "qty", "price"]);

        let types: Vec<&FluxType> = variant.field_types().collect();
        assert_eq!(types, vec![&FluxType::String, &FluxType::Int, &FluxType::Float]);
    }

    #[test]
    fn test_enum_variant_names() {
        let variants = vec![
            VariantInfo::unit("A".to_string(), make_span(0, 1)),
            VariantInfo::unit("B".to_string(), make_span(5, 6)),
            VariantInfo::unit("C".to_string(), make_span(10, 11)),
        ];
        let enum_info = EnumInfo::new("ABC".to_string(), variants, make_span(0, 20));

        let names: Vec<&str> = enum_info.variant_names().collect();
        assert_eq!(names, vec!["A", "B", "C"]);
    }
}
