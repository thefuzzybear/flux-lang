#![allow(dead_code)]
//! Main type-checking logic for the Flux language.
//!
//! The `TypeChecker` struct walks the untyped AST and produces a `TypedProgram`
//! with resolved types on every expression node. It maintains a scoped
//! environment for identifier resolution and enforces all type rules.

use std::collections::{HashMap, HashSet};

use crate::error::{CompileError, Result};
use crate::lexer::Span;
use crate::parser::ast::*;

use super::builtins;
use super::enum_info::{EnumInfo, MethodInfo, TraitInfo, TraitMethodInfo, VariantInfo};
use super::env::TypeEnvironment;
use super::typed_ast::*;
use super::types::{FluxType, FnParams};

/// Information about a registered struct definition, stored in the struct registry.
#[derive(Debug, Clone)]
pub(crate) struct StructDefInfo {
    pub name: String,
    /// Type parameters for generic structs (e.g., ["T"] for `struct Vec[T]`).
    /// Empty for non-generic structs.
    pub type_params: Vec<String>,
    pub fields: Vec<(String, FluxType)>,
    pub decorators: Vec<ValidatedDecorator>,
}

/// The core type checker. Walks the untyped AST, resolves identifiers,
/// validates type compatibility, and produces an annotated typed AST.
pub(crate) struct TypeChecker {
    env: TypeEnvironment,
    in_event_handler: bool,
    in_function_body: bool,
    state_var_names: HashSet<String>,
    /// Registry of struct definitions, populated in dependency order.
    struct_registry: HashMap<String, StructDefInfo>,
    /// The declared return type of the function currently being checked (if any).
    current_fn_return_type: Option<FluxType>,
    /// Type parameters currently in scope (from generic struct or function being checked).
    current_type_params: HashSet<String>,
    /// Trait bounds on function type parameters: fn_name → [(type_param_name, trait_name)]
    fn_trait_bounds: HashMap<String, Vec<(String, String)>>,
    /// Warnings collected during type checking (non-fatal diagnostics).
    pub(crate) warnings: Vec<String>,
}

impl TypeChecker {
    /// Create a new TypeChecker with an empty global scope.
    pub fn new() -> Self {
        Self {
            env: TypeEnvironment::new(),
            in_event_handler: false,
            in_function_body: false,
            state_var_names: HashSet::new(),
            struct_registry: HashMap::new(),
            current_fn_return_type: None,
            current_type_params: HashSet::new(),
            fn_trait_bounds: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    /// Type-check an entire program, producing a TypedProgram.
    pub fn check_program(&mut self, program: Program) -> Result<TypedProgram> {
        // Register imports into global scope
        self.register_imports(&program.imports)?;

        // Register struct definitions (validates fields, resolves types, topological order)
        let typed_structs = self.register_structs(&program.structs)?;

        // Register enum definitions (validates variants, checks for duplicates)
        let typed_enums = self.register_enums(&program.enums)?;

        // Register user-defined functions into global scope (before strategy checking)
        self.register_functions(&program.functions)?;

        // Register impl blocks (validates target type, checks method bodies, detects duplicates)
        let typed_impl_blocks = self.register_impl_blocks(&program.impl_blocks)?;

        // Register trait definitions (validates names, checks for duplicates)
        let typed_traits = self.register_traits(&program.traits)?;

        // Validate trait impls (checks method completeness and signature matching)
        let typed_trait_impl_blocks = self.register_trait_impls(&program.impl_blocks)?;

        // Detect recursion via call graph analysis (before body checking)
        let call_graph = super::call_graph::build_call_graph(&program.functions);
        if let Some(cycle) = super::call_graph::detect_cycles(&call_graph) {
            return Err(self.recursion_error(&program.functions, &cycle));
        }

        // Pre-scan strategy state block to collect state variable names
        // (used for producing specific errors when functions try to access state)
        self.collect_state_var_names(&program.strategy);

        // Check function bodies (after registration so they can call each other)
        let typed_functions = self.check_fn_defs(program.functions)?;

        // Validate data block before strategy checking
        let typed_data_block = match program.data_block {
            Some(ref db) => Some(self.check_data_block(db)?),
            None => None,
        };

        // Validate connector block if present
        let typed_connector_block = match program.connector_block {
            Some(ref cb) => Some(self.check_connector_block(cb)?),
            None => None,
        };

        // Check strategy
        let typed_strategy = self.check_strategy(program.strategy)?;

        Ok(TypedProgram {
            imports: program.imports,
            structs: typed_structs,
            enums: typed_enums,
            functions: typed_functions,
            impl_blocks: {
                let mut all_impl_blocks = typed_impl_blocks;
                all_impl_blocks.extend(typed_trait_impl_blocks);
                all_impl_blocks
            },
            traits: typed_traits,
            data_block: typed_data_block,
            connector_block: typed_connector_block,
            strategy: typed_strategy,
            span: program.span,
        })
    }

    /// Validate a data block's field values, producing a TypedDataBlock.
    ///
    /// Checks:
    /// - symbols list is non-empty and contains no empty strings
    /// - period is in the valid set
    /// - interval is in the valid set
    /// - source is a known provider
    fn check_data_block(&self, data_block: &DataBlock) -> Result<TypedDataBlock> {
        // Validate symbols list
        if let Some(ref symbols_field) = data_block.symbols {
            if symbols_field.value.is_empty() {
                return Err(CompileError::Type(format!(
                    "at byte {}: data block 'symbols' must contain at least one symbol",
                    symbols_field.span.start
                )));
            }
            for (i, sym) in symbols_field.value.iter().enumerate() {
                if sym.is_empty() {
                    return Err(CompileError::Type(format!(
                        "at byte {}: symbol at index {} must be non-empty",
                        symbols_field.span.start, i
                    )));
                }
            }
        }

        // Validate period
        let valid_periods = ["1d", "5d", "1mo", "3mo", "6mo", "1y", "2y", "5y", "max"];
        if let Some(ref period_field) = data_block.period {
            if !valid_periods.contains(&period_field.value.as_str()) {
                return Err(CompileError::Type(format!(
                    "at byte {}: invalid period '{}'. Valid options: {}",
                    period_field.span.start,
                    period_field.value,
                    valid_periods.join(", ")
                )));
            }
        }

        // Validate interval
        let valid_intervals = ["1m", "5m", "15m", "1h", "1d", "1wk", "1mo"];
        if let Some(ref interval_field) = data_block.interval {
            if !valid_intervals.contains(&interval_field.value.as_str()) {
                return Err(CompileError::Type(format!(
                    "at byte {}: invalid interval '{}'. Valid options: {}",
                    interval_field.span.start,
                    interval_field.value,
                    valid_intervals.join(", ")
                )));
            }
        }

        // Validate source
        let valid_sources = ["yahoo"];
        if let Some(ref source_field) = data_block.source {
            if !valid_sources.contains(&source_field.value.as_str()) {
                return Err(CompileError::Type(format!(
                    "at byte {}: unknown data source '{}'. Available: {}",
                    source_field.span.start,
                    source_field.value,
                    valid_sources.join(", ")
                )));
            }
        }

        Ok(TypedDataBlock {
            symbols: data_block.symbols.as_ref().map(|f| f.value.clone()),
            period: data_block.period.as_ref().map(|f| f.value.clone()),
            interval: data_block.interval.as_ref().map(|f| f.value.clone()),
            source: data_block.source.as_ref().map(|f| f.value.clone()),
            span: data_block.span,
        })
    }

    /// Validate a connector block's field values, producing a TypedConnectorBlock.
    ///
    /// Checks:
    /// - `type` (if present) is one of "websocket", "poll", "replay"
    /// - `url` is required when type is "websocket" or "poll"
    /// - `file` is required when type is "replay"
    /// - `symbols` (if present) is a non-empty list of non-empty strings
    fn check_connector_block(
        &self,
        connector_block: &ConnectorBlock,
    ) -> Result<TypedConnectorBlock> {
        let valid_types = ["websocket", "poll", "replay"];

        // Validate connector type if present
        if let Some(ref type_field) = connector_block.connector_type {
            if !valid_types.contains(&type_field.value.as_str()) {
                return Err(CompileError::Type(format!(
                    "at byte {}: invalid connector type '{}'. Valid options: {}",
                    type_field.span.start,
                    type_field.value,
                    valid_types.join(", ")
                )));
            }

            // If type is "websocket" or "poll", url must be present
            if (type_field.value == "websocket" || type_field.value == "poll")
                && connector_block.url.is_none()
            {
                return Err(CompileError::Type(format!(
                    "at byte {}: connector type '{}' requires a 'url' field",
                    type_field.span.start,
                    type_field.value,
                )));
            }

            // If type is "replay", file must be present
            if type_field.value == "replay" && connector_block.file.is_none() {
                return Err(CompileError::Type(format!(
                    "at byte {}: connector type 'replay' requires a 'file' field",
                    type_field.span.start,
                )));
            }
        }

        // Validate symbols list if present
        if let Some(ref symbols_field) = connector_block.symbols {
            if symbols_field.value.is_empty() {
                return Err(CompileError::Type(format!(
                    "at byte {}: connector 'symbols' must contain at least one symbol",
                    symbols_field.span.start
                )));
            }
            for (i, sym) in symbols_field.value.iter().enumerate() {
                if sym.is_empty() {
                    return Err(CompileError::Type(format!(
                        "at byte {}: connector symbol at index {} must be non-empty",
                        symbols_field.span.start, i
                    )));
                }
            }
        }

        Ok(TypedConnectorBlock {
            connector_type: connector_block
                .connector_type
                .as_ref()
                .map(|f| f.value.clone()),
            url: connector_block.url.as_ref().map(|f| f.value.clone()),
            symbols: connector_block.symbols.as_ref().map(|f| f.value.clone()),
            interval: connector_block.interval.as_ref().map(|f| f.value.clone()),
            file: connector_block.file.as_ref().map(|f| f.value.clone()),
            span: connector_block.span,
        })
    }

    fn register_imports(&mut self, imports: &[Import]) -> Result<()> {
        for import in imports {
            for name in &import.names {
                if self.env.resolve(name).is_some() {
                    return Err(self.type_error(
                        import.span,
                        format!("duplicate import: '{}'", name),
                    ));
                }
                self.env.insert(
                    name.clone(),
                    FluxType::Fn {
                        params: FnParams::VariadicNumeric,
                        ret: Box::new(FluxType::Float),
                    },
                );
            }
        }
        Ok(())
    }

    /// Suggest an import path for a struct type name that is not in scope.
    ///
    /// Returns `Some(path)` if the name matches a known stdlib struct, `None` otherwise.
    fn suggest_import_for_type(name: &str) -> Option<&'static str> {
        const STDLIB_STRUCT_IMPORTS: &[(&str, &[&str])] = &[
            ("market::l1", &["Tick", "Bar", "Quote", "MarketSnapshot"]),
            ("market::l2", &["Level", "Book"]),
            ("collections::buffers", &["QuoteWindow", "BarWindow"]),
        ];
        for &(module_path, struct_names) in STDLIB_STRUCT_IMPORTS {
            if struct_names.contains(&name) {
                return Some(module_path);
            }
        }
        None
    }

    /// Register struct definitions in the struct registry.
    ///
    /// Structs are registered in topological order by field-type dependencies:
    /// if struct A has a field of type B, B must be registered before A.
    /// Reports errors for:
    /// - Duplicate field names within a struct
    /// - Field types referencing undefined struct names
    ///
    /// Returns typed struct definitions in dependency-sorted order for inclusion
    /// in the TypedProgram (used by codegen for ordered emission).
    fn register_structs(&mut self, structs: &[StructDef]) -> Result<Vec<TypedStructDef>> {
        // Topologically sort structs by field-type dependencies
        let sorted = self.topological_sort_structs(structs)?;

        let mut typed_structs = Vec::with_capacity(sorted.len());
        for idx in sorted {
            let struct_def = &structs[idx];
            self.register_single_struct(struct_def)?;

            // Build the typed struct def from the now-registered info
            let info = &self.struct_registry[&struct_def.name];
            let is_bitfield = info.decorators.iter().any(|d| d.kind == DecoratorKind::Bitfield);
            let typed_fields = info
                .fields
                .iter()
                .zip(struct_def.fields.iter())
                .map(|((name, resolved_type), field)| {
                    // Compute bit_width for @bitfield structs from the source TypeAnnotation
                    let bit_width = if is_bitfield {
                        match &field.field_type {
                            TypeAnnotation::Bool => Some(1),
                            TypeAnnotation::BitInt(n) => Some(*n),
                            TypeAnnotation::Int => Some(64),
                            TypeAnnotation::F64 => Some(64),
                            _ => Some(64),
                        }
                    } else {
                        None
                    };
                    TypedStructField {
                        name: name.clone(),
                        resolved_type: resolved_type.clone(),
                        bit_width,
                        field_decorator_names: field
                            .field_decorators
                            .iter()
                            .map(|d| d.name.clone())
                            .collect(),
                        span: field.span,
                    }
                })
                .collect();

            typed_structs.push(TypedStructDef {
                name: struct_def.name.clone(),
                type_params: info.type_params.clone(),
                fields: typed_fields,
                decorators: info.decorators.clone(),
                span: struct_def.span,
            });
        }
        Ok(typed_structs)
    }

    /// Topologically sort struct definitions by field-type dependencies.
    ///
    /// Returns indices into the original slice in dependency order.
    /// If struct A contains a field of type B, B's index will appear before A's.
    fn topological_sort_structs(&self, structs: &[StructDef]) -> Result<Vec<usize>> {
        // Build a name → index map
        let name_to_idx: HashMap<&str, usize> = structs
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i))
            .collect();

        // Build adjacency list: edges[i] contains the indices that i depends on
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); structs.len()];
        for (i, struct_def) in structs.iter().enumerate() {
            for field in &struct_def.fields {
                for dep_name in Self::field_type_struct_refs(&field.field_type) {
                    if let Some(&dep_idx) = name_to_idx.get(dep_name.as_str()) {
                        deps[i].push(dep_idx);
                    }
                }
            }
        }

        // Kahn's algorithm for topological sort
        let n = structs.len();

        // Build reverse adjacency: rev_adj[dep] = list of nodes that depend on dep
        let mut rev_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, node_deps) in deps.iter().enumerate() {
            for &dep in node_deps {
                rev_adj[dep].push(i);
            }
        }

        // in_degree[i] = number of structs that i depends on
        let mut in_degree: Vec<usize> = deps.iter().map(|d| d.len()).collect();

        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for (i, &deg) in in_degree.iter().enumerate() {
            if deg == 0 {
                queue.push_back(i);
            }
        }

        let mut order = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            // For every struct that depends on `node`, decrement its in-degree
            for &dependent in &rev_adj[node] {
                in_degree[dependent] -= 1;
                if in_degree[dependent] == 0 {
                    queue.push_back(dependent);
                }
            }
        }

        if order.len() != n {
            // Cycle detected — find a struct involved in the cycle for the error
            // Pick the first struct not in the order
            let in_cycle = (0..n).find(|i| !order.contains(i)).unwrap_or(0);
            return Err(self.type_error(
                structs[in_cycle].span,
                format!(
                    "circular dependency detected involving struct '{}'",
                    structs[in_cycle].name
                ),
            ));
        }

        Ok(order)
    }

    /// Extract struct name references from a type annotation.
    fn field_type_struct_refs(ty: &TypeAnnotation) -> Vec<String> {
        match ty {
            TypeAnnotation::Named(name) => vec![name.clone()],
            TypeAnnotation::FixedArray(elem, _) => Self::field_type_struct_refs(elem),
            _ => vec![],
        }
    }

    /// Register a single struct definition: validate fields, resolve types, insert into registry.
    fn register_single_struct(&mut self, struct_def: &StructDef) -> Result<()> {
        let mut seen_fields: HashSet<String> = HashSet::new();
        let mut resolved_fields: Vec<(String, FluxType)> = Vec::new();

        // Set type parameters in scope for resolving fields of generic structs
        let prev_type_params = std::mem::take(&mut self.current_type_params);
        for tp in &struct_def.type_params {
            self.current_type_params.insert(tp.name.clone());
        }

        for field in &struct_def.fields {
            // Check for duplicate field names
            if !seen_fields.insert(field.name.clone()) {
                return Err(self.type_error(
                    field.span,
                    format!(
                        "duplicate field '{}' in struct '{}'",
                        field.name, struct_def.name
                    ),
                ));
            }

            // Validate field-level decorators (@hot/@cold)
            for dec in &field.field_decorators {
                match dec.name.as_str() {
                    "hot" | "cold" => {} // recognized field decorators
                    _ => {
                        self.warnings.push(format!(
                            "at byte {}: unknown field decorator '@{}' (ignored)",
                            dec.span.start, dec.name
                        ));
                    }
                }
            }

            // Resolve the field type
            let resolved_type = self.resolve_type_annotation(
                &field.field_type,
                &struct_def.name,
                &field.name,
                field.span,
            )?;
            resolved_fields.push((field.name.clone(), resolved_type));
        }

        // Insert into the struct registry
        let validated_decorators = self.validate_decorators(&struct_def.decorators);
        let type_params: Vec<String> = struct_def
            .type_params
            .iter()
            .map(|tp| tp.name.clone())
            .collect();
        self.struct_registry.insert(
            struct_def.name.clone(),
            StructDefInfo {
                name: struct_def.name.clone(),
                type_params,
                fields: resolved_fields,
                decorators: validated_decorators,
            },
        );

        // --- Post-registration validations ---

        // Validate @stack / @heap constraint: a @stack struct (or implicitly @stack,
        // i.e. no allocation decorator) cannot contain fields whose type is a @heap struct.
        let info = &self.struct_registry[&struct_def.name];
        let has_heap_decorator = info.decorators.iter().any(|d| d.kind == DecoratorKind::Heap);
        let has_stack_decorator = info.decorators.iter().any(|d| d.kind == DecoratorKind::Stack);
        let is_stack = !has_heap_decorator; // implicitly or explicitly @stack

        if is_stack || has_stack_decorator {
            // Check each field's type: if it's a struct marked @heap, report error
            for field in &struct_def.fields {
                let field_struct_names = Self::field_type_struct_refs(&field.field_type);
                for ref_name in field_struct_names {
                    if let Some(ref_info) = self.struct_registry.get(&ref_name) {
                        let ref_is_heap = ref_info.decorators.iter().any(|d| d.kind == DecoratorKind::Heap);
                        if ref_is_heap {
                            return Err(self.type_error(
                                field.span,
                                format!(
                                    "@stack struct '{}' cannot contain @heap-allocated field '{}'",
                                    struct_def.name, field.name
                                ),
                            ));
                        }
                    }
                }
            }
        }

        // Validate @aligned(N): N must be a power of 2 in [1, 4096]
        let info = &self.struct_registry[&struct_def.name];
        for dec in &info.decorators.clone() {
            if let DecoratorKind::Aligned(n) = dec.kind {
                if n == 0 || !n.is_power_of_two() || n > 4096 {
                    return Err(self.type_error(
                        dec.span,
                        format!(
                            "@aligned argument must be power of 2 between 1 and 4096, got {}",
                            n
                        ),
                    ));
                }
            }
        }

        // Validate @simd(N): N must be 128, 256, or 512
        let info = &self.struct_registry[&struct_def.name];
        for dec in &info.decorators.clone() {
            if let DecoratorKind::Simd(n) = dec.kind {
                if n != 128 && n != 256 && n != 512 {
                    return Err(self.type_error(
                        dec.span,
                        format!(
                            "@simd width must be 128, 256, or 512, got {}",
                            n
                        ),
                    ));
                }
            }
        }

        // Validate @pool(N): N must be a positive integer
        let info = &self.struct_registry[&struct_def.name];
        for dec in &info.decorators.clone() {
            if let DecoratorKind::Pool(n) = dec.kind {
                if n == 0 {
                    return Err(self.type_error(
                        dec.span,
                        "@pool size must be a positive integer".to_string(),
                    ));
                }
            }
        }

        // Validate @soa: all fields must be scalar (f64, int, bool)
        let info = &self.struct_registry[&struct_def.name];
        let has_soa = info.decorators.iter().any(|d| d.kind == DecoratorKind::Soa);
        if has_soa {
            let info_fields = info.fields.clone();
            let soa_span = info.decorators.iter().find(|d| d.kind == DecoratorKind::Soa).unwrap().span;
            for (field_name, field_type) in &info_fields {
                match field_type {
                    FluxType::Float | FluxType::Int | FluxType::Bool => {}
                    _ => {
                        return Err(self.type_error(
                            soa_span,
                            format!(
                                "@soa struct '{}' field '{}' must be scalar (f64, int, or bool), got {}",
                                struct_def.name, field_name, field_type
                            ),
                        ));
                    }
                }
            }
        }

        // Validate @bitfield: total bit count must not exceed 64
        let info = &self.struct_registry[&struct_def.name];
        let has_bitfield = info.decorators.iter().any(|d| d.kind == DecoratorKind::Bitfield);
        if has_bitfield {
            let bitfield_span = info.decorators.iter().find(|d| d.kind == DecoratorKind::Bitfield).unwrap().span;
            let mut total_bits: usize = 0;
            for field in &struct_def.fields {
                match &field.field_type {
                    TypeAnnotation::Bool => total_bits += 1,
                    TypeAnnotation::BitInt(n) => total_bits += n,
                    TypeAnnotation::Int => total_bits += 64, // full int width
                    TypeAnnotation::F64 => total_bits += 64,
                    _ => total_bits += 64, // conservative
                }
            }
            if total_bits > 64 {
                return Err(self.type_error(
                    bitfield_span,
                    format!(
                        "@bitfield struct total is {} bits, maximum is 64",
                        total_bits
                    ),
                ));
            }
        }

        // Validate decorator compatibility matrix
        self.validate_decorator_compatibility(struct_def)?;

        // Restore previous type params scope
        self.current_type_params = prev_type_params;

        Ok(())
    }

    /// Validate that decorator combinations on a struct are compatible.
    /// Rejects incompatible pairs and emits warnings for contradictory but non-fatal pairs.
    fn validate_decorator_compatibility(&mut self, struct_def: &StructDef) -> Result<()> {
        let info = &self.struct_registry[&struct_def.name];
        let decorators = info.decorators.clone();

        // Helper: check if a specific kind is present
        let has_kind = |kind: &DecoratorKind| -> bool {
            decorators.iter().any(|d| &d.kind == kind)
        };

        let find_span = |kind: &DecoratorKind| -> Span {
            decorators.iter().find(|d| &d.kind == kind).map(|d| d.span).unwrap_or(struct_def.span)
        };

        // Check if aligned is present (any value)
        let has_aligned = decorators.iter().any(|d| matches!(d.kind, DecoratorKind::Aligned(_)));
        let has_packed = has_kind(&DecoratorKind::Packed);
        let has_soa = has_kind(&DecoratorKind::Soa);
        let has_stack = has_kind(&DecoratorKind::Stack);
        let has_heap = has_kind(&DecoratorKind::Heap);
        let has_pool = decorators.iter().any(|d| matches!(d.kind, DecoratorKind::Pool(_)));
        let has_bitfield = has_kind(&DecoratorKind::Bitfield);
        let has_immutable = has_kind(&DecoratorKind::Immutable);
        let has_volatile = has_kind(&DecoratorKind::Volatile);

        // Incompatible pairs: @packed + @aligned
        if has_packed && has_aligned {
            let span = find_span(&DecoratorKind::Packed);
            return Err(self.type_error(
                span,
                "decorators @packed and @aligned cannot be combined on the same struct".to_string(),
            ));
        }

        // Incompatible: @soa + @packed
        if has_soa && has_packed {
            let span = find_span(&DecoratorKind::Soa);
            return Err(self.type_error(
                span,
                "decorators @soa and @packed cannot be combined on the same struct".to_string(),
            ));
        }

        // Incompatible: @stack + @heap
        if has_stack && has_heap {
            let span = find_span(&DecoratorKind::Stack);
            return Err(self.type_error(
                span,
                "decorators @stack and @heap cannot be combined on the same struct".to_string(),
            ));
        }

        // Incompatible: @pool + @heap
        if has_pool && has_heap {
            let span = decorators.iter().find(|d| matches!(d.kind, DecoratorKind::Pool(_))).unwrap().span;
            return Err(self.type_error(
                span,
                "decorators @pool and @heap cannot be combined on the same struct".to_string(),
            ));
        }

        // Incompatible: @pool + @stack
        if has_pool && has_stack {
            let span = decorators.iter().find(|d| matches!(d.kind, DecoratorKind::Pool(_))).unwrap().span;
            return Err(self.type_error(
                span,
                "decorators @pool and @stack cannot be combined on the same struct".to_string(),
            ));
        }

        // Incompatible: @bitfield + @soa
        if has_bitfield && has_soa {
            let span = find_span(&DecoratorKind::Bitfield);
            return Err(self.type_error(
                span,
                "decorators @bitfield and @soa cannot be combined on the same struct".to_string(),
            ));
        }

        // Incompatible: @immutable + @volatile
        if has_immutable && has_volatile {
            let span = find_span(&DecoratorKind::Immutable);
            return Err(self.type_error(
                span,
                "decorators @immutable and @volatile cannot be combined on the same struct".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate a list of parsed decorators, converting recognized names into
    /// `ValidatedDecorator` values. Unrecognized decorator names emit a warning
    /// (not an error) and are skipped, allowing forward-compatible usage.
    fn validate_decorators(&mut self, decorators: &[Decorator]) -> Vec<ValidatedDecorator> {
        let mut validated = Vec::new();

        for decorator in decorators {
            let kind = match decorator.name.as_str() {
                "stack" => Some(DecoratorKind::Stack),
                "heap" => Some(DecoratorKind::Heap),
                "aligned" => {
                    let arg = decorator
                        .arg
                        .as_ref()
                        .and_then(|a| match a {
                            DecoratorArg::Int(n) => Some(*n as u32),
                        })
                        .unwrap_or(64); // default alignment
                    Some(DecoratorKind::Aligned(arg))
                }
                "packed" => Some(DecoratorKind::Packed),
                "prefetch" => Some(DecoratorKind::Prefetch),
                "streaming" => Some(DecoratorKind::Streaming),
                "soa" => Some(DecoratorKind::Soa),
                "pool" => {
                    let arg = decorator
                        .arg
                        .as_ref()
                        .and_then(|a| match a {
                            DecoratorArg::Int(n) => Some(*n as u32),
                        })
                        .unwrap_or(64); // default pool size
                    Some(DecoratorKind::Pool(arg))
                }
                "hot" => Some(DecoratorKind::Hot),
                "cold" => Some(DecoratorKind::Cold),
                "volatile" => Some(DecoratorKind::Volatile),
                "bitfield" => Some(DecoratorKind::Bitfield),
                "simd" => {
                    let arg = decorator
                        .arg
                        .as_ref()
                        .and_then(|a| match a {
                            DecoratorArg::Int(n) => Some(*n as u32),
                        })
                        .unwrap_or(256); // default SIMD width
                    Some(DecoratorKind::Simd(arg))
                }
                "zero_init" => Some(DecoratorKind::ZeroInit),
                "immutable" => Some(DecoratorKind::Immutable),
                _ => {
                    // Unknown decorator: emit a warning (not an error)
                    self.warnings.push(format!(
                        "at byte {}: unknown decorator '@{}' (ignored)",
                        decorator.span.start, decorator.name
                    ));
                    None
                }
            };

            if let Some(kind) = kind {
                validated.push(ValidatedDecorator {
                    kind,
                    span: decorator.span,
                });
            }
        }

        validated
    }

    /// Resolve a TypeAnnotation to a FluxType, checking that Named references
    /// exist in the struct registry.
    fn resolve_type_annotation(
        &self,
        annotation: &TypeAnnotation,
        struct_name: &str,
        field_name: &str,
        span: Span,
    ) -> Result<FluxType> {
        match annotation {
            TypeAnnotation::F64 => Ok(FluxType::Float),
            TypeAnnotation::Int => Ok(FluxType::Int),
            TypeAnnotation::Bool => Ok(FluxType::Bool),
            TypeAnnotation::Str => Ok(FluxType::String),
            TypeAnnotation::Named(name) => {
                // Check if it's a type parameter in scope (e.g., T in a generic struct/fn)
                if self.current_type_params.contains(name) {
                    return Ok(FluxType::TypeParam(name.clone()));
                }
                if self.struct_registry.contains_key(name) {
                    Ok(FluxType::Struct(name.clone()))
                } else {
                    let msg = if let Some(import_path) = Self::suggest_import_for_type(name) {
                        format!(
                            "type '{}' is not defined. Did you mean 'from {} import {{{}}}'?",
                            name, import_path, name
                        )
                    } else {
                        format!(
                            "unknown type '{}' in struct '{}' field '{}'",
                            name, struct_name, field_name
                        )
                    };
                    Err(self.type_error(span, msg))
                }
            }
            TypeAnnotation::FixedArray(elem_type, size) => {
                if *size == 0 {
                    return Err(self.type_error(
                        span,
                        "array size must be positive, got 0".to_string(),
                    ));
                }
                let resolved_elem = self.resolve_type_annotation(
                    elem_type,
                    struct_name,
                    field_name,
                    span,
                )?;
                Ok(FluxType::FixedArray(Box::new(resolved_elem), *size))
            }
            TypeAnnotation::BitInt(_) => {
                // BitInt is used in @bitfield structs, resolve as Int for now
                Ok(FluxType::Int)
            }
            TypeAnnotation::Generic(name, type_args) => {
                // Resolve generic types: built-in (Vec, HashMap) or user-defined generic structs
                // Validate type argument count matches type parameter count
                let resolved_args: Result<Vec<_>> = type_args
                    .iter()
                    .map(|arg| self.resolve_type_annotation(arg, struct_name, field_name, span))
                    .collect();
                let resolved_args = resolved_args?;

                // Check if it's a known generic struct with type params
                if let Some(info) = self.struct_registry.get(name) {
                    let expected = info.type_params.len();
                    if expected > 0 && resolved_args.len() != expected {
                        return Err(self.type_error(
                            span,
                            format!(
                                "expected {} type arguments for '{}', got {}",
                                expected, name, resolved_args.len()
                            ),
                        ));
                    }
                }

                Ok(FluxType::Generic(name.clone(), resolved_args))
            }
        }
    }

    /// Resolve a type annotation in a function context (parameter or return type).
    ///
    /// Unlike `resolve_type_annotation` which formats errors for struct fields,
    /// this variant produces errors appropriate for function signatures.
    fn resolve_fn_type_annotation(
        &self,
        annotation: &TypeAnnotation,
        fn_name: &str,
        context: &str,
        span: Span,
    ) -> Result<FluxType> {
        match annotation {
            TypeAnnotation::F64 => Ok(FluxType::Float),
            TypeAnnotation::Int => Ok(FluxType::Int),
            TypeAnnotation::Bool => Ok(FluxType::Bool),
            TypeAnnotation::Str => Ok(FluxType::String),
            TypeAnnotation::Named(name) => {
                // Check if it's a type parameter in scope (e.g., T in a generic function)
                if self.current_type_params.contains(name) {
                    return Ok(FluxType::TypeParam(name.clone()));
                }
                if self.struct_registry.contains_key(name) {
                    Ok(FluxType::Struct(name.clone()))
                } else {
                    let msg = if let Some(import_path) = Self::suggest_import_for_type(name) {
                        format!(
                            "type '{}' is not defined. Did you mean 'from {} import {{{}}}'?",
                            name, import_path, name
                        )
                    } else {
                        format!(
                            "unknown type '{}' in function '{}' {}",
                            name, fn_name, context
                        )
                    };
                    Err(self.type_error(span, msg))
                }
            }
            TypeAnnotation::FixedArray(elem_type, size) => {
                if *size == 0 {
                    return Err(self.type_error(
                        span,
                        "array size must be positive, got 0".to_string(),
                    ));
                }
                let resolved_elem = self.resolve_fn_type_annotation(
                    elem_type,
                    fn_name,
                    context,
                    span,
                )?;
                Ok(FluxType::FixedArray(Box::new(resolved_elem), *size))
            }
            TypeAnnotation::BitInt(_) => {
                Ok(FluxType::Int)
            }
            TypeAnnotation::Generic(name, type_args) => {
                // Resolve generic types: built-in (Vec, HashMap) or user-defined generic structs
                // Validate type argument count matches type parameter count
                let resolved_args: Result<Vec<_>> = type_args
                    .iter()
                    .map(|arg| self.resolve_fn_type_annotation(arg, fn_name, context, span))
                    .collect();
                let resolved_args = resolved_args?;

                // Check if it's a known generic struct with type params
                if let Some(info) = self.struct_registry.get(name) {
                    let expected = info.type_params.len();
                    if expected > 0 && resolved_args.len() != expected {
                        return Err(self.type_error(
                            span,
                            format!(
                                "expected {} type arguments for '{}', got {}",
                                expected, name, resolved_args.len()
                            ),
                        ));
                    }
                }

                Ok(FluxType::Generic(name.clone(), resolved_args))
            }
        }
    }

    /// Register user-defined functions in the type environment.
    ///
    /// Each function is registered as `FluxType::Fn { params: Fixed(vec![Float; n]), ret: Float }`.
    /// Detects duplicate function definitions (including collisions with imports).
    fn register_functions(&mut self, functions: &[FnDef]) -> Result<()> {
        for fn_def in functions {
            // Check for duplicate names (collisions with imports or other functions)
            if self.env.resolve(&fn_def.name).is_some() {
                return Err(self.type_error(
                    fn_def.span,
                    format!("duplicate function definition '{}'", fn_def.name),
                ));
            }

            // Set type parameters in scope for resolving generic function signatures
            let prev_type_params = std::mem::take(&mut self.current_type_params);
            for tp in &fn_def.type_params {
                self.current_type_params.insert(tp.name.clone());
            }
            #[cfg(test)]
            eprintln!(
                "[register_functions] fn '{}' type_params={:?}, current_type_params={:?}",
                fn_def.name,
                fn_def.type_params.iter().map(|tp| &tp.name).collect::<Vec<_>>(),
                self.current_type_params
            );

            // Collect trait bounds for this function's type parameters
            let bounds: Vec<(String, String)> = fn_def
                .type_params
                .iter()
                .filter_map(|tp| {
                    tp.bound
                        .as_ref()
                        .map(|b| (tp.name.clone(), b.clone()))
                })
                .collect();
            if !bounds.is_empty() {
                self.fn_trait_bounds.insert(fn_def.name.clone(), bounds);
            }

            // Resolve parameter types: use annotation if present, default to Float
            let mut param_types = Vec::new();
            for param in &fn_def.params {
                let param_ty = if let Some(ref annotation) = param.param_type {
                    self.resolve_fn_type_annotation(
                        annotation,
                        &fn_def.name,
                        &format!("parameter '{}'", param.name),
                        param.span,
                    )?
                } else {
                    FluxType::Float
                };
                param_types.push(param_ty);
            }

            // Resolve return type: use annotation if present, default to Float
            let ret_type = if let Some(ref annotation) = fn_def.return_type {
                self.resolve_fn_type_annotation(
                    annotation,
                    &fn_def.name,
                    "return type",
                    fn_def.span,
                )?
            } else {
                FluxType::Float
            };

            // Restore previous type params scope
            self.current_type_params = prev_type_params;

            self.env.insert(
                fn_def.name.clone(),
                FluxType::Fn {
                    params: FnParams::Fixed(param_types),
                    ret: Box::new(ret_type),
                },
            );
        }
        Ok(())
    }

    /// Register enum definitions in the type environment.
    ///
    /// For each enum definition, this method:
    /// - Validates that the enum name doesn't conflict with existing types (structs or other enums)
    /// - Detects duplicate variant names within an enum
    /// - Resolves field types for data variants
    /// - Registers the enum in the type environment
    ///
    /// Requirements: 1.4, 1.5, 1.6
    fn register_enums(&mut self, enums: &[EnumDef]) -> Result<Vec<TypedEnumDef>> {
        let mut typed_enums = Vec::new();

        for enum_def in enums {
            // Check for enum name conflict with existing struct or enum
            if self.struct_registry.contains_key(&enum_def.name) {
                return Err(self.type_error(
                    enum_def.span,
                    format!("type name '{}' is already defined as a struct", enum_def.name),
                ));
            }
            if self.env.has_enum(&enum_def.name) {
                return Err(self.type_error(
                    enum_def.span,
                    format!("enum '{}' is already defined", enum_def.name),
                ));
            }

            // Check for duplicate variant names within this enum
            let mut seen_variants: HashSet<String> = HashSet::new();
            let mut variants = Vec::new();
            let mut typed_variants = Vec::new();

            // Set type parameters in scope for resolving variant field types
            let prev_type_params = std::mem::take(&mut self.current_type_params);
            for tp in &enum_def.type_params {
                self.current_type_params.insert(tp.name.clone());
            }

            for variant in &enum_def.variants {
                // Detect duplicate variant name
                if !seen_variants.insert(variant.name.clone()) {
                    return Err(self.type_error(
                        variant.span,
                        format!(
                            "duplicate variant '{}' in enum '{}'",
                            variant.name, enum_def.name
                        ),
                    ));
                }

                // Resolve field types for data variants
                let mut resolved_fields = Vec::new();
                for field in &variant.fields {
                    let resolved_type = self.resolve_enum_field_type(
                        &field.field_type,
                        &enum_def.name,
                        &variant.name,
                        &field.name,
                        field.span,
                    )?;
                    resolved_fields.push((field.name.clone(), resolved_type));
                }

                typed_variants.push(TypedEnumVariant {
                    name: variant.name.clone(),
                    fields: resolved_fields.clone(),
                    span: variant.span,
                });

                variants.push(VariantInfo::with_fields(
                    variant.name.clone(),
                    resolved_fields,
                    variant.span,
                ));
            }

            // Restore previous type params scope
            self.current_type_params = prev_type_params;

            // Extract type parameter names (for generic enums - Phase 4)
            let type_params: Vec<String> = enum_def
                .type_params
                .iter()
                .map(|tp| tp.name.clone())
                .collect();

            // Create and register the enum info
            let enum_info = EnumInfo::with_type_params(
                enum_def.name.clone(),
                type_params.clone(),
                variants,
                enum_def.span,
            );

            // Register in the type environment
            self.env
                .register_enum(enum_info)
                .expect("enum registration should succeed after duplicate check");

            typed_enums.push(TypedEnumDef {
                name: enum_def.name.clone(),
                type_params,
                variants: typed_variants,
                span: enum_def.span,
            });
        }

        Ok(typed_enums)
    }

    /// Resolve a type annotation in an enum variant field context.
    ///
    /// Similar to `resolve_type_annotation` for structs, but produces
    /// enum-specific error messages.
    fn resolve_enum_field_type(
        &self,
        annotation: &TypeAnnotation,
        enum_name: &str,
        variant_name: &str,
        field_name: &str,
        span: Span,
    ) -> Result<FluxType> {
        match annotation {
            TypeAnnotation::F64 => Ok(FluxType::Float),
            TypeAnnotation::Int => Ok(FluxType::Int),
            TypeAnnotation::Bool => Ok(FluxType::Bool),
            TypeAnnotation::Str => Ok(FluxType::String),
            TypeAnnotation::Named(name) => {
                // Check if it's a type parameter in scope (e.g., T in a generic enum)
                if self.current_type_params.contains(name) {
                    return Ok(FluxType::TypeParam(name.clone()));
                }
                // Check if it's a struct type
                if self.struct_registry.contains_key(name) {
                    Ok(FluxType::Struct(name.clone()))
                } else if self.env.has_enum(name) {
                    // Check if it's an enum type
                    Ok(FluxType::Enum(name.clone()))
                } else {
                    let msg = if let Some(import_path) = Self::suggest_import_for_type(name) {
                        format!(
                            "type '{}' is not defined. Did you mean 'from {} import {{{}}}'?",
                            name, import_path, name
                        )
                    } else {
                        format!(
                            "unknown type '{}' in enum '{}' variant '{}' field '{}'",
                            name, enum_name, variant_name, field_name
                        )
                    };
                    Err(self.type_error(span, msg))
                }
            }
            TypeAnnotation::FixedArray(elem_type, size) => {
                if *size == 0 {
                    return Err(self.type_error(
                        span,
                        "array size must be positive, got 0".to_string(),
                    ));
                }
                let resolved_elem = self.resolve_enum_field_type(
                    elem_type,
                    enum_name,
                    variant_name,
                    field_name,
                    span,
                )?;
                Ok(FluxType::FixedArray(Box::new(resolved_elem), *size))
            }
            TypeAnnotation::BitInt(_) => Ok(FluxType::Int),
            TypeAnnotation::Generic(name, type_args) => {
                // Resolve generic types in enum field context
                let resolved_args: Result<Vec<_>> = type_args
                    .iter()
                    .map(|arg| {
                        self.resolve_enum_field_type(
                            arg, enum_name, variant_name, field_name, span,
                        )
                    })
                    .collect();
                let resolved_args = resolved_args?;

                // Validate type argument count for known generic structs
                if let Some(info) = self.struct_registry.get(name) {
                    let expected = info.type_params.len();
                    if expected > 0 && resolved_args.len() != expected {
                        return Err(self.type_error(
                            span,
                            format!(
                                "expected {} type arguments for '{}', got {}",
                                expected, name, resolved_args.len()
                            ),
                        ));
                    }
                }

                Ok(FluxType::Generic(name.clone(), resolved_args))
            }
        }
    }

    /// Type-check impl blocks, registering methods and producing TypedImplBlock nodes.
    ///
    /// For each impl block:
    /// 1. Verify the target struct exists in the type environment
    /// 2. For each method, resolve parameter types and return type
    /// 3. Detect duplicate method names on the same struct
    /// 4. Typecheck method bodies with `self` bound to the struct type
    /// 5. Register each method in the environment for later method call resolution
    fn register_impl_blocks(&mut self, impl_blocks: &[ImplBlock]) -> Result<Vec<TypedImplBlock>> {
        let mut typed_impl_blocks = Vec::new();

        for impl_block in impl_blocks {
            // Skip trait impls for now (Phase 3) — only process inherent impls
            if impl_block.trait_name.is_some() {
                continue;
            }

            // Verify the target struct exists
            let target_type = &impl_block.target_type;
            if !self.struct_registry.contains_key(target_type) {
                return Err(self.type_error(
                    impl_block.span,
                    format!(
                        "Cannot implement methods for unknown type '{}' at {:?}",
                        target_type, impl_block.span
                    ),
                ));
            }

            let mut typed_methods = Vec::new();

            for method in &impl_block.methods {
                // Determine if the method has a `self` parameter
                let has_self = method
                    .params
                    .first()
                    .map_or(false, |p| p.name == "self");

                let is_static = !has_self;

                // Get the non-self parameters for type resolution
                let non_self_params: Vec<&FnParam> = if has_self {
                    method.params.iter().skip(1).collect()
                } else {
                    method.params.iter().collect()
                };

                // Resolve parameter types (excluding self)
                let mut param_types = Vec::new();
                for param in &non_self_params {
                    let param_ty = if let Some(ref annotation) = param.param_type {
                        self.resolve_fn_type_annotation(
                            annotation,
                            &method.name,
                            &format!("parameter '{}'", param.name),
                            param.span,
                        )?
                    } else {
                        FluxType::Float
                    };
                    param_types.push(param_ty);
                }

                // Resolve declared return type
                let declared_return_type = if let Some(ref annotation) = method.return_type {
                    Some(self.resolve_fn_type_annotation(
                        annotation,
                        &method.name,
                        "return type",
                        method.span,
                    )?)
                } else {
                    None
                };

                // Check for duplicate method name on this struct
                if self.env.get_method(target_type, &method.name).is_some() {
                    return Err(self.type_error(
                        method.span,
                        format!(
                            "Method '{}' already defined for type '{}' at {:?}",
                            method.name, target_type, method.span
                        ),
                    ));
                }

                // Typecheck the method body in a new scope
                self.env.push_scope();
                self.in_function_body = true;

                // Store the declared return type for return-statement validation
                let prev_fn_return_type = self.current_fn_return_type.take();
                self.current_fn_return_type = declared_return_type.clone();

                // Bind `self` as the struct type if this is an instance method
                if has_self {
                    self.env.insert("self".to_string(), FluxType::Struct(target_type.clone()));
                }

                // Bind non-self parameters
                let mut all_param_names = Vec::new();
                for (param, ty) in non_self_params.iter().zip(param_types.iter()) {
                    self.env.insert(param.name.clone(), ty.clone());
                    all_param_names.push(param.name.clone());
                }

                // Inject bar context bindings (same as function bodies)
                for (name, ty) in builtins::market_data_bindings() {
                    self.env.insert(name.to_string(), ty);
                }
                for (name, ty) in builtins::signal_function_bindings() {
                    self.env.insert(name.to_string(), ty);
                }
                for (name, ty) in builtins::math_function_bindings() {
                    self.env.insert(name.to_string(), ty);
                }

                // Check body statements
                let mut typed_body = Vec::new();
                for stmt in &method.body {
                    typed_body.push(self.check_stmt(stmt.clone())?);
                }

                // Infer return type
                let return_type = if let Some(ref declared) = declared_return_type {
                    declared.clone()
                } else {
                    self.infer_fn_return_type(&typed_body)
                };

                self.current_fn_return_type = prev_fn_return_type;
                self.in_function_body = false;
                self.env.pop_scope();

                // Build the full param name list (including "self" for TypedFnDef)
                let full_param_names: Vec<String> = if has_self {
                    std::iter::once("self".to_string())
                        .chain(all_param_names)
                        .collect()
                } else {
                    all_param_names
                };

                // Build the full param types list (including struct type for self)
                let full_param_types: Vec<FluxType> = if has_self {
                    std::iter::once(FluxType::Struct(target_type.clone()))
                        .chain(param_types.clone())
                        .collect()
                } else {
                    param_types.clone()
                };

                // Register the method in the type environment
                let method_info = MethodInfo {
                    name: method.name.clone(),
                    param_types: param_types.clone(),
                    return_type: return_type.clone(),
                    is_static,
                    body: typed_body.clone(),
                    span: method.span,
                };
                self.env.register_method(target_type, method_info)
                    .map_err(|()| {
                        self.type_error(
                            method.span,
                            format!(
                                "Method '{}' already defined for type '{}'",
                                method.name, target_type
                            ),
                        )
                    })?;

                typed_methods.push(TypedFnDef {
                    name: method.name.clone(),
                    type_params: method.type_params.iter().map(|tp| tp.name.clone()).collect(),
                    type_param_bounds: method.type_params.iter().map(|tp| tp.bound.clone()).collect(),
                    params: full_param_names,
                    param_types: full_param_types,
                    body: typed_body,
                    return_type,
                    span: method.span,
                });
            }

            typed_impl_blocks.push(TypedImplBlock {
                trait_name: None,
                target_type: target_type.clone(),
                methods: typed_methods,
                span: impl_block.span,
            });
        }

        Ok(typed_impl_blocks)
    }

    /// Register trait definitions in the type environment.
    ///
    /// Validates:
    /// - No duplicate trait names (conflicts with existing traits or types)
    /// - No duplicate method signatures within a single trait
    fn register_traits(&mut self, traits: &[TraitDef]) -> Result<Vec<TypedTraitDef>> {
        let mut typed_traits = Vec::new();

        for trait_def in traits {
            // Check for trait name conflict with existing traits
            if self.env.has_trait(&trait_def.name) {
                return Err(self.type_error(
                    trait_def.span,
                    format!(
                        "Trait name '{}' already defined at {:?}",
                        trait_def.name, trait_def.span
                    ),
                ));
            }

            // Check for trait name conflict with existing types (structs, enums)
            if self.struct_registry.contains_key(&trait_def.name)
                || self.env.has_enum(&trait_def.name)
            {
                return Err(self.type_error(
                    trait_def.span,
                    format!(
                        "Name conflict: '{}' is already defined as a type",
                        trait_def.name
                    ),
                ));
            }

            // Check for duplicate method names within the trait
            let mut seen_methods: HashSet<String> = HashSet::new();
            let mut trait_methods = Vec::new();

            for method_sig in &trait_def.methods {
                if !seen_methods.insert(method_sig.name.clone()) {
                    return Err(self.type_error(
                        method_sig.span,
                        format!(
                            "Duplicate method signature '{}' in trait '{}'",
                            method_sig.name, trait_def.name
                        ),
                    ));
                }

                // Determine if method has self
                let has_self = method_sig
                    .params
                    .first()
                    .map_or(false, |p| p.name == "self");

                // Resolve parameter types (excluding self)
                let non_self_params: Vec<&FnParam> = if has_self {
                    method_sig.params.iter().skip(1).collect()
                } else {
                    method_sig.params.iter().collect()
                };

                let mut param_types = Vec::new();
                for param in &non_self_params {
                    let param_ty = if let Some(ref annotation) = param.param_type {
                        self.resolve_fn_type_annotation(
                            annotation,
                            &method_sig.name,
                            &format!("parameter '{}'", param.name),
                            param.span,
                        )?
                    } else {
                        FluxType::Float // default type
                    };
                    param_types.push(param_ty);
                }

                // Resolve return type
                let return_type = if let Some(ref annotation) = method_sig.return_type {
                    self.resolve_fn_type_annotation(
                        annotation,
                        &method_sig.name,
                        "return type",
                        method_sig.span,
                    )?
                } else {
                    FluxType::Void
                };

                trait_methods.push(TraitMethodInfo {
                    name: method_sig.name.clone(),
                    param_types,
                    return_type,
                    has_self,
                });
            }

            // Register the trait in the type environment
            let trait_info = TraitInfo {
                name: trait_def.name.clone(),
                methods: trait_methods.clone(),
                span: trait_def.span,
            };
            self.env.register_trait(trait_info).map_err(|()| {
                self.type_error(
                    trait_def.span,
                    format!("Trait '{}' already defined", trait_def.name),
                )
            })?;

            typed_traits.push(TypedTraitDef {
                name: trait_def.name.clone(),
                methods: trait_methods,
                span: trait_def.span,
            });
        }

        Ok(typed_traits)
    }

    /// Register trait implementations (impl TraitName for StructName { ... }).
    ///
    /// Validates:
    /// - The trait exists
    /// - The struct exists
    /// - All required methods are implemented
    /// - Method signatures match the trait declaration (param types, return type)
    fn register_trait_impls(&mut self, impl_blocks: &[ImplBlock]) -> Result<Vec<TypedImplBlock>> {
        let mut typed_impl_blocks = Vec::new();

        for impl_block in impl_blocks {
            // Only process trait impls (skip inherent impls — handled by register_impl_blocks)
            let trait_name = match &impl_block.trait_name {
                Some(name) => name.clone(),
                None => continue,
            };

            let target_type = &impl_block.target_type;

            // Verify the trait exists
            let trait_info = match self.env.get_trait(&trait_name) {
                Some(info) => info.clone(),
                None => {
                    return Err(self.type_error(
                        impl_block.span,
                        format!("Unknown trait '{}' in impl block", trait_name),
                    ));
                }
            };

            // Verify the struct exists
            if !self.struct_registry.contains_key(target_type) {
                return Err(self.type_error(
                    impl_block.span,
                    format!(
                        "Cannot implement trait '{}' for unknown type '{}'",
                        trait_name, target_type
                    ),
                ));
            }

            // Typecheck each method in the impl block
            let mut typed_methods = Vec::new();
            let mut impl_method_infos = Vec::new();
            let mut impl_method_names: HashSet<String> = HashSet::new();

            for method in &impl_block.methods {
                impl_method_names.insert(method.name.clone());

                // Determine if the method has a `self` parameter
                let has_self = method
                    .params
                    .first()
                    .map_or(false, |p| p.name == "self");

                let is_static = !has_self;

                // Get the non-self parameters for type resolution
                let non_self_params: Vec<&FnParam> = if has_self {
                    method.params.iter().skip(1).collect()
                } else {
                    method.params.iter().collect()
                };

                // Resolve parameter types (excluding self)
                let mut param_types = Vec::new();
                for param in &non_self_params {
                    let param_ty = if let Some(ref annotation) = param.param_type {
                        self.resolve_fn_type_annotation(
                            annotation,
                            &method.name,
                            &format!("parameter '{}'", param.name),
                            param.span,
                        )?
                    } else {
                        FluxType::Float
                    };
                    param_types.push(param_ty);
                }

                // Resolve declared return type
                let declared_return_type = if let Some(ref annotation) = method.return_type {
                    Some(self.resolve_fn_type_annotation(
                        annotation,
                        &method.name,
                        "return type",
                        method.span,
                    )?)
                } else {
                    None
                };

                // Verify signature matches the trait declaration
                if let Some(trait_method) = trait_info.methods.iter().find(|m| m.name == method.name)
                {
                    // Check parameter types match
                    if param_types.len() != trait_method.param_types.len() {
                        return Err(self.type_error(
                            method.span,
                            format!(
                                "Method '{}' signature mismatch: trait '{}' declares {} parameters (excluding self), impl provides {}",
                                method.name, trait_name, trait_method.param_types.len(), param_types.len()
                            ),
                        ));
                    }

                    for (i, (impl_ty, trait_ty)) in
                        param_types.iter().zip(trait_method.param_types.iter()).enumerate()
                    {
                        if impl_ty != trait_ty && !impl_ty.is_assignable_to(trait_ty) {
                            return Err(self.type_error(
                                method.span,
                                format!(
                                    "Method '{}' signature mismatch: parameter {} expects '{}' (from trait '{}'), got '{}'",
                                    method.name, i + 1, trait_ty, trait_name, impl_ty
                                ),
                            ));
                        }
                    }

                    // Check return type matches
                    let impl_return = declared_return_type.clone().unwrap_or(FluxType::Void);
                    if impl_return != trait_method.return_type
                        && !impl_return.is_assignable_to(&trait_method.return_type)
                    {
                        return Err(self.type_error(
                            method.span,
                            format!(
                                "Method '{}' signature mismatch: trait '{}' declares return type '{}', impl provides '{}'",
                                method.name, trait_name, trait_method.return_type, impl_return
                            ),
                        ));
                    }
                }

                // Typecheck the method body in a new scope
                self.env.push_scope();
                self.in_function_body = true;

                // Store the declared return type for return-statement validation
                let prev_fn_return_type = self.current_fn_return_type.take();
                self.current_fn_return_type = declared_return_type.clone();

                // Bind `self` as the struct type if this is an instance method
                if has_self {
                    self.env
                        .insert("self".to_string(), FluxType::Struct(target_type.clone()));
                }

                // Bind non-self parameters
                let mut all_param_names = Vec::new();
                for (param, ty) in non_self_params.iter().zip(param_types.iter()) {
                    self.env.insert(param.name.clone(), ty.clone());
                    all_param_names.push(param.name.clone());
                }

                // Inject bar context bindings
                for (name, ty) in builtins::market_data_bindings() {
                    self.env.insert(name.to_string(), ty);
                }
                for (name, ty) in builtins::signal_function_bindings() {
                    self.env.insert(name.to_string(), ty);
                }
                for (name, ty) in builtins::math_function_bindings() {
                    self.env.insert(name.to_string(), ty);
                }

                // Check body statements
                let mut typed_body = Vec::new();
                for stmt in &method.body {
                    typed_body.push(self.check_stmt(stmt.clone())?);
                }

                // Infer return type
                let return_type = if let Some(ref declared) = declared_return_type {
                    declared.clone()
                } else {
                    self.infer_fn_return_type(&typed_body)
                };

                self.current_fn_return_type = prev_fn_return_type;
                self.in_function_body = false;
                self.env.pop_scope();

                // Build the full param name list (including "self" for TypedFnDef)
                let full_param_names: Vec<String> = if has_self {
                    std::iter::once("self".to_string())
                        .chain(all_param_names)
                        .collect()
                } else {
                    all_param_names
                };

                // Build the full param types list (including struct type for self)
                let full_param_types: Vec<FluxType> = if has_self {
                    std::iter::once(FluxType::Struct(target_type.clone()))
                        .chain(param_types.clone())
                        .collect()
                } else {
                    param_types.clone()
                };

                impl_method_infos.push(MethodInfo {
                    name: method.name.clone(),
                    param_types: param_types.clone(),
                    return_type: return_type.clone(),
                    is_static,
                    body: typed_body.clone(),
                    span: method.span,
                });

                typed_methods.push(TypedFnDef {
                    name: method.name.clone(),
                    type_params: method.type_params.iter().map(|tp| tp.name.clone()).collect(),
                    type_param_bounds: method.type_params.iter().map(|tp| tp.bound.clone()).collect(),
                    params: full_param_names,
                    param_types: full_param_types,
                    body: typed_body,
                    return_type,
                    span: method.span,
                });
            }

            // Verify all required trait methods are implemented
            let missing_methods: Vec<&str> = trait_info
                .methods
                .iter()
                .filter(|m| !impl_method_names.contains(&m.name))
                .map(|m| m.name.as_str())
                .collect();

            if !missing_methods.is_empty() {
                return Err(self.type_error(
                    impl_block.span,
                    format!(
                        "Type '{}' does not implement all methods of trait '{}': missing [{}]",
                        target_type,
                        trait_name,
                        missing_methods.join(", ")
                    ),
                ));
            }

            // Register trait impl in type environment
            self.env
                .register_trait_impl(&trait_name, target_type, impl_method_infos)
                .map_err(|()| {
                    self.type_error(
                        impl_block.span,
                        format!(
                            "Trait '{}' already implemented for type '{}'",
                            trait_name, target_type
                        ),
                    )
                })?;

            typed_impl_blocks.push(TypedImplBlock {
                trait_name: Some(trait_name),
                target_type: target_type.clone(),
                methods: typed_methods,
                span: impl_block.span,
            });
        }

        Ok(typed_impl_blocks)
    }

    /// Pre-scan the strategy's state block to collect state variable names.
    ///
    /// This is called before checking function bodies so that we can produce
    /// a specific error ("functions cannot access state variable 'X'") instead
    /// of a generic "undefined identifier" error when a function references
    /// a state variable.
    fn collect_state_var_names(&mut self, strategy: &Strategy) {
        for item in &strategy.body {
            if let StrategyItem::StateBlock(sb) = item {
                for var in &sb.variables {
                    self.state_var_names.insert(var.name.clone());
                }
            }
        }
    }

    /// Type-check all user-defined function bodies, producing TypedFnDef nodes.
    ///
    /// Called after `register_functions` so that functions can reference each other.
    /// Each function body is checked in a new scope with parameter bindings,
    /// bar context, built-in functions, and other user-defined function bindings.
    fn check_fn_defs(&mut self, functions: Vec<FnDef>) -> Result<Vec<TypedFnDef>> {
        let mut typed_functions = Vec::new();
        for fn_def in functions {
            typed_functions.push(self.check_fn_def(fn_def)?);
        }
        Ok(typed_functions)
    }

    /// Type-check a single user-defined function body.
    ///
    /// Pushes a new scope with:
    /// - Parameter bindings (typed according to annotations, or Float if untyped)
    /// - Bar context bindings (close, open, high, low, volume, symbol, in_position)
    /// - Signal function bindings (OPEN, CLOSE)
    /// - Math/stats/portfolio function bindings
    /// Then checks all body statements and validates the return type.
    fn check_fn_def(&mut self, fn_def: FnDef) -> Result<TypedFnDef> {
        self.env.push_scope();
        self.in_function_body = true;

        // Set type parameters in scope for resolving generic function signatures
        let prev_type_params = std::mem::take(&mut self.current_type_params);
        for tp in &fn_def.type_params {
            self.current_type_params.insert(tp.name.clone());
        }

        // Resolve declared return type if present
        let declared_return_type = if let Some(ref annotation) = fn_def.return_type {
            Some(self.resolve_fn_type_annotation(
                annotation,
                &fn_def.name,
                "return type",
                fn_def.span,
            )?)
        } else {
            None
        };

        // Store the declared return type for return-statement validation
        let prev_fn_return_type = self.current_fn_return_type.take();
        self.current_fn_return_type = declared_return_type.clone();

        // Bind parameters with resolved types
        let mut param_types = Vec::new();
        for param in &fn_def.params {
            let param_ty = if let Some(ref annotation) = param.param_type {
                self.resolve_fn_type_annotation(
                    annotation,
                    &fn_def.name,
                    &format!("parameter '{}'", param.name),
                    param.span,
                )?
            } else {
                FluxType::Float
            };
            self.env.insert(param.name.clone(), param_ty.clone());
            param_types.push(param_ty);
        }

        // Inject bar context (same bindings as event handlers)
        for (name, ty) in builtins::market_data_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Inject signal function bindings
        for (name, ty) in builtins::signal_function_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Inject math/stats/portfolio function bindings
        for (name, ty) in builtins::math_function_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Check body statements
        let mut typed_body = Vec::new();
        for stmt in fn_def.body {
            typed_body.push(self.check_stmt(stmt)?);
        }

        // Infer return type from return statements (or Null if none)
        let return_type = if let Some(ref declared) = declared_return_type {
            declared.clone()
        } else {
            self.infer_fn_return_type(&typed_body)
        };

        self.current_fn_return_type = prev_fn_return_type;
        self.in_function_body = false;
        self.env.pop_scope();

        Ok(TypedFnDef {
            name: fn_def.name,
            type_params: fn_def.type_params.iter().map(|tp| tp.name.clone()).collect(),
            type_param_bounds: fn_def.type_params.iter().map(|tp| tp.bound.clone()).collect(),
            params: fn_def.params.into_iter().map(|p| p.name).collect(),
            param_types,
            body: typed_body,
            return_type,
            span: fn_def.span,
        })
    }

    /// Infer a function's return type by walking its body statements.
    ///
    /// If any `return expr` is found, uses the expression's resolved type.
    /// If no return statement exists, the return type is `FluxType::Null`.
    fn infer_fn_return_type(&self, body: &[TypedStmt]) -> FluxType {
        if let Some(ty) = self.find_return_type(body) {
            ty
        } else {
            FluxType::Null
        }
    }

    /// Recursively search statements for a return expression's type.
    fn find_return_type(&self, stmts: &[TypedStmt]) -> Option<FluxType> {
        for stmt in stmts {
            match stmt {
                TypedStmt::Return(ret) => {
                    return Some(match &ret.value {
                        Some(expr) => expr.resolved_type.clone(),
                        None => FluxType::Null,
                    });
                }
                TypedStmt::If(if_stmt) => {
                    if let Some(ty) = self.find_return_type(&if_stmt.body) {
                        return Some(ty);
                    }
                    for elif in &if_stmt.elif_branches {
                        if let Some(ty) = self.find_return_type(&elif.body) {
                            return Some(ty);
                        }
                    }
                    if let Some(else_body) = &if_stmt.else_body {
                        if let Some(ty) = self.find_return_type(else_body) {
                            return Some(ty);
                        }
                    }
                }
                TypedStmt::For(for_loop) => {
                    if let Some(ty) = self.find_return_type(&for_loop.body) {
                        return Some(ty);
                    }
                }
                TypedStmt::While(while_loop) => {
                    if let Some(ty) = self.find_return_type(&while_loop.body) {
                        return Some(ty);
                    }
                }
                _ => {}
            }
        }
        None
    }


    fn check_strategy(&mut self, strategy: Strategy) -> Result<TypedStrategy> {
        self.env.push_scope(); // strategy scope

        // First pass: register params and state so they are visible to each other
        for item in &strategy.body {
            match item {
                StrategyItem::ParamsBlock(pb) => self.register_params(pb)?,
                StrategyItem::StateBlock(sb) => self.register_state(sb)?,
                _ => {}
            }
        }

        // Second pass: type-check all items
        let mut typed_body = Vec::new();
        for item in strategy.body {
            typed_body.push(self.check_strategy_item(item)?);
        }

        self.env.pop_scope(); // leave strategy scope

        Ok(TypedStrategy {
            name: strategy.name,
            body: typed_body,
            span: strategy.span,
        })
    }

    fn register_params(&mut self, params_block: &ParamsBlock) -> Result<()> {
        for param in &params_block.params {
            let ty = self.infer_literal_type(&param.default_value)?;
            self.env.insert(param.name.clone(), ty);
        }
        Ok(())
    }

    fn register_state(&mut self, state_block: &StateBlock) -> Result<()> {
        for var in &state_block.variables {
            let ty = self.infer_state_init_type(&var.initial_value)?;
            self.env.insert(var.name.clone(), ty);
        }
        Ok(())
    }

    fn infer_literal_type(&self, expr: &Expr) -> Result<FluxType> {
        match &expr.kind {
            ExprKind::IntLiteral(_) => Ok(FluxType::Int),
            ExprKind::FloatLiteral(_) => Ok(FluxType::Float),
            ExprKind::StringLiteral(_) => Ok(FluxType::String),
            ExprKind::BoolLiteral(_) => Ok(FluxType::Bool),
            ExprKind::NullLiteral => Ok(FluxType::Null),
            _ => Err(self.type_error(
                expr.span,
                "parameter default must be a literal value".to_string(),
            )),
        }
    }

    fn infer_state_init_type(&self, expr: &Expr) -> Result<FluxType> {
        match &expr.kind {
            ExprKind::IntLiteral(_) => Ok(FluxType::Int),
            ExprKind::FloatLiteral(_) => Ok(FluxType::Float),
            ExprKind::StringLiteral(_) => Ok(FluxType::String),
            ExprKind::BoolLiteral(_) => Ok(FluxType::Bool),
            ExprKind::NullLiteral => Ok(FluxType::Null),
            ExprKind::ListLiteral(elements) => {
                if elements.is_empty() {
                    Ok(FluxType::List(Box::new(FluxType::Null)))
                } else {
                    // Infer element type from first element (must be literal)
                    let first_ty = self.infer_literal_type(&elements[0])?;
                    let mut all_numeric = first_ty.is_numeric();
                    for elem in elements.iter().skip(1) {
                        let elem_ty = self.infer_literal_type(elem)?;
                        if !elem_ty.is_numeric() {
                            all_numeric = false;
                        }
                        if elem_ty != first_ty {
                            // Check numeric coercion
                            if first_ty.is_numeric() && elem_ty.is_numeric() {
                                continue; // will coerce to Float
                            }
                            return Err(self.type_error(
                                expr.span,
                                format!(
                                    "list elements have incompatible types: {} and {}",
                                    first_ty, elem_ty
                                ),
                            ));
                        }
                    }
                    // All-numeric list literals infer VecFloat
                    if all_numeric {
                        Ok(FluxType::VecFloat)
                    } else {
                        Ok(FluxType::List(Box::new(first_ty)))
                    }
                }
            }
            ExprKind::Ident(name) => {
                if let Some(ty) = self.env.resolve(name) {
                    Ok(ty.clone())
                } else {
                    Err(self.type_error(
                        expr.span,
                        format!("undefined identifier '{}'", name),
                    ))
                }
            }
            _ => Err(self.type_error(
                expr.span,
                "state variable initializer must be a literal or list literal".to_string(),
            )),
        }
    }

    fn check_strategy_item(&mut self, item: StrategyItem) -> Result<TypedStrategyItem> {
        match item {
            StrategyItem::Property(prop) => {
                let typed_value = self.check_expr(prop.value)?;
                Ok(TypedStrategyItem::Property(TypedProperty {
                    name: prop.name,
                    value: typed_value,
                    span: prop.span,
                }))
            }
            StrategyItem::ParamsBlock(pb) => {
                let typed_params = self.check_params_block(pb)?;
                Ok(TypedStrategyItem::ParamsBlock(typed_params))
            }
            StrategyItem::StateBlock(sb) => {
                let typed_state = self.check_state_block(sb)?;
                Ok(TypedStrategyItem::StateBlock(typed_state))
            }
            StrategyItem::EventHandler(eh) => {
                let typed_handler = self.check_event_handler(eh)?;
                Ok(TypedStrategyItem::EventHandler(typed_handler))
            }
        }
    }

    fn check_params_block(&mut self, pb: ParamsBlock) -> Result<TypedParamsBlock> {
        let mut typed_params = Vec::new();
        for param in pb.params {
            let resolved_type = self.infer_literal_type(&param.default_value)?;
            let typed_default = self.check_expr(param.default_value)?;
            typed_params.push(TypedParam {
                name: param.name,
                default_value: typed_default,
                resolved_type,
                span: param.span,
            });
        }
        Ok(TypedParamsBlock {
            params: typed_params,
            span: pb.span,
        })
    }

    fn check_state_block(&mut self, sb: StateBlock) -> Result<TypedStateBlock> {
        let mut typed_vars = Vec::new();
        for var in sb.variables {
            let typed_init = self.check_expr(var.initial_value)?;
            let resolved_type = typed_init.resolved_type.clone();
            typed_vars.push(TypedStateVar {
                name: var.name,
                initial_value: typed_init,
                resolved_type,
                span: var.span,
            });
        }
        Ok(TypedStateBlock {
            variables: typed_vars,
            span: sb.span,
        })
    }

    fn check_event_handler(&mut self, handler: EventHandler) -> Result<TypedEventHandler> {
        // Validate event name
        if !builtins::valid_event_names().contains(&handler.event_name.as_str()) {
            return Err(self.type_error(
                handler.span,
                format!("unrecognized event handler '{}'", handler.event_name),
            ));
        }

        self.env.push_scope(); // handler scope
        self.in_event_handler = true;

        // Inject market data bindings
        for (name, ty) in builtins::market_data_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Inject signal function bindings
        for (name, ty) in builtins::signal_function_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Inject math/stats/portfolio function bindings
        for (name, ty) in builtins::math_function_bindings() {
            self.env.insert(name.to_string(), ty);
        }

        // Check handler body statements
        let mut typed_body = Vec::new();
        for stmt in handler.body {
            typed_body.push(self.check_stmt(stmt)?);
        }

        self.in_event_handler = false;
        self.env.pop_scope(); // leave handler scope

        Ok(TypedEventHandler {
            event_name: handler.event_name,
            body: typed_body,
            span: handler.span,
        })
    }

    fn check_stmt(&mut self, stmt: Stmt) -> Result<TypedStmt> {
        match stmt {
            Stmt::Assignment(assign) => self.check_assignment(assign),
            Stmt::If(if_stmt) => self.check_if(if_stmt),
            Stmt::For(for_loop) => self.check_for(for_loop),
            Stmt::While(while_loop) => self.check_while(while_loop),
            Stmt::Return(ret) => self.check_return(ret),
            Stmt::Expr(expr_stmt) => self.check_expr_stmt(expr_stmt),
        }
    }

    fn check_assignment(&mut self, assign: Assignment) -> Result<TypedStmt> {
        let typed_value = self.check_expr(assign.value)?;
        let value_type = typed_value.resolved_type.clone();

        // Handle different assignment targets
        match &assign.target.kind {
            ExprKind::Ident(name) => {
                if let Some(existing_ty) = self.env.resolve(name).cloned() {
                    // Reassignment: check type compatibility
                    if !value_type.is_assignable_to(&existing_ty) {
                        return Err(self.type_error(
                            assign.span,
                            format!(
                                "cannot assign {} to variable of type {}",
                                value_type, existing_ty
                            ),
                        ));
                    }
                } else {
                    // New variable: add to current scope
                    self.env.insert(name.clone(), value_type.clone());
                }
                let typed_target = TypedExpr {
                    kind: TypedExprKind::Ident(name.clone()),
                    resolved_type: value_type,
                    span: assign.target.span,
                };
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
            ExprKind::IndexAccess { .. } => {
                let typed_target = self.check_expr(assign.target)?;
                // Verify value type matches element type
                if !value_type.is_assignable_to(&typed_target.resolved_type) {
                    return Err(self.type_error(
                        assign.span,
                        format!(
                            "cannot assign {} to element of type {}",
                            value_type, typed_target.resolved_type
                        ),
                    ));
                }
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
            ExprKind::MemberAccess { object, field } => {
                // Check @immutable enforcement before moving assign.target
                let immutable_field = field.clone();
                let immutable_struct_name = self.resolve_struct_type_from_expr(object);
                if let Some(ref struct_name) = immutable_struct_name {
                    if let Some(info) = self.struct_registry.get(struct_name) {
                        let is_immutable = info.decorators.iter().any(|d| d.kind == DecoratorKind::Immutable);
                        if is_immutable {
                            return Err(self.type_error(
                                assign.span,
                                format!(
                                    "cannot assign to field '{}' of @immutable struct '{}'",
                                    immutable_field, struct_name
                                ),
                            ));
                        }
                    }
                }
                let typed_target = self.check_expr(assign.target)?;
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
            _ => {
                // Type-check the target expression normally
                let typed_target = self.check_expr(assign.target)?;
                Ok(TypedStmt::Assignment(TypedAssignment {
                    target: typed_target,
                    value: typed_value,
                    span: assign.span,
                }))
            }
        }
    }

    fn check_if(&mut self, if_stmt: IfStmt) -> Result<TypedStmt> {
        let typed_condition = self.check_expr(if_stmt.condition)?;
        if typed_condition.resolved_type != FluxType::Bool {
            return Err(self.type_error(
                typed_condition.span,
                format!(
                    "if condition must be Bool, found {}",
                    typed_condition.resolved_type
                ),
            ));
        }

        // Check if body in new scope
        self.env.push_scope();
        let mut typed_body = Vec::new();
        for stmt in if_stmt.body {
            typed_body.push(self.check_stmt(stmt)?);
        }
        self.env.pop_scope();

        // Check elif branches
        let mut typed_elifs = Vec::new();
        for elif in if_stmt.elif_branches {
            let typed_elif_cond = self.check_expr(elif.condition)?;
            if typed_elif_cond.resolved_type != FluxType::Bool {
                return Err(self.type_error(
                    typed_elif_cond.span,
                    format!(
                        "elif condition must be Bool, found {}",
                        typed_elif_cond.resolved_type
                    ),
                ));
            }
            self.env.push_scope();
            let mut typed_elif_body = Vec::new();
            for stmt in elif.body {
                typed_elif_body.push(self.check_stmt(stmt)?);
            }
            self.env.pop_scope();
            typed_elifs.push(TypedElifBranch {
                condition: typed_elif_cond,
                body: typed_elif_body,
                span: elif.span,
            });
        }

        // Check else body
        let typed_else = if let Some(else_body) = if_stmt.else_body {
            self.env.push_scope();
            let mut typed_else_body = Vec::new();
            for stmt in else_body {
                typed_else_body.push(self.check_stmt(stmt)?);
            }
            self.env.pop_scope();
            Some(typed_else_body)
        } else {
            None
        };

        Ok(TypedStmt::If(TypedIfStmt {
            condition: typed_condition,
            body: typed_body,
            elif_branches: typed_elifs,
            else_body: typed_else,
            span: if_stmt.span,
        }))
    }

    fn check_for(&mut self, for_loop: ForLoop) -> Result<TypedStmt> {
        let typed_iterable = self.check_expr(for_loop.iterable)?;

        // Iterable must be a List type
        let elem_type = match &typed_iterable.resolved_type {
            FluxType::List(t) => t.as_ref().clone(),
            other => {
                return Err(self.type_error(
                    typed_iterable.span,
                    format!("for-loop requires List type, found {}", other),
                ));
            }
        };

        // Push loop body scope with the loop variable bound
        self.env.push_scope();
        self.env.insert(for_loop.variable.clone(), elem_type.clone());

        let mut typed_body = Vec::new();
        for stmt in for_loop.body {
            typed_body.push(self.check_stmt(stmt)?);
        }

        self.env.pop_scope();

        Ok(TypedStmt::For(TypedForLoop {
            variable: for_loop.variable,
            variable_type: elem_type,
            iterable: typed_iterable,
            body: typed_body,
            span: for_loop.span,
        }))
    }

    fn check_while(&mut self, while_loop: WhileLoop) -> Result<TypedStmt> {
        let typed_condition = self.check_expr(while_loop.condition)?;
        if typed_condition.resolved_type != FluxType::Bool {
            return Err(self.type_error(
                typed_condition.span,
                format!(
                    "while condition must be Bool, found {}",
                    typed_condition.resolved_type
                ),
            ));
        }

        self.env.push_scope();
        let mut typed_body = Vec::new();
        for stmt in while_loop.body {
            typed_body.push(self.check_stmt(stmt)?);
        }
        self.env.pop_scope();

        Ok(TypedStmt::While(TypedWhileLoop {
            condition: typed_condition,
            body: typed_body,
            span: while_loop.span,
        }))
    }

    fn check_return(&mut self, ret: ReturnStmt) -> Result<TypedStmt> {
        let typed_value = if let Some(val) = ret.value {
            let typed_expr = self.check_expr(val)?;

            // Validate return expression type against declared return type
            if let Some(ref declared_ret) = self.current_fn_return_type {
                if !typed_expr.resolved_type.is_assignable_to(declared_ret) {
                    // Use struct-specific error format when both types are structs
                    let msg = match (&typed_expr.resolved_type, declared_ret) {
                        (FluxType::Struct(actual), FluxType::Struct(expected)) => {
                            format!(
                                "expected struct '{}', got struct '{}'",
                                expected, actual
                            )
                        }
                        _ => {
                            format!(
                                "expected return type {}, got {}",
                                declared_ret, typed_expr.resolved_type
                            )
                        }
                    };
                    return Err(self.type_error(typed_expr.span, msg));
                }
            }

            Some(typed_expr)
        } else {
            // Return with no value — check that declared return type is Null/Void (or absent)
            if let Some(ref declared_ret) = self.current_fn_return_type {
                if *declared_ret != FluxType::Null && *declared_ret != FluxType::Void {
                    return Err(self.type_error(
                        ret.span,
                        format!(
                            "expected return type {}, got Null",
                            declared_ret
                        ),
                    ));
                }
            }
            None
        };
        Ok(TypedStmt::Return(TypedReturnStmt {
            value: typed_value,
            span: ret.span,
        }))
    }

    fn check_expr_stmt(&mut self, expr_stmt: ExprStmt) -> Result<TypedStmt> {
        let typed_expr = self.check_expr(expr_stmt.expr)?;
        Ok(TypedStmt::Expr(TypedExprStmt {
            expr: typed_expr,
            span: expr_stmt.span,
        }))
    }

    // -----------------------------------------------------------------------
    // Expression checking
    // -----------------------------------------------------------------------

    /// Type-check an expression, returning a TypedExpr with a resolved type.
    pub fn check_expr(&mut self, expr: Expr) -> Result<TypedExpr> {
        let span = expr.span;
        match expr.kind {
            ExprKind::IntLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::IntLiteral(v),
                resolved_type: FluxType::Int,
                span,
            }),
            ExprKind::FloatLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::FloatLiteral(v),
                resolved_type: FluxType::Float,
                span,
            }),
            ExprKind::StringLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::StringLiteral(v),
                resolved_type: FluxType::String,
                span,
            }),
            ExprKind::BoolLiteral(v) => Ok(TypedExpr {
                kind: TypedExprKind::BoolLiteral(v),
                resolved_type: FluxType::Bool,
                span,
            }),
            ExprKind::NullLiteral => Ok(TypedExpr {
                kind: TypedExprKind::NullLiteral,
                resolved_type: FluxType::Null,
                span,
            }),
            ExprKind::Ident(name) => self.check_ident(&name, span),
            ExprKind::BinaryOp { left, op, right } => {
                self.check_binary_op(*left, op, *right, span)
            }
            ExprKind::UnaryOp { op, operand } => self.check_unary_op(op, *operand, span),
            ExprKind::FunctionCall { function, args } => {
                self.check_function_call(*function, args, span)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => self.check_method_call(*receiver, &method, args, span),
            ExprKind::IndexAccess { object, index } => {
                self.check_index_access(*object, *index, span)
            }
            ExprKind::ListLiteral(elements) => self.check_list_literal(elements, span),
            ExprKind::MemberAccess { object, field } => {
                self.check_member_access(*object, &field, span)
            }
            ExprKind::StructLiteral {
                struct_name,
                fields,
            } => self.check_struct_literal(&struct_name, fields, span),
            ExprKind::EnumConstruction {
                enum_name,
                variant_name,
                args,
            } => self.check_enum_construction(&enum_name, &variant_name, args, span),
            ExprKind::Match(match_expr) => {
                self.check_match_expr(match_expr, span)
            }
        }
    }

    fn check_ident(&mut self, name: &str, span: Span) -> Result<TypedExpr> {
        if let Some(ty) = self.env.resolve(name) {
            let resolved = ty.clone();
            Ok(TypedExpr {
                kind: TypedExprKind::Ident(name.to_string()),
                resolved_type: resolved,
                span,
            })
        } else {
            // Check if a function body is trying to access a state variable
            if self.in_function_body && self.state_var_names.contains(name) {
                return Err(self.type_error(
                    span,
                    format!("functions cannot access state variable '{}'", name),
                ));
            }
            // Check if it's a market data identifier used outside an event handler
            let market_data_names: Vec<&str> = builtins::market_data_bindings()
                .iter()
                .map(|(n, _)| *n)
                .collect();
            if market_data_names.contains(&name) && !self.in_event_handler {
                Err(self.type_error(
                    span,
                    format!("'{}' is only available inside event handlers", name),
                ))
            } else {
                Err(self.type_error(
                    span,
                    format!("undefined identifier '{}'", name),
                ))
            }
        }
    }

    fn check_binary_op(
        &mut self,
        left: Expr,
        op: BinOp,
        right: Expr,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_left = self.check_expr(left)?;
        let typed_right = self.check_expr(right)?;
        let left_ty = &typed_left.resolved_type;
        let right_ty = &typed_right.resolved_type;

        let result_type = match op {
            // Arithmetic operators
            BinOp::Add => {
                // String concatenation
                if left_ty == &FluxType::String && right_ty == &FluxType::String {
                    FluxType::String
                } else if let Some(ty) = FluxType::arithmetic_result(left_ty, right_ty) {
                    ty
                } else {
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '+' requires numeric operands, found {} and {}",
                            left_ty, right_ty
                        ),
                    ));
                }
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                if let Some(ty) = FluxType::arithmetic_result(left_ty, right_ty) {
                    ty
                } else {
                    let op_str = match op {
                        BinOp::Sub => "-",
                        BinOp::Mul => "*",
                        BinOp::Div => "/",
                        BinOp::Mod => "%",
                        _ => unreachable!(),
                    };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires numeric operands, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
            }
            // Comparison operators (ordering)
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if left_ty.is_numeric() && right_ty.is_numeric() {
                    FluxType::Bool
                } else {
                    let op_str = match op {
                        BinOp::Lt => "<",
                        BinOp::Le => "<=",
                        BinOp::Gt => ">",
                        BinOp::Ge => ">=",
                        _ => unreachable!(),
                    };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires numeric operands, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
            }
            // Equality operators
            BinOp::Eq | BinOp::Ne => {
                if left_ty == right_ty {
                    FluxType::Bool
                } else if left_ty.is_numeric() && right_ty.is_numeric() {
                    FluxType::Bool
                } else {
                    let op_str = if op == BinOp::Eq { "==" } else { "!=" };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires matching types, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
            }
            // Logical operators
            BinOp::And | BinOp::Or => {
                if left_ty != &FluxType::Bool || right_ty != &FluxType::Bool {
                    let op_str = if op == BinOp::And { "and" } else { "or" };
                    return Err(self.type_error(
                        span,
                        format!(
                            "operator '{}' requires boolean operands, found {} and {}",
                            op_str, left_ty, right_ty
                        ),
                    ));
                }
                FluxType::Bool
            }
        };

        Ok(TypedExpr {
            kind: TypedExprKind::BinaryOp {
                left: Box::new(typed_left),
                op,
                right: Box::new(typed_right),
            },
            resolved_type: result_type,
            span,
        })
    }

    fn check_unary_op(&mut self, op: UnaryOp, operand: Expr, span: Span) -> Result<TypedExpr> {
        let typed_operand = self.check_expr(operand)?;
        let operand_ty = &typed_operand.resolved_type;

        let result_type = match op {
            UnaryOp::Neg => {
                if !operand_ty.is_numeric() {
                    return Err(self.type_error(
                        span,
                        format!("negation requires numeric operand, found {}", operand_ty),
                    ));
                }
                operand_ty.clone()
            }
            UnaryOp::Not => {
                if operand_ty != &FluxType::Bool {
                    return Err(self.type_error(
                        span,
                        format!(
                            "logical negation requires boolean operand, found {}",
                            operand_ty
                        ),
                    ));
                }
                FluxType::Bool
            }
        };

        Ok(TypedExpr {
            kind: TypedExprKind::UnaryOp {
                op,
                operand: Box::new(typed_operand),
            },
            resolved_type: result_type,
            span,
        })
    }

    fn check_function_call(
        &mut self,
        function: Expr,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_function = self.check_expr(function)?;

        // Check the function expression is callable
        let fn_type = typed_function.resolved_type.clone();
        match &fn_type {
            FluxType::Fn { params, ret } => {
                let typed_args = self.check_call_args(
                    &typed_function,
                    params,
                    args,
                    span,
                )?;
                // Infer concrete types for type parameters from argument types
                let ret_type = self.infer_generic_return_type(
                    params,
                    &typed_args,
                    ret.as_ref(),
                );

                // Check trait bounds on type parameters at this call site
                let fn_name = match &typed_function.kind {
                    TypedExprKind::Ident(n) => Some(n.clone()),
                    _ => None,
                };
                if let Some(ref name) = fn_name {
                    self.check_trait_bounds_at_call_site(name, params, &typed_args, span)?;
                }

                Ok(TypedExpr {
                    kind: TypedExprKind::FunctionCall {
                        function: Box::new(typed_function),
                        args: typed_args,
                    },
                    resolved_type: ret_type,
                    span,
                })
            }
            _ => {
                // Get the function name for a better error message
                let name = match &typed_function.kind {
                    TypedExprKind::Ident(n) => n.clone(),
                    _ => "expression".to_string(),
                };
                Err(self.type_error(
                    span,
                    format!("'{}' is not a function (type: {})", name, fn_type),
                ))
            }
        }
    }

    /// Infer concrete types for type parameters at a generic function call site.
    ///
    /// Given the declared parameter types and the actual argument types, builds a
    /// substitution map (TypeParam name → concrete FluxType) and applies it to
    /// the declared return type.
    fn infer_generic_return_type(
        &self,
        params: &FnParams,
        typed_args: &[TypedExpr],
        declared_ret: &FluxType,
    ) -> FluxType {
        // Only attempt inference for fixed params (not variadic/overloaded)
        let param_types = match params {
            FnParams::Fixed(types) => types,
            _ => return declared_ret.clone(),
        };

        // Build substitution map by matching param types to arg types
        let mut substitutions: HashMap<String, FluxType> = HashMap::new();
        for (param_ty, arg) in param_types.iter().zip(typed_args.iter()) {
            Self::collect_type_param_bindings(param_ty, &arg.resolved_type, &mut substitutions);
        }

        // If no type params found, return type as-is
        if substitutions.is_empty() {
            return declared_ret.clone();
        }

        // Substitute type params in the return type
        Self::substitute_type_params(declared_ret, &substitutions)
    }

    /// Check that concrete types inferred at a generic function call site satisfy
    /// the declared trait bounds on the function's type parameters.
    ///
    /// For example, given `fn process[T: DataFeed](feed: T)` called with a `LiveFeed`,
    /// this verifies that `LiveFeed` has a registered trait impl for `DataFeed`.
    ///
    /// Requirements: 9.2, 9.3, 13.5
    fn check_trait_bounds_at_call_site(
        &self,
        fn_name: &str,
        params: &FnParams,
        typed_args: &[TypedExpr],
        span: Span,
    ) -> Result<()> {
        // Look up trait bounds for this function
        let bounds = match self.fn_trait_bounds.get(fn_name) {
            Some(b) => b,
            None => return Ok(()), // No bounds to check
        };

        // Build substitution map: type param name → concrete type
        let param_types = match params {
            FnParams::Fixed(types) => types,
            _ => return Ok(()),
        };

        let mut substitutions: HashMap<String, FluxType> = HashMap::new();
        for (param_ty, arg) in param_types.iter().zip(typed_args.iter()) {
            Self::collect_type_param_bindings(param_ty, &arg.resolved_type, &mut substitutions);
        }

        // For each bound, check that the concrete type implements the required trait
        for (type_param_name, trait_name) in bounds {
            if let Some(concrete_type) = substitutions.get(type_param_name) {
                let type_name = Self::flux_type_to_name(concrete_type);
                if let Some(ref name) = type_name {
                    if !self.env.has_trait_impl(name, trait_name) {
                        return Err(self.type_error(
                            span,
                            format!(
                                "Type '{}' does not implement trait '{}' required by bound on '{}'",
                                name, trait_name, type_param_name
                            ),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// Extract a type name string from a FluxType for trait bound checking.
    /// Returns the struct/enum name for nominal types, None for primitives or
    /// types that can't implement user-defined traits.
    fn flux_type_to_name(ty: &FluxType) -> Option<String> {
        match ty {
            FluxType::Struct(name) => Some(name.clone()),
            FluxType::Enum(name) => Some(name.clone()),
            FluxType::Generic(name, _) => Some(name.clone()),
            FluxType::Int => Some("Int".to_string()),
            FluxType::Float => Some("Float".to_string()),
            FluxType::String => Some("String".to_string()),
            FluxType::Bool => Some("Bool".to_string()),
            _ => None,
        }
    }

    /// Recursively collect type parameter → concrete type bindings by matching
    /// a declared type (which may contain TypeParam) against an actual type.
    fn collect_type_param_bindings(
        declared: &FluxType,
        actual: &FluxType,
        bindings: &mut HashMap<String, FluxType>,
    ) {
        match declared {
            FluxType::TypeParam(name) => {
                // First binding wins (don't overwrite with conflicting info)
                bindings.entry(name.clone()).or_insert_with(|| actual.clone());
            }
            FluxType::Generic(name, decl_args) => {
                if let FluxType::Generic(actual_name, actual_args) = actual {
                    if name == actual_name && decl_args.len() == actual_args.len() {
                        for (d, a) in decl_args.iter().zip(actual_args.iter()) {
                            Self::collect_type_param_bindings(d, a, bindings);
                        }
                    }
                }
            }
            FluxType::List(inner) => {
                if let FluxType::List(actual_inner) = actual {
                    Self::collect_type_param_bindings(inner, actual_inner, bindings);
                }
            }
            FluxType::FixedArray(elem, _) => {
                if let FluxType::FixedArray(actual_elem, _) = actual {
                    Self::collect_type_param_bindings(elem, actual_elem, bindings);
                }
            }
            _ => {} // Concrete types don't contribute bindings
        }
    }

    /// Substitute type parameters with concrete types in a FluxType.
    fn substitute_type_params(
        ty: &FluxType,
        substitutions: &HashMap<String, FluxType>,
    ) -> FluxType {
        match ty {
            FluxType::TypeParam(name) => {
                substitutions.get(name).cloned().unwrap_or_else(|| ty.clone())
            }
            FluxType::Generic(name, args) => {
                let resolved_args: Vec<FluxType> = args
                    .iter()
                    .map(|a| Self::substitute_type_params(a, substitutions))
                    .collect();
                FluxType::Generic(name.clone(), resolved_args)
            }
            FluxType::List(inner) => {
                FluxType::List(Box::new(Self::substitute_type_params(inner, substitutions)))
            }
            FluxType::FixedArray(elem, size) => {
                FluxType::FixedArray(
                    Box::new(Self::substitute_type_params(elem, substitutions)),
                    *size,
                )
            }
            FluxType::Fn { params, ret } => {
                let new_ret = Self::substitute_type_params(ret, substitutions);
                FluxType::Fn {
                    params: params.clone(),
                    ret: Box::new(new_ret),
                }
            }
            // All other types are concrete and don't need substitution
            _ => ty.clone(),
        }
    }

    fn check_call_args(
        &mut self,
        function: &TypedExpr,
        params: &FnParams,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<Vec<TypedExpr>> {
        let fn_name = match &function.kind {
            TypedExprKind::Ident(n) => n.clone(),
            _ => "function".to_string(),
        };

        match params {
            FnParams::Fixed(param_types) => {
                if args.len() != param_types.len() {
                    return Err(self.type_error(
                        span,
                        format!(
                            "'{}' expects {} arguments, found {}",
                            fn_name,
                            param_types.len(),
                            args.len()
                        ),
                    ));
                }
                let mut typed_args = Vec::new();
                for (i, (arg, expected_ty)) in
                    args.into_iter().zip(param_types.iter()).enumerate()
                {
                    let typed_arg = self.check_expr(arg)?;
                    if !typed_arg.resolved_type.is_assignable_to(expected_ty) {
                        // Use struct-specific error format when both types are structs
                        let msg = match (&typed_arg.resolved_type, expected_ty) {
                            (FluxType::Struct(actual), FluxType::Struct(expected)) => {
                                format!(
                                    "expected struct '{}', got struct '{}'",
                                    expected, actual
                                )
                            }
                            _ => {
                                format!(
                                    "'{}' argument {} must be {}, found {}",
                                    fn_name,
                                    i + 1,
                                    expected_ty,
                                    typed_arg.resolved_type
                                )
                            }
                        };
                        return Err(self.type_error(typed_arg.span, msg));
                    }
                    typed_args.push(typed_arg);
                }
                Ok(typed_args)
            }
            FnParams::VariadicNumeric => {
                let mut typed_args = Vec::new();
                for (i, arg) in args.into_iter().enumerate() {
                    let typed_arg = self.check_expr(arg)?;
                    if !typed_arg.resolved_type.is_numeric() {
                        return Err(self.type_error(
                            typed_arg.span,
                            format!(
                                "'{}' argument {} must be numeric, found {}",
                                fn_name,
                                i + 1,
                                typed_arg.resolved_type
                            ),
                        ));
                    }
                    typed_args.push(typed_arg);
                }
                Ok(typed_args)
            }
            FnParams::Overloaded(signatures) => {
                // Type-check all args first
                let mut typed_args = Vec::new();
                for arg in args {
                    typed_args.push(self.check_expr(arg)?);
                }

                // Try each signature
                for sig in signatures {
                    if typed_args.len() != sig.len() {
                        continue;
                    }
                    let mut matches = true;
                    for (typed_arg, expected_ty) in typed_args.iter().zip(sig.iter()) {
                        if !typed_arg.resolved_type.is_assignable_to(expected_ty) {
                            matches = false;
                            break;
                        }
                    }
                    if matches {
                        return Ok(typed_args);
                    }
                }

                // No signature matched — generate helpful error
                let arg_count = typed_args.len();
                let expected_counts: Vec<usize> =
                    signatures.iter().map(|s| s.len()).collect();
                if !expected_counts.contains(&arg_count) {
                    Err(self.type_error(
                        span,
                        format!(
                            "'{}' expects {} arguments, found {}",
                            fn_name,
                            expected_counts
                                .iter()
                                .map(|c| c.to_string())
                                .collect::<Vec<_>>()
                                .join(" or "),
                            arg_count
                        ),
                    ))
                } else {
                    // Arg count matched at least one sig but types were wrong
                    let arg_types: Vec<String> = typed_args
                        .iter()
                        .map(|a| a.resolved_type.to_string())
                        .collect();
                    Err(self.type_error(
                        span,
                        format!(
                            "'{}' called with incompatible argument types: ({})",
                            fn_name,
                            arg_types.join(", ")
                        ),
                    ))
                }
            }
        }
    }

    fn check_method_call(
        &mut self,
        receiver: Expr,
        method: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_receiver = self.check_expr(receiver)?;
        let receiver_ty = typed_receiver.resolved_type.clone();

        match &receiver_ty {
            FluxType::List(elem_ty) => {
                let elem_type = elem_ty.as_ref().clone();
                match method {
                    "append" => {
                        if args.len() != 1 {
                            return Err(self.type_error(
                                span,
                                format!("'append' expects 1 argument, found {}", args.len()),
                            ));
                        }
                        let typed_arg = self.check_expr(args.into_iter().next().unwrap())?;
                        if !typed_arg.resolved_type.is_assignable_to(&elem_type) {
                            return Err(self.type_error(
                                typed_arg.span,
                                format!(
                                    "'append' argument must be {}, found {}",
                                    elem_type, typed_arg.resolved_type
                                ),
                            ));
                        }
                        Ok(TypedExpr {
                            kind: TypedExprKind::MethodCall {
                                receiver: Box::new(typed_receiver),
                                method: method.to_string(),
                                args: vec![typed_arg],
                            },
                            resolved_type: FluxType::Void,
                            span,
                        })
                    }
                    "len" => {
                        if !args.is_empty() {
                            return Err(self.type_error(
                                span,
                                format!("'len' expects 0 arguments, found {}", args.len()),
                            ));
                        }
                        Ok(TypedExpr {
                            kind: TypedExprKind::MethodCall {
                                receiver: Box::new(typed_receiver),
                                method: method.to_string(),
                                args: vec![],
                            },
                            resolved_type: FluxType::Int,
                            span,
                        })
                    }
                    "pop" => {
                        if !args.is_empty() {
                            return Err(self.type_error(
                                span,
                                format!("'pop' expects 0 arguments, found {}", args.len()),
                            ));
                        }
                        Ok(TypedExpr {
                            kind: TypedExprKind::MethodCall {
                                receiver: Box::new(typed_receiver),
                                method: method.to_string(),
                                args: vec![],
                            },
                            resolved_type: elem_type,
                            span,
                        })
                    }
                    _ => Err(self.type_error(
                        span,
                        format!(
                            "type {} does not have method '{}'",
                            receiver_ty, method
                        ),
                    )),
                }
            }
            FluxType::Struct(struct_name) => {
                // Look up methods registered via impl blocks
                // Clone the method info to avoid borrow conflicts with check_expr
                let method_info_opt = self.env.get_method(struct_name, method).cloned();
                if let Some(method_info) = method_info_opt {
                    let expected_params = method_info.param_types;
                    let return_type = method_info.return_type;
                    let is_static = method_info.is_static;

                    if is_static {
                        return Err(self.type_error(
                            span,
                            format!(
                                "Method '{}' on type '{}' is static and cannot be called on an instance",
                                method, struct_name
                            ),
                        ));
                    }

                    // Check argument count
                    if args.len() != expected_params.len() {
                        return Err(self.type_error(
                            span,
                            format!(
                                "Method '{}' on type '{}' expects {} arguments, found {}",
                                method, struct_name, expected_params.len(), args.len()
                            ),
                        ));
                    }

                    // Typecheck arguments
                    let mut typed_args = Vec::new();
                    for (arg, expected_ty) in args.into_iter().zip(expected_params.iter()) {
                        let typed_arg = self.check_expr(arg)?;
                        if !typed_arg.resolved_type.is_assignable_to(expected_ty) {
                            return Err(self.type_error(
                                typed_arg.span,
                                format!(
                                    "Method '{}' argument expects {}, found {}",
                                    method, expected_ty, typed_arg.resolved_type
                                ),
                            ));
                        }
                        typed_args.push(typed_arg);
                    }

                    Ok(TypedExpr {
                        kind: TypedExprKind::MethodCall {
                            receiver: Box::new(typed_receiver),
                            method: method.to_string(),
                            args: typed_args,
                        },
                        resolved_type: return_type,
                        span,
                    })
                } else {
                    // Not found in inherent impl — try trait impl methods
                    let trait_method_opt = self.env.get_trait_method(struct_name, method).cloned();
                    if let Some(trait_method_info) = trait_method_opt {
                        let expected_params = trait_method_info.param_types;
                        let return_type = trait_method_info.return_type;
                        let is_static = trait_method_info.is_static;

                        if is_static {
                            return Err(self.type_error(
                                span,
                                format!(
                                    "Method '{}' on type '{}' is static and cannot be called on an instance",
                                    method, struct_name
                                ),
                            ));
                        }

                        // Check argument count
                        if args.len() != expected_params.len() {
                            return Err(self.type_error(
                                span,
                                format!(
                                    "Method '{}' on type '{}' expects {} arguments, found {}",
                                    method, struct_name, expected_params.len(), args.len()
                                ),
                            ));
                        }

                        // Typecheck arguments
                        let mut typed_args = Vec::new();
                        for (arg, expected_ty) in args.into_iter().zip(expected_params.iter()) {
                            let typed_arg = self.check_expr(arg)?;
                            if !typed_arg.resolved_type.is_assignable_to(expected_ty) {
                                return Err(self.type_error(
                                    typed_arg.span,
                                    format!(
                                        "Method '{}' argument expects {}, found {}",
                                        method, expected_ty, typed_arg.resolved_type
                                    ),
                                ));
                            }
                            typed_args.push(typed_arg);
                        }

                        Ok(TypedExpr {
                            kind: TypedExprKind::MethodCall {
                                receiver: Box::new(typed_receiver),
                                method: method.to_string(),
                                args: typed_args,
                            },
                            resolved_type: return_type,
                            span,
                        })
                    } else {
                        Err(self.type_error(
                            span,
                            format!("No method '{}' on type '{}'", method, struct_name),
                        ))
                    }
                }
            }
            FluxType::Generic(type_name, type_args) if type_name == "HashMap" && type_args.len() == 2 => {
                let key_type = type_args[0].clone();
                let value_type = type_args[1].clone();
                self.check_hashmap_method_call(
                    typed_receiver, &key_type, &value_type, method, args, span,
                )
            }
            _ => Err(self.type_error(
                span,
                format!("type {} does not have method '{}'", receiver_ty, method),
            )),
        }
    }

    fn check_index_access(
        &mut self,
        object: Expr,
        index: Expr,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_object = self.check_expr(object)?;
        let typed_index = self.check_expr(index)?;

        // Determine element type based on receiver
        let elem_type = match &typed_object.resolved_type {
            FluxType::VecFloat => {
                // VecFloat index must be Int
                if typed_index.resolved_type != FluxType::Int {
                    return Err(self.type_error(
                        typed_index.span,
                        format!(
                            "VecFloat index must be Int, found {}",
                            typed_index.resolved_type
                        ),
                    ));
                }
                FluxType::Float
            }
            FluxType::List(t) => {
                // List index must be Int
                if typed_index.resolved_type != FluxType::Int {
                    return Err(self.type_error(
                        typed_index.span,
                        format!(
                            "index must be Int, found {}",
                            typed_index.resolved_type
                        ),
                    ));
                }
                t.as_ref().clone()
            }
            FluxType::FixedArray(elem_type, _size) => {
                // FixedArray index must be Int
                if typed_index.resolved_type != FluxType::Int {
                    return Err(self.type_error(
                        typed_index.span,
                        format!(
                            "index must be Int, found {}",
                            typed_index.resolved_type
                        ),
                    ));
                }
                elem_type.as_ref().clone()
            }
            other => {
                return Err(self.type_error(
                    span,
                    format!("type {} does not support indexing", other),
                ));
            }
        };

        Ok(TypedExpr {
            kind: TypedExprKind::IndexAccess {
                object: Box::new(typed_object),
                index: Box::new(typed_index),
            },
            resolved_type: elem_type,
            span,
        })
    }

    fn check_list_literal(&mut self, elements: Vec<Expr>, span: Span) -> Result<TypedExpr> {
        if elements.is_empty() {
            return Ok(TypedExpr {
                kind: TypedExprKind::ListLiteral(vec![]),
                resolved_type: FluxType::List(Box::new(FluxType::Null)),
                span,
            });
        }

        let mut typed_elements = Vec::new();
        for elem in elements {
            typed_elements.push(self.check_expr(elem)?);
        }

        // Infer element type
        let first_ty = typed_elements[0].resolved_type.clone();
        let mut all_same = true;
        let mut all_numeric = first_ty.is_numeric();

        for elem in typed_elements.iter().skip(1) {
            if elem.resolved_type != first_ty {
                all_same = false;
            }
            if !elem.resolved_type.is_numeric() {
                all_numeric = false;
            }
        }

        if all_numeric {
            // All elements are numeric (Int or Float) → infer VecFloat
            return Ok(TypedExpr {
                kind: TypedExprKind::ListLiteral(typed_elements),
                resolved_type: FluxType::VecFloat,
                span,
            });
        }

        // Check if some elements are numeric and others are not → type error
        // on the non-numeric element
        let has_any_numeric = typed_elements.iter().any(|e| e.resolved_type.is_numeric());
        if has_any_numeric {
            // Find the first non-numeric element and report the error with its span
            let (pos, offending) = typed_elements
                .iter()
                .enumerate()
                .find(|(_, e)| !e.resolved_type.is_numeric())
                .unwrap();
            return Err(self.type_error(
                offending.span,
                format!(
                    "list literal expected numeric element, found {} at position {}",
                    offending.resolved_type, pos
                ),
            ));
        }

        // All elements are non-numeric
        let elem_type = if all_same {
            // Homogeneous non-numeric list
            first_ty
        } else {
            // Incompatible non-numeric types
            let other_ty = typed_elements
                .iter()
                .skip(1)
                .find(|e| e.resolved_type != first_ty)
                .map(|e| &e.resolved_type)
                .unwrap();
            return Err(self.type_error(
                span,
                format!(
                    "list elements have incompatible types: {} and {}",
                    first_ty, other_ty
                ),
            ));
        };

        Ok(TypedExpr {
            kind: TypedExprKind::ListLiteral(typed_elements),
            resolved_type: FluxType::List(Box::new(elem_type)),
            span,
        })
    }

    fn check_member_access(
        &mut self,
        object: Expr,
        field: &str,
        span: Span,
    ) -> Result<TypedExpr> {
        let typed_object = self.check_expr(object)?;

        // Handle struct field access
        if let FluxType::Struct(ref struct_name) = typed_object.resolved_type {
            if let Some(struct_info) = self.struct_registry.get(struct_name).cloned() {
                // Look up the field by name
                if let Some((_field_name, field_type)) =
                    struct_info.fields.iter().find(|(name, _)| name == field)
                {
                    return Ok(TypedExpr {
                        kind: TypedExprKind::MemberAccess {
                            object: Box::new(typed_object),
                            field: field.to_string(),
                        },
                        resolved_type: field_type.clone(),
                        span,
                    });
                } else {
                    // Field doesn't exist — report error with available fields
                    let available: Vec<&str> = struct_info
                        .fields
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect();
                    return Err(self.type_error(
                        span,
                        format!(
                            "struct '{}' has no field '{}'. Available: {}",
                            struct_name,
                            field,
                            available.join(", ")
                        ),
                    ));
                }
            }
        }

        // Non-struct types don't support member access
        Err(self.type_error(
            span,
            format!(
                "type {} does not support member access '{}'",
                typed_object.resolved_type, field
            ),
        ))
    }

    /// Type-check a struct literal expression.
    ///
    /// Validates:
    /// 1. The struct name is defined in the struct registry
    /// 2. No unknown fields are provided
    /// 3. All declared fields are provided (no missing fields)
    /// 4. Each field value type matches the declared field type
    fn check_struct_literal(
        &mut self,
        struct_name: &str,
        fields: Vec<(String, Expr)>,
        span: Span,
    ) -> Result<TypedExpr> {
        // 1. Look up struct in registry
        let struct_info = match self.struct_registry.get(struct_name) {
            Some(info) => info.clone(),
            None => {
                let msg = if let Some(import_path) = Self::suggest_import_for_type(struct_name) {
                    format!(
                        "type '{}' is not defined. Did you mean 'from {} import {{{}}}'?",
                        struct_name, import_path, struct_name
                    )
                } else {
                    format!("unknown struct type '{}'", struct_name)
                };
                return Err(self.type_error(span, msg));
            }
        };

        // Build a lookup from declared field names to their types
        let declared_fields: HashMap<&str, &FluxType> = struct_info
            .fields
            .iter()
            .map(|(name, ty)| (name.as_str(), ty))
            .collect();

        // 2. Check for unknown fields (provided but not in definition)
        for (field_name, _) in &fields {
            if !declared_fields.contains_key(field_name.as_str()) {
                return Err(self.type_error(
                    span,
                    format!("struct '{}' has no field '{}'", struct_name, field_name),
                ));
            }
        }

        // 3. Check for missing fields (defined but not provided)
        let provided_names: HashSet<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        let missing: Vec<&str> = struct_info
            .fields
            .iter()
            .filter(|(name, _)| !provided_names.contains(name.as_str()))
            .map(|(name, _)| name.as_str())
            .collect();

        if !missing.is_empty() {
            return Err(self.type_error(
                span,
                format!(
                    "struct literal '{}' missing fields: {}",
                    struct_name,
                    missing.join(", ")
                ),
            ));
        }

        // 4. Type-check each provided field value
        let mut typed_fields = Vec::new();
        for (field_name, value_expr) in fields {
            let typed_value = self.check_expr(value_expr)?;
            let expected_type = declared_fields[field_name.as_str()];

            if !typed_value.resolved_type.is_assignable_to(expected_type) {
                return Err(self.type_error(
                    span,
                    format!(
                        "field '{}' expects {}, got {}",
                        field_name, expected_type, typed_value.resolved_type
                    ),
                ));
            }

            typed_fields.push((field_name, typed_value));
        }

        // Return the struct type
        Ok(TypedExpr {
            kind: TypedExprKind::StructLiteral {
                struct_name: struct_name.to_string(),
                fields: typed_fields,
            },
            resolved_type: FluxType::Struct(struct_name.to_string()),
            span,
        })
    }

    /// Typecheck a match expression.
    ///
    /// Validates:
    /// - The scrutinee type is an enum
    /// - All patterns reference valid variants of the scrutinee's enum type
    /// - Binding counts match variant field counts
    /// - Pattern-bound variables are introduced with correct types into arm body scopes
    /// - All arms produce compatible result types
    ///
    /// Note: Exhaustiveness checking validates all variants are covered or wildcard present.
    /// Requirements: 3.5, 3.6, 3.7, 3.8, 13.3
    fn check_match_expr(
        &mut self,
        match_expr: MatchExpr,
        span: Span,
    ) -> Result<TypedExpr> {
        use super::exhaustiveness::{check_exhaustiveness, ExhaustivenessResult};

        // 1. Typecheck the scrutinee expression
        let typed_scrutinee = self.check_expr(*match_expr.scrutinee)?;

        // 2. Verify scrutinee type is an enum
        let enum_name = match &typed_scrutinee.resolved_type {
            FluxType::Enum(name) => name.clone(),
            other => {
                return Err(self.type_error(
                    typed_scrutinee.span,
                    format!("match scrutinee must be an enum type, got {}", other),
                ));
            }
        };

        // 3. Look up the enum info
        let enum_info = match self.env.get_enum(&enum_name) {
            Some(info) => info.clone(),
            None => {
                return Err(self.type_error(
                    span,
                    format!("unknown enum type '{}' in match expression", enum_name),
                ));
            }
        };

        // 4. Collect patterns for exhaustiveness check
        let patterns: Vec<_> = match_expr.arms.iter().map(|a| a.pattern.clone()).collect();

        // 5. Typecheck each arm
        let mut typed_arms = Vec::new();
        let mut result_type: Option<FluxType> = None;

        for arm in match_expr.arms {
            let (typed_pattern, arm_bindings) = self.check_match_pattern(
                &arm.pattern,
                &enum_name,
                &enum_info,
            )?;

            // Push a new scope for the arm body and add pattern bindings
            self.env.push_scope();
            for (binding_name, binding_type) in &arm_bindings {
                self.env.insert(binding_name.clone(), binding_type.clone());
            }

            // Typecheck arm body statements
            let mut typed_body = Vec::new();
            for stmt in arm.body {
                typed_body.push(self.check_stmt(stmt)?);
            }

            self.env.pop_scope();

            // Infer result type from the arm body (last expression or Null)
            let arm_result_type = self.infer_arm_result_type(&typed_body);

            // Unify arm result types
            match &result_type {
                None => {
                    result_type = Some(arm_result_type);
                }
                Some(existing) => {
                    // Allow compatible types (e.g., Int/Float coercion)
                    if !arm_result_type.is_assignable_to(existing)
                        && !existing.is_assignable_to(&arm_result_type)
                    {
                        // If types differ but aren't compatible, use Null as the common type
                        result_type = Some(FluxType::Null);
                    }
                }
            }

            typed_arms.push(TypedMatchArm {
                pattern: typed_pattern,
                body: typed_body,
                span: arm.span,
            });
        }

        // 6. Exhaustiveness check
        match check_exhaustiveness(&enum_info, &patterns) {
            ExhaustivenessResult::Exhaustive => {}
            ExhaustivenessResult::NonExhaustive { missing_variants } => {
                let variants_list = missing_variants.join(", ");
                return Err(self.type_error(
                    span,
                    format!(
                        "non-exhaustive match on '{}': missing variant{} {}",
                        enum_name,
                        if missing_variants.len() == 1 { "" } else { "s" },
                        variants_list,
                    ),
                ));
            }
        }

        let final_result_type = result_type.unwrap_or(FluxType::Null);

        Ok(TypedExpr {
            kind: TypedExprKind::Match(TypedMatchExpr {
                scrutinee: Box::new(typed_scrutinee),
                arms: typed_arms,
                result_type: final_result_type.clone(),
                span,
            }),
            resolved_type: final_result_type,
            span,
        })
    }

    /// Typecheck a match pattern and return the typed pattern with any bindings introduced.
    fn check_match_pattern(
        &self,
        pattern: &Pattern,
        enum_name: &str,
        enum_info: &EnumInfo,
    ) -> Result<(TypedPattern, Vec<(String, FluxType)>)> {
        match pattern {
            Pattern::Variant {
                enum_name: pat_enum_name,
                variant_name,
                bindings,
                span,
            } => {
                // Verify the enum name matches the scrutinee type
                if pat_enum_name != enum_name {
                    return Err(self.type_error(
                        *span,
                        format!(
                            "pattern references enum '{}' but scrutinee is of type '{}'",
                            pat_enum_name, enum_name
                        ),
                    ));
                }

                // Look up the variant
                let variant_info = match enum_info.find_variant(variant_name) {
                    Some(info) => info,
                    None => {
                        return Err(self.type_error(
                            *span,
                            format!(
                                "enum '{}' has no variant '{}'",
                                enum_name, variant_name
                            ),
                        ));
                    }
                };

                // Verify binding count matches field count
                let expected_count = variant_info.field_count();
                let actual_count = bindings.len();
                if actual_count != expected_count {
                    return Err(self.type_error(
                        *span,
                        format!(
                            "variant '{}.{}' has {} field{}, but pattern binds {}",
                            enum_name,
                            variant_name,
                            expected_count,
                            if expected_count == 1 { "" } else { "s" },
                            actual_count
                        ),
                    ));
                }

                // Build typed bindings: pair each binding name with the corresponding field type
                let typed_bindings: Vec<(String, FluxType)> = bindings
                    .iter()
                    .zip(variant_info.fields.iter())
                    .map(|(name, (_, field_type))| (name.clone(), field_type.clone()))
                    .collect();

                let typed_pattern = TypedPattern::Variant {
                    enum_name: enum_name.to_string(),
                    variant_name: variant_name.clone(),
                    bindings: typed_bindings.clone(),
                    span: *span,
                };

                Ok((typed_pattern, typed_bindings))
            }
            Pattern::Wildcard { span } => {
                let typed_pattern = TypedPattern::Wildcard { span: *span };
                Ok((typed_pattern, Vec::new()))
            }
        }
    }

    /// Infer the result type of a match arm body.
    ///
    /// If the body ends with an expression statement, use that expression's type.
    /// Otherwise, the arm produces Null.
    fn infer_arm_result_type(&self, body: &[TypedStmt]) -> FluxType {
        if let Some(last_stmt) = body.last() {
            match last_stmt {
                TypedStmt::Expr(expr_stmt) => expr_stmt.expr.resolved_type.clone(),
                TypedStmt::Return(ret) => {
                    ret.value.as_ref().map_or(FluxType::Null, |v| v.resolved_type.clone())
                }
                _ => FluxType::Null,
            }
        } else {
            FluxType::Null
        }
    }

    /// Typecheck an enum construction expression.
    ///
    /// Validates:
    /// 1. The enum name exists in the type environment
    /// 2. The variant name exists within that enum
    /// 3. The number of arguments matches the variant's field count
    /// 4. Each argument type matches the corresponding field type
    ///
    /// Requirements: 2.3, 2.4, 2.5
    fn check_enum_construction(
        &mut self,
        enum_name: &str,
        variant_name: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<TypedExpr> {
        // Handle HashMap.new() as a built-in generic static method call
        if enum_name == "HashMap" {
            return self.check_hashmap_static_call(variant_name, args, span);
        }

        // 1. Look up the enum in the type environment
        let enum_info = match self.env.get_enum(enum_name) {
            Some(info) => info.clone(),
            None => {
                return Err(self.type_error(
                    span,
                    format!("unknown enum type '{}'", enum_name),
                ));
            }
        };

        // 2. Look up the variant within the enum
        let variant_info = match enum_info.find_variant(variant_name) {
            Some(info) => info.clone(),
            None => {
                return Err(self.type_error(
                    span,
                    format!(
                        "enum '{}' has no variant '{}'",
                        enum_name, variant_name
                    ),
                ));
            }
        };

        // 3. Check argument count matches field count
        let expected_count = variant_info.field_count();
        let actual_count = args.len();
        if actual_count != expected_count {
            return Err(self.type_error(
                span,
                format!(
                    "variant '{}.{}' expects {} argument{}, got {}",
                    enum_name,
                    variant_name,
                    expected_count,
                    if expected_count == 1 { "" } else { "s" },
                    actual_count
                ),
            ));
        }

        // 4. Typecheck each argument and verify it matches the field type
        let mut typed_args = Vec::new();
        for (i, arg_expr) in args.into_iter().enumerate() {
            let typed_arg = self.check_expr(arg_expr)?;
            let (field_name, expected_type) = &variant_info.fields[i];

            if !typed_arg.resolved_type.is_assignable_to(expected_type) {
                return Err(self.type_error(
                    span,
                    format!(
                        "field '{}' of '{}.{}' expects {}, got {}",
                        field_name, enum_name, variant_name, expected_type, typed_arg.resolved_type
                    ),
                ));
            }

            typed_args.push(typed_arg);
        }

        // Return the enum type
        Ok(TypedExpr {
            kind: TypedExprKind::EnumConstruction {
                enum_name: enum_name.to_string(),
                variant_name: variant_name.to_string(),
                args: typed_args,
            },
            resolved_type: FluxType::Enum(enum_name.to_string()),
            span,
        })
    }

    // -----------------------------------------------------------------------
    // HashMap built-in generic type support
    // -----------------------------------------------------------------------

    /// Handle static method calls on `HashMap` (e.g., `HashMap.new()`).
    fn check_hashmap_static_call(
        &mut self,
        method_name: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<TypedExpr> {
        match method_name {
            "new" => {
                if !args.is_empty() {
                    return Err(self.type_error(
                        span,
                        format!("'HashMap.new' expects 0 arguments, found {}", args.len()),
                    ));
                }
                // Return HashMap[K, V] with unresolved type params
                let hashmap_type = FluxType::Generic(
                    "HashMap".to_string(),
                    vec![FluxType::TypeParam("K".to_string()), FluxType::TypeParam("V".to_string())],
                );
                Ok(TypedExpr {
                    kind: TypedExprKind::MethodCall {
                        receiver: Box::new(TypedExpr {
                            kind: TypedExprKind::Ident("HashMap".to_string()),
                            resolved_type: FluxType::Void, // type namespace, not a value
                            span,
                        }),
                        method: "new".to_string(),
                        args: vec![],
                    },
                    resolved_type: hashmap_type,
                    span,
                })
            }
            _ => Err(self.type_error(
                span,
                format!("'HashMap' has no static method '{}'", method_name),
            )),
        }
    }

    /// Handle method calls on a `HashMap[K, V]` receiver.
    ///
    /// Supported methods:
    /// - `insert(key: K, value: V) -> HashMap[K, V]`
    /// - `get(key: K) -> V`
    /// - `contains_key(key: K) -> Bool`
    /// - `remove(key: K) -> HashMap[K, V]`
    fn check_hashmap_method_call(
        &mut self,
        typed_receiver: TypedExpr,
        key_type: &FluxType,
        value_type: &FluxType,
        method: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<TypedExpr> {
        let receiver_ty = typed_receiver.resolved_type.clone();

        // Extract the receiver variable name for type refinement
        let receiver_var_name = match &typed_receiver.kind {
            TypedExprKind::Ident(name) => Some(name.clone()),
            _ => None,
        };

        match method {
            "insert" => {
                if args.len() != 2 {
                    return Err(self.type_error(
                        span,
                        format!("'insert' expects 2 arguments (key, value), found {}", args.len()),
                    ));
                }
                let mut args_iter = args.into_iter();
                let typed_key = self.check_expr(args_iter.next().unwrap())?;
                let typed_value = self.check_expr(args_iter.next().unwrap())?;

                // Infer or validate key type
                let resolved_key = self.resolve_hashmap_type_param(
                    key_type, &typed_key.resolved_type, "key", "insert", span,
                )?;
                // Infer or validate value type
                let resolved_value = self.resolve_hashmap_type_param(
                    value_type, &typed_value.resolved_type, "value", "insert", span,
                )?;

                let result_type = FluxType::Generic(
                    "HashMap".to_string(),
                    vec![resolved_key.clone(), resolved_value.clone()],
                );

                // Refine the receiver variable type if it had unresolved TypeParams
                if let Some(ref var_name) = receiver_var_name {
                    if matches!(key_type, FluxType::TypeParam(_)) || matches!(value_type, FluxType::TypeParam(_)) {
                        self.env.insert(var_name.clone(), result_type.clone());
                    }
                }

                Ok(TypedExpr {
                    kind: TypedExprKind::MethodCall {
                        receiver: Box::new(typed_receiver),
                        method: "insert".to_string(),
                        args: vec![typed_key, typed_value],
                    },
                    resolved_type: result_type,
                    span,
                })
            }
            "get" => {
                if args.len() != 1 {
                    return Err(self.type_error(
                        span,
                        format!("'get' expects 1 argument (key), found {}", args.len()),
                    ));
                }
                let typed_key = self.check_expr(args.into_iter().next().unwrap())?;

                // Validate key type
                self.resolve_hashmap_type_param(
                    key_type, &typed_key.resolved_type, "key", "get", span,
                )?;

                // Return the value type
                let result_type = value_type.clone();

                Ok(TypedExpr {
                    kind: TypedExprKind::MethodCall {
                        receiver: Box::new(typed_receiver),
                        method: "get".to_string(),
                        args: vec![typed_key],
                    },
                    resolved_type: result_type,
                    span,
                })
            }
            "contains_key" => {
                if args.len() != 1 {
                    return Err(self.type_error(
                        span,
                        format!("'contains_key' expects 1 argument (key), found {}", args.len()),
                    ));
                }
                let typed_key = self.check_expr(args.into_iter().next().unwrap())?;

                // Validate key type
                self.resolve_hashmap_type_param(
                    key_type, &typed_key.resolved_type, "key", "contains_key", span,
                )?;

                Ok(TypedExpr {
                    kind: TypedExprKind::MethodCall {
                        receiver: Box::new(typed_receiver),
                        method: "contains_key".to_string(),
                        args: vec![typed_key],
                    },
                    resolved_type: FluxType::Bool,
                    span,
                })
            }
            "remove" => {
                if args.len() != 1 {
                    return Err(self.type_error(
                        span,
                        format!("'remove' expects 1 argument (key), found {}", args.len()),
                    ));
                }
                let typed_key = self.check_expr(args.into_iter().next().unwrap())?;

                // Validate key type
                self.resolve_hashmap_type_param(
                    key_type, &typed_key.resolved_type, "key", "remove", span,
                )?;

                Ok(TypedExpr {
                    kind: TypedExprKind::MethodCall {
                        receiver: Box::new(typed_receiver),
                        method: "remove".to_string(),
                        args: vec![typed_key],
                    },
                    resolved_type: receiver_ty,
                    span,
                })
            }
            _ => Err(self.type_error(
                span,
                format!("type HashMap does not have method '{}'", method),
            )),
        }
    }

    /// Resolve a HashMap type parameter: if the declared param is a TypeParam (unresolved),
    /// accept any concrete type. If it's concrete, validate the actual type matches.
    fn resolve_hashmap_type_param(
        &self,
        declared: &FluxType,
        actual: &FluxType,
        param_name: &str,
        method_name: &str,
        span: Span,
    ) -> Result<FluxType> {
        match declared {
            FluxType::TypeParam(_) => {
                // Unresolved: infer from the actual argument type
                Ok(actual.clone())
            }
            _ => {
                // Concrete: validate the argument matches
                if !actual.is_assignable_to(declared) {
                    return Err(self.type_error(
                        span,
                        format!(
                            "HashMap '{}' {} type mismatch: expected {}, found {}",
                            method_name, param_name, declared, actual
                        ),
                    ));
                }
                Ok(declared.clone())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Attempt to resolve the struct type name from an expression (for @immutable checking).
    /// Returns the struct name if the expression is an identifier with a struct type.
    fn resolve_struct_type_from_expr(&self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(name) => {
                if let Some(ty) = self.env.resolve(name) {
                    if let FluxType::Struct(struct_name) = ty {
                        return Some(struct_name.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Construct a `CompileError::Type` with a consistent format:
    /// `"at byte {span.start}: {description}"`
    fn type_error(&self, span: Span, message: String) -> CompileError {
        CompileError::Type(format!("at byte {}: {}", span.start, message))
    }

    /// Produce a recursion detection error from a cycle path.
    ///
    /// For direct recursion (cycle length 2, e.g. ["foo", "foo"]):
    ///   "at byte N: recursive call detected: 'foo' calls itself"
    ///
    /// For mutual recursion (cycle length > 2, e.g. ["foo", "bar", "foo"]):
    ///   "at byte N: recursive call detected: cycle 'foo' → 'bar' → 'foo'"
    fn recursion_error(&self, functions: &[FnDef], cycle: &[String]) -> CompileError {
        // Find the span of the first function in the cycle
        let first_name = &cycle[0];
        let span = functions
            .iter()
            .find(|f| &f.name == first_name)
            .map(|f| f.span)
            .unwrap_or(Span::new(0, 0));

        let message = if cycle.len() == 2 && cycle[0] == cycle[1] {
            // Direct self-recursion
            format!("recursive call detected: '{}' calls itself", cycle[0])
        } else {
            // Mutual recursion — format the cycle path
            let path = cycle
                .iter()
                .map(|name| format!("'{}'", name))
                .collect::<Vec<_>>()
                .join(" → ");
            format!("recursive call detected: cycle {}", path)
        };

        self.type_error(span, message)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;
    use crate::parser::ast::{
        Assignment, BinOp, ElifBranch, Expr, ExprKind, ForLoop, IfStmt, Stmt, UnaryOp, WhileLoop,
    };

    fn make_expr(kind: ExprKind) -> Expr {
        Expr { kind, span: Span::new(0, 1) }
    }

    // -----------------------------------------------------------------------
    // 1. Literals
    // -----------------------------------------------------------------------

    #[test]
    fn test_literal_int() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::IntLiteral(42))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_literal_float() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::FloatLiteral(3.14))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_literal_string() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::StringLiteral("hello".to_string()))).unwrap();
        assert_eq!(result.resolved_type, FluxType::String);
    }

    #[test]
    fn test_literal_bool() {
        let mut tc = TypeChecker::new();
        let result = tc.check_expr(make_expr(ExprKind::BoolLiteral(true))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    // -----------------------------------------------------------------------
    // 2. Identifier resolution
    // -----------------------------------------------------------------------

    #[test]
    fn test_ident_resolved() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let result = tc.check_expr(make_expr(ExprKind::Ident("x".to_string()))).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_ident_undefined() {
        let mut tc = TypeChecker::new();
        let err = tc.check_expr(make_expr(ExprKind::Ident("unknown".to_string()))).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undefined identifier 'unknown'"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 3. Binary ops - arithmetic
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_int_int() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::IntLiteral(2))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_add_float_float() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::FloatLiteral(1.0))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::FloatLiteral(2.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_add_int_float() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::FloatLiteral(2.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_add_string_string() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("hello".to_string()))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::StringLiteral(" world".to_string()))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::String);
    }

    #[test]
    fn test_add_string_int_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("hello".to_string()))),
            op: BinOp::Add,
            right: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("numeric operands") || msg.contains("String") && msg.contains("Int"),
            "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 4. Binary ops - comparison
    // -----------------------------------------------------------------------

    #[test]
    fn test_lt_numeric() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Lt,
            right: Box::new(make_expr(ExprKind::FloatLiteral(2.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_lt_non_numeric_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("a".to_string()))),
            op: BinOp::Lt,
            right: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("numeric operands"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 5. Binary ops - equality
    // -----------------------------------------------------------------------

    #[test]
    fn test_eq_same_type() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Eq,
            right: Box::new(make_expr(ExprKind::IntLiteral(2))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_eq_numeric_cross() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Eq,
            right: Box::new(make_expr(ExprKind::FloatLiteral(1.0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_eq_incompatible_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::StringLiteral("a".to_string()))),
            op: BinOp::Eq,
            right: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("matching types"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 6. Binary ops - logical
    // -----------------------------------------------------------------------

    #[test]
    fn test_and_bool_bool() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::BoolLiteral(true))),
            op: BinOp::And,
            right: Box::new(make_expr(ExprKind::BoolLiteral(false))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_or_non_bool_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::BinaryOp {
            left: Box::new(make_expr(ExprKind::IntLiteral(1))),
            op: BinOp::Or,
            right: Box::new(make_expr(ExprKind::BoolLiteral(true))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("boolean operands"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 7. Unary ops
    // -----------------------------------------------------------------------

    #[test]
    fn test_neg_int() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Neg,
            operand: Box::new(make_expr(ExprKind::IntLiteral(5))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_neg_string_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Neg,
            operand: Box::new(make_expr(ExprKind::StringLiteral("x".to_string()))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("numeric operand"), "got: {}", msg);
    }

    #[test]
    fn test_not_bool() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(make_expr(ExprKind::BoolLiteral(true))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Bool);
    }

    #[test]
    fn test_not_int_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(make_expr(ExprKind::IntLiteral(1))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("boolean operand"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 8. Function calls
    // -----------------------------------------------------------------------

    #[test]
    fn test_call_variadic_numeric() {
        let mut tc = TypeChecker::new();
        tc.env.insert("sma".to_string(), FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Float),
        });
        let expr = make_expr(ExprKind::FunctionCall {
            function: Box::new(make_expr(ExprKind::Ident("sma".to_string()))),
            args: vec![
                make_expr(ExprKind::IntLiteral(10)),
                make_expr(ExprKind::IntLiteral(20)),
            ],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Float);
    }

    #[test]
    fn test_call_not_callable() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let expr = make_expr(ExprKind::FunctionCall {
            function: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
            args: vec![],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not a function"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 9. Method calls
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_len() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            method: "len".to_string(),
            args: vec![],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_list_append() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            method: "append".to_string(),
            args: vec![make_expr(ExprKind::IntLiteral(42))],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Void);
    }

    #[test]
    fn test_list_pop() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            method: "pop".to_string(),
            args: vec![],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_invalid_method_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let expr = make_expr(ExprKind::MethodCall {
            receiver: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
            method: "len".to_string(),
            args: vec![],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does not have method"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 10. Index access
    // -----------------------------------------------------------------------

    #[test]
    fn test_index_list_int() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::IndexAccess {
            object: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            index: Box::new(make_expr(ExprKind::IntLiteral(0))),
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Int);
    }

    #[test]
    fn test_index_non_list_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let expr = make_expr(ExprKind::IndexAccess {
            object: Box::new(make_expr(ExprKind::Ident("x".to_string()))),
            index: Box::new(make_expr(ExprKind::IntLiteral(0))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does not support indexing"), "got: {}", msg);
    }

    #[test]
    fn test_index_non_int_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("arr".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let expr = make_expr(ExprKind::IndexAccess {
            object: Box::new(make_expr(ExprKind::Ident("arr".to_string()))),
            index: Box::new(make_expr(ExprKind::StringLiteral("x".to_string()))),
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("index must be Int"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 11. List literals
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_list() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![]));
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::List(Box::new(FluxType::Null)));
    }

    #[test]
    fn test_homogeneous_list() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![
            make_expr(ExprKind::IntLiteral(1)),
            make_expr(ExprKind::IntLiteral(2)),
            make_expr(ExprKind::IntLiteral(3)),
        ]));
        let result = tc.check_expr(expr).unwrap();
        // All-numeric lists now infer VecFloat
        assert_eq!(result.resolved_type, FluxType::VecFloat);
    }

    #[test]
    fn test_mixed_numeric_list() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![
            make_expr(ExprKind::IntLiteral(1)),
            make_expr(ExprKind::FloatLiteral(2.0)),
        ]));
        let result = tc.check_expr(expr).unwrap();
        // Mixed Int/Float lists also infer VecFloat
        assert_eq!(result.resolved_type, FluxType::VecFloat);
    }

    #[test]
    fn test_incompatible_list_error() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::ListLiteral(vec![
            make_expr(ExprKind::IntLiteral(1)),
            make_expr(ExprKind::StringLiteral("hello".to_string())),
        ]));
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        // Numeric + non-numeric → error on the non-numeric element
        assert!(msg.contains("list literal expected numeric element, found String at position 1"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 12. Statements - assignment
    // -----------------------------------------------------------------------

    fn make_stmt_assignment(target_name: &str, value: ExprKind) -> Stmt {
        Stmt::Assignment(Assignment {
            target: Expr { kind: ExprKind::Ident(target_name.to_string()), span: Span::new(0, 1) },
            value: Expr { kind: value, span: Span::new(2, 3) },
            span: Span::new(0, 3),
        })
    }

    #[test]
    fn test_assignment_new_variable() {
        let mut tc = TypeChecker::new();
        let stmt = make_stmt_assignment("x", ExprKind::IntLiteral(42));
        tc.check_stmt(stmt).unwrap();
        // New variable should now be in scope
        assert_eq!(tc.env.resolve("x"), Some(&FluxType::Int));
    }

    #[test]
    fn test_assignment_existing_variable_ok() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let stmt = make_stmt_assignment("x", ExprKind::IntLiteral(99));
        tc.check_stmt(stmt).unwrap();
        // Variable still exists with same type
        assert_eq!(tc.env.resolve("x"), Some(&FluxType::Int));
    }

    #[test]
    fn test_assignment_type_mismatch() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Int);
        let stmt = make_stmt_assignment("x", ExprKind::StringLiteral("hello".to_string()));
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot assign") && msg.contains("String") && msg.contains("Int"),
            "got: {}", msg);
    }

    #[test]
    fn test_assignment_int_to_float_coercion() {
        let mut tc = TypeChecker::new();
        tc.env.insert("x".to_string(), FluxType::Float);
        let stmt = make_stmt_assignment("x", ExprKind::IntLiteral(5));
        // Int is assignable to Float (coercion)
        tc.check_stmt(stmt).unwrap();
        assert_eq!(tc.env.resolve("x"), Some(&FluxType::Float));
    }

    // -----------------------------------------------------------------------
    // 13. Statements - if/elif/else
    // -----------------------------------------------------------------------

    #[test]
    fn test_if_bool_condition() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 10),
        });
        tc.check_stmt(stmt).unwrap();
    }

    #[test]
    fn test_if_non_bool_condition_error() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::IntLiteral(1), span: Span::new(0, 1) },
            body: vec![],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 10),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be Bool"), "got: {}", msg);
    }

    #[test]
    fn test_elif_non_bool_condition_error() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![],
            elif_branches: vec![
                ElifBranch {
                    condition: Expr { kind: ExprKind::IntLiteral(0), span: Span::new(10, 11) },
                    body: vec![],
                    span: Span::new(10, 20),
                },
            ],
            else_body: None,
            span: Span::new(0, 20),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be Bool"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 14. Statements - for loop
    // -----------------------------------------------------------------------

    #[test]
    fn test_for_list_iterable() {
        let mut tc = TypeChecker::new();
        tc.env.insert("items".to_string(), FluxType::List(Box::new(FluxType::Int)));
        let stmt = Stmt::For(ForLoop {
            variable: "item".to_string(),
            iterable: Expr { kind: ExprKind::Ident("items".to_string()), span: Span::new(5, 10) },
            body: vec![],
            span: Span::new(0, 20),
        });
        tc.check_stmt(stmt).unwrap();
    }

    #[test]
    fn test_for_non_list_error() {
        let mut tc = TypeChecker::new();
        tc.env.insert("count".to_string(), FluxType::Int);
        let stmt = Stmt::For(ForLoop {
            variable: "item".to_string(),
            iterable: Expr { kind: ExprKind::Ident("count".to_string()), span: Span::new(5, 10) },
            body: vec![],
            span: Span::new(0, 20),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("requires List type"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 15. Statements - while loop
    // -----------------------------------------------------------------------

    #[test]
    fn test_while_bool_condition() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::While(WhileLoop {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![],
            span: Span::new(0, 10),
        });
        tc.check_stmt(stmt).unwrap();
    }

    #[test]
    fn test_while_non_bool_error() {
        let mut tc = TypeChecker::new();
        let stmt = Stmt::While(WhileLoop {
            condition: Expr { kind: ExprKind::IntLiteral(1), span: Span::new(0, 1) },
            body: vec![],
            span: Span::new(0, 10),
        });
        let err = tc.check_stmt(stmt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be Bool"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // 16. Scope isolation
    // -----------------------------------------------------------------------

    #[test]
    fn test_scope_isolation_if() {
        let mut tc = TypeChecker::new();
        // Create an if statement that declares a variable inside the body
        let stmt = Stmt::If(IfStmt {
            condition: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(0, 4) },
            body: vec![
                make_stmt_assignment("inner_var", ExprKind::IntLiteral(10)),
            ],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 30),
        });
        tc.check_stmt(stmt).unwrap();
        // Variable declared inside if body should NOT be accessible after
        assert_eq!(tc.env.resolve("inner_var"), None);
    }

    // -----------------------------------------------------------------------
    // 17. Top-level program checking
    // -----------------------------------------------------------------------

    use crate::parser::ast::{
        Program, Strategy, StrategyItem, Import, ParamsBlock, Param,
        StateBlock, StateVar, EventHandler, ExprStmt, Property,
    };

    fn make_program(imports: Vec<Import>, body: Vec<StrategyItem>) -> Program {
        Program {
            structs: vec![], enums: vec![],
            imports,
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: Strategy {
                name: "Test".to_string(),
                body,
                span: Span::new(0, 100),
            },
            span: Span::new(0, 100),
        }
    }

    #[test]
    fn test_check_program_minimal() {
        let mut tc = TypeChecker::new();
        let program = make_program(vec![], vec![]);
        let result = tc.check_program(program);
        assert!(result.is_ok(), "minimal program should type-check: {:?}", result.err());
    }

    #[test]
    fn test_import_registration() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![Import {
                module_path: "indicators".to_string(),
                names: vec!["sma".to_string()],
                span: Span::new(0, 20),
            }],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("sma".to_string()),
                                span: Span::new(30, 33),
                            }),
                            args: vec![Expr {
                                kind: ExprKind::IntLiteral(20),
                                span: Span::new(34, 36),
                            }],
                        },
                        span: Span::new(30, 37),
                    },
                    span: Span::new(30, 37),
                })],
                span: Span::new(25, 50),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "imported fn should be callable: {:?}", result.err());
    }

    #[test]
    fn test_duplicate_import_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![
                Import {
                    module_path: "indicators".to_string(),
                    names: vec!["sma".to_string()],
                    span: Span::new(0, 20),
                },
                Import {
                    module_path: "indicators".to_string(),
                    names: vec!["sma".to_string()],
                    span: Span::new(21, 40),
                },
            ],
            vec![],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate import"), "got: {}", msg);
    }

    #[test]
    fn test_params_literal_defaults() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::ParamsBlock(ParamsBlock {
                params: vec![
                    Param {
                        name: "period".to_string(),
                        default_value: Expr { kind: ExprKind::IntLiteral(20), span: Span::new(10, 12) },
                        span: Span::new(5, 12),
                    },
                    Param {
                        name: "threshold".to_string(),
                        default_value: Expr { kind: ExprKind::FloatLiteral(2.0), span: Span::new(15, 18) },
                        span: Span::new(13, 18),
                    },
                    Param {
                        name: "name".to_string(),
                        default_value: Expr { kind: ExprKind::StringLiteral("test".to_string()), span: Span::new(20, 26) },
                        span: Span::new(19, 26),
                    },
                    Param {
                        name: "enabled".to_string(),
                        default_value: Expr { kind: ExprKind::BoolLiteral(true), span: Span::new(28, 32) },
                        span: Span::new(27, 32),
                    },
                ],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "params with literal defaults should pass: {:?}", result.err());
    }

    #[test]
    fn test_params_non_literal_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::ParamsBlock(ParamsBlock {
                params: vec![Param {
                    name: "bad".to_string(),
                    default_value: Expr {
                        kind: ExprKind::BinaryOp {
                            left: Box::new(Expr { kind: ExprKind::IntLiteral(1), span: Span::new(10, 11) }),
                            op: BinOp::Add,
                            right: Box::new(Expr { kind: ExprKind::IntLiteral(2), span: Span::new(14, 15) }),
                        },
                        span: Span::new(10, 15),
                    },
                    span: Span::new(5, 15),
                }],
                span: Span::new(0, 20),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be a literal"), "got: {}", msg);
    }

    #[test]
    fn test_state_literal_init() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::StateBlock(StateBlock {
                variables: vec![StateVar {
                    name: "count".to_string(),
                    initial_value: Expr { kind: ExprKind::IntLiteral(0), span: Span::new(10, 11) },
                    span: Span::new(5, 11),
                }],
                span: Span::new(0, 20),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "state with literal init should pass: {:?}", result.err());
    }

    #[test]
    fn test_state_list_init() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::StateBlock(StateBlock {
                variables: vec![StateVar {
                    name: "prices".to_string(),
                    initial_value: Expr { kind: ExprKind::ListLiteral(vec![]), span: Span::new(10, 12) },
                    span: Span::new(5, 12),
                }],
                span: Span::new(0, 20),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "state with [] init should pass: {:?}", result.err());
    }

    #[test]
    fn test_state_undefined_ident_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::StateBlock(StateBlock {
                variables: vec![StateVar {
                    name: "x".to_string(),
                    initial_value: Expr {
                        kind: ExprKind::Ident("undefined_var".to_string()),
                        span: Span::new(10, 23),
                    },
                    span: Span::new(5, 23),
                }],
                span: Span::new(0, 30),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undefined identifier"), "got: {}", msg);
    }

    #[test]
    fn test_event_handler_valid() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![],
                span: Span::new(0, 20),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "on_bar handler should be valid: {:?}", result.err());
    }

    #[test]
    fn test_event_handler_invalid_name() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "tick".to_string(),
                body: vec![],
                span: Span::new(0, 20),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unrecognized event handler"), "got: {}", msg);
    }

    #[test]
    fn test_market_data_inside_handler() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::Ident("close".to_string()),
                        span: Span::new(10, 15),
                    },
                    span: Span::new(10, 15),
                })],
                span: Span::new(0, 30),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "close should be accessible inside handler: {:?}", result.err());
    }

    #[test]
    fn test_market_data_outside_handler_error() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::Property(Property {
                name: "value".to_string(),
                value: Expr {
                    kind: ExprKind::Ident("close".to_string()),
                    span: Span::new(10, 15),
                },
                span: Span::new(5, 15),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("only available inside event handlers"), "got: {}", msg);
    }

    #[test]
    fn test_signal_open_valid() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("OPEN".to_string()),
                                span: Span::new(10, 14),
                            }),
                            args: vec![
                                Expr {
                                    kind: ExprKind::Ident("symbol".to_string()),
                                    span: Span::new(15, 21),
                                },
                                Expr {
                                    kind: ExprKind::IntLiteral(100),
                                    span: Span::new(23, 26),
                                },
                            ],
                        },
                        span: Span::new(10, 27),
                    },
                    span: Span::new(10, 27),
                })],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "OPEN(symbol, 100) should be valid: {:?}", result.err());
    }

    #[test]
    fn test_signal_open_wrong_args() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("OPEN".to_string()),
                                span: Span::new(10, 14),
                            }),
                            args: vec![
                                Expr {
                                    kind: ExprKind::IntLiteral(100),
                                    span: Span::new(15, 18),
                                },
                                Expr {
                                    kind: ExprKind::StringLiteral("hi".to_string()),
                                    span: Span::new(20, 24),
                                },
                            ],
                        },
                        span: Span::new(10, 25),
                    },
                    span: Span::new(10, 25),
                })],
                span: Span::new(0, 40),
            })],
        );
        let err = tc.check_program(program).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("argument") || msg.contains("OPEN"), "got: {}", msg);
    }

    #[test]
    fn test_signal_close_one_arg() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("CLOSE".to_string()),
                                span: Span::new(10, 15),
                            }),
                            args: vec![Expr {
                                kind: ExprKind::Ident("symbol".to_string()),
                                span: Span::new(16, 22),
                            }],
                        },
                        span: Span::new(10, 23),
                    },
                    span: Span::new(10, 23),
                })],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "CLOSE(symbol) should be valid: {:?}", result.err());
    }

    #[test]
    fn test_signal_close_two_args() {
        let mut tc = TypeChecker::new();
        let program = make_program(
            vec![],
            vec![StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident("CLOSE".to_string()),
                                span: Span::new(10, 15),
                            }),
                            args: vec![
                                Expr {
                                    kind: ExprKind::Ident("symbol".to_string()),
                                    span: Span::new(16, 22),
                                },
                                Expr {
                                    kind: ExprKind::IntLiteral(50),
                                    span: Span::new(24, 26),
                                },
                            ],
                        },
                        span: Span::new(10, 27),
                    },
                    span: Span::new(10, 27),
                })],
                span: Span::new(0, 40),
            })],
        );
        let result = tc.check_program(program);
        assert!(result.is_ok(), "CLOSE(symbol, 50) should be valid: {:?}", result.err());
    }

    // -----------------------------------------------------------------------
    // Connector block validation tests
    // -----------------------------------------------------------------------

    /// Helper: build a Program with a connector block and minimal strategy.
    fn make_program_with_connector(connector: ConnectorBlock) -> Program {
        Program {
            structs: vec![], enums: vec![],
            imports: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: Some(connector),
            strategy: Strategy {
                name: "Test".to_string(),
                body: vec![StrategyItem::EventHandler(EventHandler {
                    event_name: "bar".to_string(),
                    body: vec![],
                    span: Span::new(100, 120),
                })],
                span: Span::new(80, 130),
            },
            span: Span::new(0, 130),
        }
    }

    #[test]
    fn test_connector_block_valid_websocket() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "websocket".to_string(),
                span: Span::new(10, 21),
            }),
            url: Some(DataField {
                value: "wss://example.com".to_string(),
                span: Span::new(30, 49),
            }),
            symbols: Some(DataField {
                value: vec!["AAPL".to_string(), "MSFT".to_string()],
                span: Span::new(50, 70),
            }),
            interval: None,
            file: None,
            span: Span::new(0, 75),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_ok(), "Valid websocket connector should pass: {:?}", result.err());
    }

    #[test]
    fn test_connector_block_valid_poll() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "poll".to_string(),
                span: Span::new(10, 16),
            }),
            url: Some(DataField {
                value: "https://api.example.com/bars".to_string(),
                span: Span::new(30, 60),
            }),
            symbols: Some(DataField {
                value: vec!["SPY".to_string()],
                span: Span::new(65, 75),
            }),
            interval: Some(DataField {
                value: "1m".to_string(),
                span: Span::new(80, 84),
            }),
            file: None,
            span: Span::new(0, 90),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_ok(), "Valid poll connector should pass: {:?}", result.err());
    }

    #[test]
    fn test_connector_block_valid_replay() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "replay".to_string(),
                span: Span::new(10, 18),
            }),
            url: None,
            symbols: Some(DataField {
                value: vec!["AAPL".to_string()],
                span: Span::new(30, 40),
            }),
            interval: None,
            file: Some(DataField {
                value: "data/prices.csv".to_string(),
                span: Span::new(50, 67),
            }),
            span: Span::new(0, 70),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_ok(), "Valid replay connector should pass: {:?}", result.err());
    }

    #[test]
    fn test_connector_block_no_type_is_ok() {
        // If type is missing entirely, that's OK (optional validation)
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: None,
            url: Some(DataField {
                value: "wss://example.com".to_string(),
                span: Span::new(10, 29),
            }),
            symbols: Some(DataField {
                value: vec!["AAPL".to_string()],
                span: Span::new(30, 40),
            }),
            interval: None,
            file: None,
            span: Span::new(0, 45),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_ok(), "Connector without type should pass: {:?}", result.err());
    }

    #[test]
    fn test_connector_block_invalid_type() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "grpc".to_string(),
                span: Span::new(10, 16),
            }),
            url: None,
            symbols: None,
            interval: None,
            file: None,
            span: Span::new(0, 20),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_err(), "Invalid connector type should be rejected");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("at byte 10:"), "Expected span-prefixed error, got: {}", msg);
                assert!(msg.contains("grpc"), "Error should mention the invalid value, got: {}", msg);
                assert!(msg.contains("websocket"), "Error should list valid options, got: {}", msg);
                assert!(msg.contains("poll"), "Error should list valid options, got: {}", msg);
                assert!(msg.contains("replay"), "Error should list valid options, got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_websocket_missing_url() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "websocket".to_string(),
                span: Span::new(10, 21),
            }),
            url: None,
            symbols: Some(DataField {
                value: vec!["AAPL".to_string()],
                span: Span::new(30, 40),
            }),
            interval: None,
            file: None,
            span: Span::new(0, 45),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_err(), "Websocket without url should be rejected");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("at byte 10:"), "Expected span-prefixed error, got: {}", msg);
                assert!(msg.contains("websocket"), "Error should mention the type, got: {}", msg);
                assert!(msg.contains("url"), "Error should mention 'url', got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_poll_missing_url() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "poll".to_string(),
                span: Span::new(10, 16),
            }),
            url: None,
            symbols: None,
            interval: None,
            file: None,
            span: Span::new(0, 20),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_err(), "Poll without url should be rejected");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("at byte 10:"), "Expected span-prefixed error, got: {}", msg);
                assert!(msg.contains("poll"), "Error should mention the type, got: {}", msg);
                assert!(msg.contains("url"), "Error should mention 'url', got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_replay_missing_file() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "replay".to_string(),
                span: Span::new(10, 18),
            }),
            url: None,
            symbols: Some(DataField {
                value: vec!["AAPL".to_string()],
                span: Span::new(30, 40),
            }),
            interval: None,
            file: None,
            span: Span::new(0, 45),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_err(), "Replay without file should be rejected");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("at byte 10:"), "Expected span-prefixed error, got: {}", msg);
                assert!(msg.contains("replay"), "Error should mention 'replay', got: {}", msg);
                assert!(msg.contains("file"), "Error should mention 'file', got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_empty_symbols() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "websocket".to_string(),
                span: Span::new(10, 21),
            }),
            url: Some(DataField {
                value: "wss://example.com".to_string(),
                span: Span::new(30, 49),
            }),
            symbols: Some(DataField {
                value: vec![],
                span: Span::new(50, 60),
            }),
            interval: None,
            file: None,
            span: Span::new(0, 65),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_err(), "Empty symbols list should be rejected");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("at byte 50:"), "Expected span-prefixed error, got: {}", msg);
                assert!(msg.contains("at least one symbol"), "Error should mention non-empty requirement, got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_empty_string_in_symbols() {
        let mut tc = TypeChecker::new();
        let connector = ConnectorBlock {
            connector_type: Some(DataField {
                value: "websocket".to_string(),
                span: Span::new(10, 21),
            }),
            url: Some(DataField {
                value: "wss://example.com".to_string(),
                span: Span::new(30, 49),
            }),
            symbols: Some(DataField {
                value: vec!["AAPL".to_string(), "".to_string()],
                span: Span::new(50, 70),
            }),
            interval: None,
            file: None,
            span: Span::new(0, 75),
        };
        let program = make_program_with_connector(connector);
        let result = tc.check_program(program);
        assert!(result.is_err(), "Empty string in symbols should be rejected");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("at byte 50:"), "Expected span-prefixed error, got: {}", msg);
                assert!(msg.contains("index 1"), "Error should mention the position, got: {}", msg);
                assert!(msg.contains("non-empty"), "Error should mention non-empty, got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_no_connector_block_is_ok() {
        // Programs without a connector block should still pass
        let mut tc = TypeChecker::new();
        let program = make_program(vec![], vec![
            StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body: vec![],
                span: Span::new(0, 20),
            }),
        ]);
        let result = tc.check_program(program);
        assert!(result.is_ok(), "No connector block should pass: {:?}", result.err());
    }

    #[test]
    fn test_connector_block_end_to_end_parse_and_check_valid() {
        // Full pipeline: lex → parse → check for a valid connector block
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;
        use crate::typeck::check;

        let source = r#"connector {
    type = "websocket"
    url = "wss://stream.example.com/v1"
    symbols = ["AAPL", "MSFT"]
}

strategy Test {
    on bar {
    }
}"#;

        let tokens = lex_with_spans(source).expect("Lexing failed");
        let program = parse(tokens).expect("Parsing failed");
        let result = check(program);
        assert!(result.is_ok(), "Valid connector block should pass end-to-end: {:?}", result.err());

        let typed = result.unwrap();
        let cb = typed.connector_block.expect("Should have typed connector block");
        assert_eq!(cb.connector_type, Some("websocket".to_string()));
        assert_eq!(cb.url, Some("wss://stream.example.com/v1".to_string()));
        assert_eq!(cb.symbols, Some(vec!["AAPL".to_string(), "MSFT".to_string()]));
    }

    #[test]
    fn test_connector_block_end_to_end_invalid_type_error() {
        // Full pipeline: lex → parse → check for an invalid connector type
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;
        use crate::typeck::check;

        let source = r#"connector {
    type = "grpc"
    symbols = ["AAPL"]
}

strategy Test {
    on bar {
    }
}"#;

        let tokens = lex_with_spans(source).expect("Lexing failed");
        let program = parse(tokens).expect("Parsing failed");
        let result = check(program);
        assert!(result.is_err(), "Invalid connector type should fail");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("grpc"), "Error should mention 'grpc', got: {}", msg);
                assert!(msg.contains("websocket"), "Error should list valid types, got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_end_to_end_missing_url_error() {
        // Full pipeline: lex → parse → check for websocket missing url
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;
        use crate::typeck::check;

        let source = r#"connector {
    type = "websocket"
    symbols = ["AAPL"]
}

strategy Test {
    on bar {
    }
}"#;

        let tokens = lex_with_spans(source).expect("Lexing failed");
        let program = parse(tokens).expect("Parsing failed");
        let result = check(program);
        assert!(result.is_err(), "Websocket without url should fail");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("url"), "Error should mention 'url', got: {}", msg);
                assert!(msg.contains("websocket"), "Error should mention 'websocket', got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    #[test]
    fn test_connector_block_end_to_end_replay_missing_file_error() {
        // Full pipeline: lex → parse → check for replay missing file
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;
        use crate::typeck::check;

        let source = r#"connector {
    type = "replay"
    symbols = ["AAPL"]
}

strategy Test {
    on bar {
    }
}"#;

        let tokens = lex_with_spans(source).expect("Lexing failed");
        let program = parse(tokens).expect("Parsing failed");
        let result = check(program);
        assert!(result.is_err(), "Replay without file should fail");
        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(msg.contains("file"), "Error should mention 'file', got: {}", msg);
                assert!(msg.contains("replay"), "Error should mention 'replay', got: {}", msg);
            }
            other => panic!("Expected CompileError::Type, got: {:?}", other),
        }
    }

    // ===== Task 4.2: Struct registry tests =====

    /// Helper: build a StructDef from name, fields, and decorators.
    fn make_struct_def(name: &str, fields: Vec<(&str, TypeAnnotation)>) -> StructDef {
        StructDef {
            name: name.to_string(),
            type_params: vec![],
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(i, (fname, ftype))| StructField {
                    name: fname.to_string(),
                    field_type: ftype,
                    field_decorators: vec![],
                    span: Span::new(10 + i * 10, 15 + i * 10),
                })
                .collect(),
            decorators: vec![],
            span: Span::new(0, 50),
        }
    }

    #[test]
    fn test_struct_registry_simple_struct() {
        let mut tc = TypeChecker::new();
        let structs = vec![make_struct_def("Point", vec![
            ("x", TypeAnnotation::F64),
            ("y", TypeAnnotation::F64),
        ])];
        tc.register_structs(&structs).unwrap();

        assert!(tc.struct_registry.contains_key("Point"));
        let info = &tc.struct_registry["Point"];
        assert_eq!(info.name, "Point");
        assert_eq!(info.fields.len(), 2);
        assert_eq!(info.fields[0], ("x".to_string(), FluxType::Float));
        assert_eq!(info.fields[1], ("y".to_string(), FluxType::Float));
    }

    #[test]
    fn test_struct_registry_all_scalar_types() {
        let mut tc = TypeChecker::new();
        let structs = vec![make_struct_def("AllTypes", vec![
            ("a", TypeAnnotation::F64),
            ("b", TypeAnnotation::Int),
            ("c", TypeAnnotation::Bool),
            ("d", TypeAnnotation::Str),
        ])];
        tc.register_structs(&structs).unwrap();

        let info = &tc.struct_registry["AllTypes"];
        assert_eq!(info.fields[0].1, FluxType::Float);
        assert_eq!(info.fields[1].1, FluxType::Int);
        assert_eq!(info.fields[2].1, FluxType::Bool);
        assert_eq!(info.fields[3].1, FluxType::String);
    }

    #[test]
    fn test_struct_registry_duplicate_field_error() {
        let mut tc = TypeChecker::new();
        let structs = vec![make_struct_def("Bad", vec![
            ("price", TypeAnnotation::F64),
            ("size", TypeAnnotation::F64),
            ("price", TypeAnnotation::F64),
        ])];
        let err = tc.register_structs(&structs).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate field 'price' in struct 'Bad'"),
            "Expected duplicate field error, got: {}", msg);
    }

    #[test]
    fn test_struct_registry_undefined_type_error() {
        let mut tc = TypeChecker::new();
        let structs = vec![make_struct_def("Container", vec![
            ("data", TypeAnnotation::Named("Unknown".to_string())),
        ])];
        let err = tc.register_structs(&structs).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown type 'Unknown' in struct 'Container' field 'data'"),
            "Expected undefined type error, got: {}", msg);
    }

    #[test]
    fn test_struct_registry_dependency_order() {
        // Quote should be registered before MarketSnapshot since
        // MarketSnapshot has a field of type Quote.
        let mut tc = TypeChecker::new();
        let structs = vec![
            make_struct_def("MarketSnapshot", vec![
                ("quote", TypeAnnotation::Named("Quote".to_string())),
                ("mid", TypeAnnotation::F64),
            ]),
            make_struct_def("Quote", vec![
                ("bid", TypeAnnotation::F64),
                ("ask", TypeAnnotation::F64),
            ]),
        ];
        tc.register_structs(&structs).unwrap();

        // Both should be registered
        assert!(tc.struct_registry.contains_key("Quote"));
        assert!(tc.struct_registry.contains_key("MarketSnapshot"));

        // MarketSnapshot's quote field should resolve to Struct("Quote")
        let info = &tc.struct_registry["MarketSnapshot"];
        assert_eq!(info.fields[0], ("quote".to_string(), FluxType::Struct("Quote".to_string())));
    }

    #[test]
    fn test_struct_registry_fixed_array_field() {
        let mut tc = TypeChecker::new();
        let structs = vec![
            make_struct_def("Level", vec![
                ("price", TypeAnnotation::F64),
                ("size", TypeAnnotation::F64),
            ]),
            make_struct_def("Book", vec![
                ("bids", TypeAnnotation::FixedArray(Box::new(TypeAnnotation::Named("Level".to_string())), 20)),
                ("asks", TypeAnnotation::FixedArray(Box::new(TypeAnnotation::Named("Level".to_string())), 20)),
            ]),
        ];
        tc.register_structs(&structs).unwrap();

        let info = &tc.struct_registry["Book"];
        assert_eq!(
            info.fields[0].1,
            FluxType::FixedArray(Box::new(FluxType::Struct("Level".to_string())), 20)
        );
    }

    #[test]
    fn test_struct_registry_undefined_type_in_array() {
        let mut tc = TypeChecker::new();
        let structs = vec![make_struct_def("Book", vec![
            ("bids", TypeAnnotation::FixedArray(Box::new(TypeAnnotation::Named("Level".to_string())), 20)),
        ])];
        let err = tc.register_structs(&structs).unwrap_err();
        let msg = err.to_string();
        // Level is a known stdlib struct, so the error suggests importing it
        assert!(msg.contains("type 'Level' is not defined") && msg.contains("from market::l2 import {Level}"),
            "Expected import suggestion error, got: {}", msg);
    }

    #[test]
    fn test_struct_registry_multiple_independent_structs() {
        let mut tc = TypeChecker::new();
        let structs = vec![
            make_struct_def("Tick", vec![
                ("price", TypeAnnotation::F64),
                ("size", TypeAnnotation::F64),
            ]),
            make_struct_def("Bar", vec![
                ("open", TypeAnnotation::F64),
                ("close", TypeAnnotation::F64),
            ]),
        ];
        tc.register_structs(&structs).unwrap();
        assert!(tc.struct_registry.contains_key("Tick"));
        assert!(tc.struct_registry.contains_key("Bar"));
    }

    #[test]
    fn test_struct_registry_chain_dependency() {
        // A depends on B, B depends on C — all should register in order C, B, A
        let mut tc = TypeChecker::new();
        let structs = vec![
            make_struct_def("A", vec![
                ("b_field", TypeAnnotation::Named("B".to_string())),
            ]),
            make_struct_def("B", vec![
                ("c_field", TypeAnnotation::Named("C".to_string())),
            ]),
            make_struct_def("C", vec![
                ("val", TypeAnnotation::Int),
            ]),
        ];
        tc.register_structs(&structs).unwrap();
        assert!(tc.struct_registry.contains_key("A"));
        assert!(tc.struct_registry.contains_key("B"));
        assert!(tc.struct_registry.contains_key("C"));

        // Verify the resolved types
        let info_a = &tc.struct_registry["A"];
        assert_eq!(info_a.fields[0].1, FluxType::Struct("B".to_string()));
        let info_b = &tc.struct_registry["B"];
        assert_eq!(info_b.fields[0].1, FluxType::Struct("C".to_string()));
    }

    #[test]
    fn test_struct_registry_error_span_format() {
        // Verify error messages follow the "at byte N:" format
        let mut tc = TypeChecker::new();
        let structs = vec![make_struct_def("Bad", vec![
            ("x", TypeAnnotation::F64),
            ("x", TypeAnnotation::Int),
        ])];
        let err = tc.register_structs(&structs).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("at byte"), "Error should contain 'at byte', got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // Enum construction typechecking
    // -----------------------------------------------------------------------

    fn register_test_enum(tc: &mut TypeChecker) {
        use crate::typeck::enum_info::{EnumInfo, VariantInfo};
        let variants = vec![
            VariantInfo::unit("Market".to_string(), Span::new(10, 16)),
            VariantInfo::with_fields(
                "Limit".to_string(),
                vec![("price".to_string(), FluxType::Float)],
                Span::new(20, 40),
            ),
            VariantInfo::with_fields(
                "StopLimit".to_string(),
                vec![
                    ("stop".to_string(), FluxType::Float),
                    ("limit".to_string(), FluxType::Float),
                ],
                Span::new(45, 80),
            ),
        ];
        let enum_info = EnumInfo::new("OrderType".to_string(), variants, Span::new(0, 100));
        tc.env.register_enum(enum_info).unwrap();
    }

    #[test]
    fn test_enum_construction_unit_variant() {
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "Market".to_string(),
            args: vec![],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Enum("OrderType".to_string()));
    }

    #[test]
    fn test_enum_construction_data_variant_single_field() {
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "Limit".to_string(),
            args: vec![make_expr(ExprKind::FloatLiteral(150.0))],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Enum("OrderType".to_string()));
    }

    #[test]
    fn test_enum_construction_data_variant_multiple_fields() {
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "StopLimit".to_string(),
            args: vec![
                make_expr(ExprKind::FloatLiteral(100.0)),
                make_expr(ExprKind::FloatLiteral(105.0)),
            ],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Enum("OrderType".to_string()));
    }

    #[test]
    fn test_enum_construction_int_coerced_to_float() {
        // Int should be assignable to Float fields
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "Limit".to_string(),
            args: vec![make_expr(ExprKind::IntLiteral(150))],
        });
        let result = tc.check_expr(expr).unwrap();
        assert_eq!(result.resolved_type, FluxType::Enum("OrderType".to_string()));
    }

    #[test]
    fn test_enum_construction_unknown_enum() {
        let mut tc = TypeChecker::new();
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "NonExistent".to_string(),
            variant_name: "Foo".to_string(),
            args: vec![],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown enum type 'NonExistent'"), "got: {}", msg);
    }

    #[test]
    fn test_enum_construction_unknown_variant() {
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "FillOrKill".to_string(),
            args: vec![],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("has no variant 'FillOrKill'"), "got: {}", msg);
    }

    #[test]
    fn test_enum_construction_too_many_args() {
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "Market".to_string(),
            args: vec![make_expr(ExprKind::FloatLiteral(1.0))],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("expects 0 arguments, got 1"), "got: {}", msg);
    }

    #[test]
    fn test_enum_construction_too_few_args() {
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "StopLimit".to_string(),
            args: vec![make_expr(ExprKind::FloatLiteral(100.0))],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("expects 2 arguments, got 1"), "got: {}", msg);
    }

    #[test]
    fn test_enum_construction_type_mismatch() {
        let mut tc = TypeChecker::new();
        register_test_enum(&mut tc);
        let expr = make_expr(ExprKind::EnumConstruction {
            enum_name: "OrderType".to_string(),
            variant_name: "Limit".to_string(),
            args: vec![make_expr(ExprKind::StringLiteral("bad".to_string()))],
        });
        let err = tc.check_expr(expr).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("field 'price'"), "got: {}", msg);
        assert!(msg.contains("expects Float"), "got: {}", msg);
        assert!(msg.contains("got Str"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // Trait Bound Checking (Requirements 9.2, 9.3, 13.5)
    // -----------------------------------------------------------------------

    /// Helper: lex → parse → typecheck a source string.
    fn check_source(source: &str) -> Result<TypedProgram> {
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;

        let tokens = lex_with_spans(source).expect("lexing failed");
        let program = parse(tokens).expect("parsing failed");
        crate::typeck::check(program)
    }

    #[test]
    fn test_debug_generic_fn_type_params() {
        // Verify parser sets type_params on FnDef correctly
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;
        use crate::parser::ast::TypeAnnotation;

        // Test without struct to isolate the function issue
        let source = "fn identity[T](val: T) -> f64 {\n    return 1.0\n}\n\nstrategy Test {\n    on bar {\n        x = 1.0\n    }\n}\n";
        let tokens = lex_with_spans(source).unwrap();
        let program = parse(tokens).unwrap();

        assert_eq!(program.functions.len(), 1, "Expected 1 function");
        let f = &program.functions[0];
        assert_eq!(f.type_params.len(), 1, "Expected 1 type param");
        assert_eq!(f.type_params[0].name, "T");

        // Verify the param type annotation
        let param = &f.params[0];
        assert_eq!(param.name, "val");
        let param_type = param.param_type.as_ref().unwrap();
        assert_eq!(
            *param_type,
            TypeAnnotation::Named("T".to_string()),
            "Expected Named('T'), got {:?}",
            param_type
        );

        // Full pipeline typecheck should work (no struct interference)
        let tokens2 = lex_with_spans(source).unwrap();
        let program2 = parse(tokens2).unwrap();
        let result = crate::typeck::check(program2);
        assert!(result.is_ok(), "Full pipeline (no struct) should succeed, got: {:?}", result.err());
    }

    #[test]
    fn test_trait_bound_satisfied() {
        // Define a trait, a struct implementing it, and a bounded generic function.
        // Calling with the implementing type should succeed.
        let source = r#"
trait DataFeed {
    fn next(self) -> f64
}

struct LiveFeed {
    price: f64
}

impl DataFeed for LiveFeed {
    fn next(self) -> f64 {
        return 42.0
    }
}

fn process[T: DataFeed](feed: T) -> f64 {
    return 1.0
}

strategy Test {
    on bar {
        lf = LiveFeed { price = 42.0 }
        result = process(lf)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    }

    #[test]
    fn test_trait_bound_violated() {
        // Define a trait, a struct that does NOT implement it, and a bounded generic fn.
        // Calling with the non-implementing type should produce an error.
        let source = r#"
trait DataFeed {
    fn next(self) -> f64
}

struct BadFeed {
    price: f64
}

fn process[T: DataFeed](feed: T) -> f64 {
    return 1.0
}

strategy Test {
    on bar {
        bf = BadFeed { price = 10.0 }
        result = process(bf)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected a trait bound violation error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not implement trait"),
            "Expected 'does not implement trait' in error, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("BadFeed"),
            "Expected type name 'BadFeed' in error, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("DataFeed"),
            "Expected trait name 'DataFeed' in error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_trait_bound_error_includes_type_param_name() {
        // Verify the error message mentions the type parameter name ('T').
        let source = r#"
trait Printable {
    fn show(self) -> f64
}

struct Widget {
    id: int
}

fn display[T: Printable](item: T) -> f64 {
    return 0.0
}

strategy Test {
    on bar {
        w = Widget { id = 1 }
        display(w)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("'T'"),
            "Expected type param name 'T' in error, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("Widget"),
            "Expected 'Widget' in error, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("Printable"),
            "Expected 'Printable' in error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_trait_bound_no_bound_no_error() {
        // A generic function without trait bounds should not produce trait errors.
        let source = r#"
struct Point {
    x: f64
}

fn identity[T](val: T) -> T {
    return 1.0
}

strategy Test {
    on bar {
        p = Point { x = 1.0 }
        result = identity(p)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "Expected Ok for unbounded generic, got: {:?}", result.err());
    }

    #[test]
    fn test_trait_bound_multiple_type_params() {
        // Multiple type params with different bounds: one satisfied, one not.
        let source = r#"
trait Readable {
    fn read(self) -> f64
}

trait Writable {
    fn write(self) -> f64
}

struct Reader {
    val: f64
}

impl Readable for Reader {
    fn read(self) -> f64 {
        return 1.0
    }
}

fn transfer[R: Readable, W: Writable](src: R, dst: W) -> f64 {
    return 1.0
}

strategy Test {
    on bar {
        r = Reader { val = 5.0 }
        transfer(r, r)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected trait bound violation for Writable");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Writable"),
            "Expected 'Writable' in error, got: {}",
            err_msg
        );
    }

    // -----------------------------------------------------------------------
    // HashMap built-in generic type tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_hashmap_new_typechecks() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "HashMap.new() should typecheck: {:?}", result.err());
    }

    #[test]
    fn test_hashmap_insert_typechecks() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("AAPL", 150.0)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "HashMap insert should typecheck: {:?}", result.err());
    }

    #[test]
    fn test_hashmap_get_returns_value_type() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("AAPL", 150.0)
        price = m.get("AAPL")
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "HashMap get should typecheck: {:?}", result.err());
    }

    #[test]
    fn test_hashmap_contains_key_returns_bool() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("AAPL", 150.0)
        exists = m.contains_key("AAPL")
        if exists {
            OPEN(symbol, 100.0)
        }
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "HashMap contains_key should typecheck: {:?}", result.err());
    }

    #[test]
    fn test_hashmap_remove_returns_hashmap() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("AAPL", 150.0)
        m = m.remove("AAPL")
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "HashMap remove should typecheck: {:?}", result.err());
    }

    #[test]
    fn test_hashmap_key_type_mismatch() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("AAPL", 150.0)
        m = m.insert(123, 200.0)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected key type mismatch error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("key") && err_msg.contains("mismatch"),
            "Expected key type mismatch error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_hashmap_value_type_mismatch() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("AAPL", 150.0)
        m = m.insert("GOOG", "not a number")
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected value type mismatch error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("value") && err_msg.contains("mismatch"),
            "Expected value type mismatch error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_hashmap_new_with_args_error() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new(42)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected error for HashMap.new() with args");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("0 arguments"),
            "Expected '0 arguments' in error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_hashmap_insert_wrong_arg_count() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("key")
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected error for insert with 1 arg");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("2 arguments"),
            "Expected '2 arguments' in error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_hashmap_unknown_method_error() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m.clear()
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected error for unknown method");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not have method 'clear'"),
            "Expected 'does not have method' error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_hashmap_unknown_static_method_error() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.create()
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected error for unknown static method");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("no static method 'create'"),
            "Expected 'no static method' error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_hashmap_int_keys_float_values() {
        // Test HashMap with Int keys and Float values
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert(1, 100.5)
        m = m.insert(2, 200.5)
        val = m.get(1)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_ok(), "HashMap[Int, Float] should typecheck: {:?}", result.err());
    }

    #[test]
    fn test_hashmap_get_key_type_mismatch() {
        let source = r#"
strategy Test {
    on bar {
        m = HashMap.new()
        m = m.insert("AAPL", 150.0)
        val = m.get(42)
    }
}
"#;
        let result = check_source(source);
        assert!(result.is_err(), "Expected key type mismatch on get");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("key") && err_msg.contains("mismatch"),
            "Expected key type mismatch error, got: {}",
            err_msg
        );
    }
}
