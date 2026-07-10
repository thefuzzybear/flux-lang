//! Property-based tests for type system codegen (enums, impl blocks).
//!
//! Feature: flux-type-system, Property 14: Codegen Enum Output Validity
//! Feature: flux-type-system, Property 15: Codegen Impl/Trait Output Validity (impl portion)
//! Uses proptest to verify that enum definitions, construction expressions,
//! and impl blocks emit valid Rust code with correct syntax.

#[cfg(test)]
mod tests {
    use crate::codegen::generate;
    use crate::lexer::Span;
    use crate::typeck::typed_ast::*;
    use crate::typeck::types::FluxType;
    use proptest::prelude::*;

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a valid Rust/Flux identifier for enum/variant/field names.
    /// PascalCase for enum and variant names, snake_case for field names.
    fn arb_pascal_ident() -> impl Strategy<Value = String> {
        "[A-Z][a-z]{2,6}"
    }

    fn arb_field_ident() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{1,5}".prop_filter("must not be Rust keyword", |n| {
            !matches!(
                n.as_str(),
                "as" | "break" | "const" | "continue" | "crate" | "else" | "enum"
                    | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in"
                    | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub"
                    | "ref" | "return" | "self" | "static" | "struct" | "super"
                    | "trait" | "true" | "type" | "unsafe" | "use" | "where"
                    | "while" | "async" | "await" | "dyn"
            )
        })
    }

    /// Generate a field type suitable for enum variant fields.
    fn arb_enum_field_type() -> impl Strategy<Value = FluxType> {
        prop_oneof![
            Just(FluxType::Int),
            Just(FluxType::Float),
            Just(FluxType::String),
            Just(FluxType::Bool),
        ]
    }

    /// Generate a typed enum variant (unit or data variant with 1-3 fields).
    fn arb_typed_enum_variant() -> impl Strategy<Value = TypedEnumVariant> {
        let unit_variant = arb_pascal_ident().prop_map(|name| TypedEnumVariant {
            name,
            fields: vec![],
            span: Span::new(0, 1),
        });

        let data_variant = (
            arb_pascal_ident(),
            prop::collection::vec((arb_field_ident(), arb_enum_field_type()), 1..=3),
        )
            .prop_map(|(name, fields)| {
                // Deduplicate field names by appending index
                let mut seen = std::collections::HashSet::new();
                let deduped_fields: Vec<(String, FluxType)> = fields
                    .into_iter()
                    .enumerate()
                    .map(|(i, (fname, ftype))| {
                        let unique_name = if seen.contains(&fname) {
                            format!("{}_{}", fname, i)
                        } else {
                            seen.insert(fname.clone());
                            fname
                        };
                        (unique_name, ftype)
                    })
                    .collect();
                TypedEnumVariant {
                    name,
                    fields: deduped_fields,
                    span: Span::new(0, 1),
                }
            });

        prop_oneof![unit_variant, data_variant]
    }

    /// Generate a TypedEnumDef with 1-5 variants (mix of unit and data).
    fn arb_typed_enum_def() -> impl Strategy<Value = TypedEnumDef> {
        (
            arb_pascal_ident(),
            prop::collection::vec(arb_typed_enum_variant(), 1..=5),
        )
            .prop_map(|(name, variants)| {
                // Deduplicate variant names
                let mut seen = std::collections::HashSet::new();
                let deduped_variants: Vec<TypedEnumVariant> = variants
                    .into_iter()
                    .enumerate()
                    .map(|(i, mut v)| {
                        if seen.contains(&v.name) {
                            v.name = format!("{}{}", v.name, i);
                        }
                        seen.insert(v.name.clone());
                        v
                    })
                    .collect();
                TypedEnumDef {
                    name,
                    type_params: vec![],
                    variants: deduped_variants,
                    span: Span::new(0, 1),
                }
            })
    }

    /// Helper: build a minimal TypedProgram containing enum definitions.
    fn build_enum_program(enums: Vec<TypedEnumDef>) -> TypedProgram {
        TypedProgram {
            imports: vec![],
            structs: vec![],
            enums,
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "EnumTest".to_string(),
                body: vec![],
                span: Span::new(0, 10),
            },
            span: Span::new(0, 10),
        }
    }

    /// Helper: build a program with an enum def and a construction expression
    /// in the event handler.
    fn build_enum_construction_program(
        enum_def: TypedEnumDef,
        enum_name: String,
        variant_name: String,
        args: Vec<TypedExpr>,
    ) -> TypedProgram {
        let construction_expr = TypedExpr {
            kind: TypedExprKind::EnumConstruction {
                enum_name: enum_name.clone(),
                variant_name,
                args,
            },
            resolved_type: FluxType::Enum(enum_name),
            span: Span::new(50, 80),
        };

        TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![enum_def],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "EnumConstructTest".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![TypedStmt::Expr(TypedExprStmt {
                        expr: construction_expr,
                        span: Span::new(50, 80),
                    })],
                    span: Span::new(40, 90),
                })],
                span: Span::new(0, 100),
            },
            span: Span::new(0, 100),
        }
    }

    /// Generate a literal TypedExpr consistent with a given FluxType.
    fn arb_literal_for_type(ty: &FluxType) -> BoxedStrategy<TypedExpr> {
        let span = Span::new(0, 1);
        match ty {
            FluxType::Int => (0i64..1000)
                .prop_map(move |v| TypedExpr {
                    kind: TypedExprKind::IntLiteral(v),
                    resolved_type: FluxType::Int,
                    span,
                })
                .boxed(),
            FluxType::Float => (1u32..999)
                .prop_map(move |v| TypedExpr {
                    kind: TypedExprKind::FloatLiteral(v as f64 + 0.5),
                    resolved_type: FluxType::Float,
                    span,
                })
                .boxed(),
            FluxType::String => "[a-z]{1,5}"
                .prop_map(move |s| TypedExpr {
                    kind: TypedExprKind::StringLiteral(s),
                    resolved_type: FluxType::String,
                    span,
                })
                .boxed(),
            FluxType::Bool => any::<bool>()
                .prop_map(move |b| TypedExpr {
                    kind: TypedExprKind::BoolLiteral(b),
                    resolved_type: FluxType::Bool,
                    span,
                })
                .boxed(),
            _ => Just(TypedExpr {
                kind: TypedExprKind::IntLiteral(0),
                resolved_type: FluxType::Int,
                span,
            })
            .boxed(),
        }
    }

    // ========================================================================
    // Property 14: Codegen Enum Output Validity
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-type-system, Property 14: Codegen Enum Output Validity
        /// **Validates: Requirements 1.7, 2.7, 3.10**
        ///
        /// For any valid typed enum definition (with any mix of unit and data variants),
        /// the codegen stage SHALL emit Rust source containing:
        /// - `#[derive(Debug, Clone, PartialEq)]`
        /// - A syntactically valid `enum EnumName {` declaration
        /// - Unit variants appearing without braces
        /// - Data variants appearing with struct-style fields `{ field: Type }`
        #[test]
        fn prop_codegen_enum_definition_validity(enum_def in arb_typed_enum_def()) {
            let enum_name = enum_def.name.clone();
            let variants = enum_def.variants.clone();
            let program = build_enum_program(vec![enum_def]);

            let result = generate(&program);
            prop_assert!(result.is_ok(), "generate() failed: {:?}", result.err());
            let output = result.unwrap();

            // 1. Must contain derive attributes
            prop_assert!(
                output.contains("#[derive(Debug, Clone, PartialEq)]"),
                "Output must contain derive attributes, got:\n{}", output
            );

            // 2. Must contain `enum EnumName {`
            let enum_header = format!("enum {} {{", enum_name);
            prop_assert!(
                output.contains(&enum_header),
                "Output must contain '{}', got:\n{}", enum_header, output
            );

            // 3. Verify each variant appears correctly in output
            for variant in &variants {
                if variant.fields.is_empty() {
                    // Unit variant: should appear as `VariantName,` (no braces)
                    prop_assert!(
                        output.contains(&variant.name),
                        "Output must contain unit variant '{}', got:\n{}", variant.name, output
                    );
                    // Unit variant should NOT have `{ }` immediately after its name
                    // Count occurrences in the enum block only
                    let enum_block_start = output.find(&enum_header).unwrap();
                    let enum_block = &output[enum_block_start..];
                    // For unit variants, the variant name should be followed by `,`
                    // not by `{` (unless it's also a data variant name coincidentally)
                    let variant_line = enum_block
                        .lines()
                        .find(|line| line.trim().starts_with(&variant.name));
                    if let Some(line) = variant_line {
                        let trimmed = line.trim();
                        // Unit variant line should end with `,` and NOT contain `{`
                        prop_assert!(
                            !trimmed.contains('{'),
                            "Unit variant '{}' should not have braces, got line: '{}'",
                            variant.name, trimmed
                        );
                    }
                } else {
                    // Data variant: should appear with struct-style fields `{ field: Type }`
                    let data_pattern = format!("{} {{", variant.name);
                    prop_assert!(
                        output.contains(&data_pattern),
                        "Output must contain data variant '{}' with braces, got:\n{}",
                        variant.name, output
                    );
                    // Verify each field name appears in the output
                    for (field_name, _field_type) in &variant.fields {
                        prop_assert!(
                            output.contains(field_name),
                            "Output must contain field '{}' of variant '{}', got:\n{}",
                            field_name, variant.name, output
                        );
                    }
                }
            }
        }

        /// **Validates: Requirements 2.7**
        ///
        /// For any enum construction expression, the codegen SHALL emit Rust code
        /// using `::` separator (not `.`) for variant access.
        #[test]
        fn prop_codegen_enum_construction_uses_double_colon(enum_def in arb_typed_enum_def()) {
            // Pick the first variant and construct it
            let enum_name = enum_def.name.clone();
            let variant = enum_def.variants[0].clone();
            let variant_name = variant.name.clone();

            // Build argument expressions matching the variant's fields
            let args: Vec<TypedExpr> = variant
                .fields
                .iter()
                .map(|(_name, ty)| {
                    // Create a fixed literal (not random here, just demonstrating type match)
                    match ty {
                        FluxType::Int => TypedExpr {
                            kind: TypedExprKind::IntLiteral(42),
                            resolved_type: FluxType::Int,
                            span: Span::new(0, 1),
                        },
                        FluxType::Float => TypedExpr {
                            kind: TypedExprKind::FloatLiteral(3.14),
                            resolved_type: FluxType::Float,
                            span: Span::new(0, 1),
                        },
                        FluxType::String => TypedExpr {
                            kind: TypedExprKind::StringLiteral("test".to_string()),
                            resolved_type: FluxType::String,
                            span: Span::new(0, 1),
                        },
                        FluxType::Bool => TypedExpr {
                            kind: TypedExprKind::BoolLiteral(true),
                            resolved_type: FluxType::Bool,
                            span: Span::new(0, 1),
                        },
                        _ => TypedExpr {
                            kind: TypedExprKind::IntLiteral(0),
                            resolved_type: FluxType::Int,
                            span: Span::new(0, 1),
                        },
                    }
                })
                .collect();

            let program = build_enum_construction_program(
                enum_def,
                enum_name.clone(),
                variant_name.clone(),
                args,
            );

            let result = generate(&program);
            prop_assert!(result.is_ok(), "generate() failed: {:?}", result.err());
            let output = result.unwrap();

            // Must contain `EnumName::VariantName` (double colon separator)
            let expected_construction = format!("{}::{}", enum_name, variant_name);
            prop_assert!(
                output.contains(&expected_construction),
                "Output must contain '{}' (:: separator), got:\n{}",
                expected_construction, output
            );

            // Must NOT contain `EnumName.VariantName` (dot separator, which is Flux syntax)
            let dot_construction = format!("{}.{}", enum_name, variant_name);
            prop_assert!(
                !output.contains(&dot_construction),
                "Output must NOT contain '{}' (dot separator is Flux syntax, not Rust), got:\n{}",
                dot_construction, output
            );
        }
    }

    // ========================================================================
    // Generators for Property 15: Codegen Impl/Trait Output Validity (impl)
    // ========================================================================

    /// Generate a valid snake_case method name.
    fn arb_method_name() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{2,8}".prop_filter("must not be Rust keyword", |n| {
            !matches!(
                n.as_str(),
                "as" | "break" | "const" | "continue" | "crate" | "else" | "enum"
                    | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in"
                    | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub"
                    | "ref" | "return" | "self" | "static" | "struct" | "super"
                    | "trait" | "true" | "type" | "unsafe" | "use" | "where"
                    | "while" | "async" | "await" | "dyn"
            )
        })
    }

    /// Generate a return type for a method (subset of FluxType that maps cleanly).
    fn arb_method_return_type() -> impl Strategy<Value = FluxType> {
        prop_oneof![
            Just(FluxType::Int),
            Just(FluxType::Float),
            Just(FluxType::String),
            Just(FluxType::Bool),
            Just(FluxType::Void),
        ]
    }

    /// Generate a typed method definition for an impl block.
    /// `is_instance` controls whether the method has `self` as first param.
    fn arb_typed_method(is_instance: bool) -> impl Strategy<Value = TypedFnDef> {
        (arb_method_name(), arb_method_return_type()).prop_map(move |(name, ret_type)| {
            let span = Span::new(0, 1);

            // Build params and param_types
            let (params, param_types) = if is_instance {
                // Instance method: self + no additional params for simplicity
                (vec!["self".to_string()], vec![FluxType::Void]) // self type placeholder
            } else {
                // Static method: one parameter of type f64
                (
                    vec!["value".to_string()],
                    vec![FluxType::Float],
                )
            };

            // Build a simple body: return a literal matching the return type
            let body = match &ret_type {
                FluxType::Void => vec![],
                FluxType::Int => vec![TypedStmt::Return(TypedReturnStmt {
                    value: Some(TypedExpr {
                        kind: TypedExprKind::IntLiteral(42),
                        resolved_type: FluxType::Int,
                        span,
                    }),
                    span,
                })],
                FluxType::Float => vec![TypedStmt::Return(TypedReturnStmt {
                    value: Some(TypedExpr {
                        kind: TypedExprKind::FloatLiteral(3.14),
                        resolved_type: FluxType::Float,
                        span,
                    }),
                    span,
                })],
                FluxType::String => vec![TypedStmt::Return(TypedReturnStmt {
                    value: Some(TypedExpr {
                        kind: TypedExprKind::StringLiteral("hello".to_string()),
                        resolved_type: FluxType::String,
                        span,
                    }),
                    span,
                })],
                FluxType::Bool => vec![TypedStmt::Return(TypedReturnStmt {
                    value: Some(TypedExpr {
                        kind: TypedExprKind::BoolLiteral(true),
                        resolved_type: FluxType::Bool,
                        span,
                    }),
                    span,
                })],
                _ => vec![],
            };

            TypedFnDef {
                name,
                params,
                param_types,
                body,
                return_type: ret_type,
                span,
            }
        })
    }

    /// Generate a TypedImplBlock with 1-4 methods (mix of instance and static).
    fn arb_typed_impl_block() -> impl Strategy<Value = (TypedImplBlock, TypedStructDef)> {
        (
            arb_pascal_ident(),
            prop::collection::vec(
                prop::bool::ANY.prop_flat_map(|is_instance| arb_typed_method(is_instance)),
                1..=4,
            ),
        )
            .prop_map(|(struct_name, methods)| {
                // Deduplicate method names
                let mut seen = std::collections::HashSet::new();
                let deduped_methods: Vec<TypedFnDef> = methods
                    .into_iter()
                    .enumerate()
                    .map(|(i, mut m)| {
                        if seen.contains(&m.name) {
                            m.name = format!("{}_{}", m.name, i);
                        }
                        seen.insert(m.name.clone());
                        m
                    })
                    .collect();

                let impl_block = TypedImplBlock {
                    trait_name: None,
                    target_type: struct_name.clone(),
                    methods: deduped_methods,
                    span: Span::new(0, 1),
                };

                // Create a corresponding struct definition (needed for valid program)
                let struct_def = TypedStructDef {
                    name: struct_name,
                    fields: vec![TypedStructField {
                        name: "value".to_string(),
                        resolved_type: FluxType::Float,
                        bit_width: None,
                        field_decorator_names: vec![],
                        span: Span::new(0, 1),
                    }],
                    decorators: vec![],
                    span: Span::new(0, 1),
                };

                (impl_block, struct_def)
            })
    }

    /// Helper: build a minimal TypedProgram containing struct + impl block.
    fn build_impl_program(
        struct_def: TypedStructDef,
        impl_block: TypedImplBlock,
    ) -> TypedProgram {
        TypedProgram {
            imports: vec![],
            structs: vec![struct_def],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![impl_block],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "ImplTest".to_string(),
                body: vec![],
                span: Span::new(0, 10),
            },
            span: Span::new(0, 10),
        }
    }

    // ========================================================================
    // Property 15: Codegen Impl/Trait Output Validity (impl portion)
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-type-system, Property 15: Codegen Impl/Trait Output Validity
        /// **Validates: Requirements 4.11**
        ///
        /// For any valid typed impl block (with varying instance and static methods),
        /// the codegen stage SHALL emit a syntactically valid Rust `impl StructName { }`
        /// block where:
        /// - The output contains `impl StructName {`
        /// - Instance methods have `&self` parameter
        /// - Method names are present in the output
        /// - Non-void return types are annotated
        #[test]
        fn prop_codegen_impl_block_validity(
            (impl_block, struct_def) in arb_typed_impl_block()
        ) {
            let struct_name = impl_block.target_type.clone();
            let methods = impl_block.methods.clone();
            let program = build_impl_program(struct_def, impl_block);

            let result = generate(&program);
            prop_assert!(result.is_ok(), "generate() failed: {:?}", result.err());
            let output = result.unwrap();

            // 1. Must contain `impl StructName {`
            let impl_header = format!("impl {} {{", struct_name);
            prop_assert!(
                output.contains(&impl_header),
                "Output must contain '{}', got:\n{}", impl_header, output
            );

            // 2. Verify each method appears correctly
            for method in &methods {
                // Method name must appear in the output
                let fn_decl = format!("fn {}(", method.name);
                prop_assert!(
                    output.contains(&fn_decl),
                    "Output must contain method declaration '{}', got:\n{}",
                    fn_decl, output
                );

                // Instance methods (first param is "self") must have `&self`
                let is_instance = method.params.first().map(|p| p == "self").unwrap_or(false);
                if is_instance {
                    // Find the method signature line and check for &self
                    let method_sig = format!("fn {}(&self", method.name);
                    prop_assert!(
                        output.contains(&method_sig),
                        "Instance method '{}' must have '&self' parameter, got:\n{}",
                        method.name, output
                    );
                }

                // Non-void return types must be annotated
                match &method.return_type {
                    FluxType::Void | FluxType::Null => {}
                    ret_type => {
                        let rust_type = match ret_type {
                            FluxType::Int => "i64",
                            FluxType::Float => "f64",
                            FluxType::String => "String",
                            FluxType::Bool => "bool",
                            _ => continue,
                        };
                        // The method should contain `-> Type`
                        let return_annotation = format!("-> {}", rust_type);
                        prop_assert!(
                            output.contains(&return_annotation),
                            "Method '{}' with return type {:?} must have '{}' annotation, got:\n{}",
                            method.name, ret_type, return_annotation, output
                        );
                    }
                }
            }
        }
    }
}
