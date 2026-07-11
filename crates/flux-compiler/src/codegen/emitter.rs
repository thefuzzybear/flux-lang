//! Code emitter: walks the TypedProgram and produces Rust source code.

use std::collections::{HashMap, HashSet};

use crate::error::{CompileError, Result};
use crate::parser::ast::{BinOp, UnaryOp};
use crate::typeck::typed_ast::*;
use crate::typeck::types::FluxType;

use super::fn_context::{analyze_function_context, FnContext};
use super::type_map::map_type;

/// Known market data identifiers available through `ctx`.
const MARKET_DATA: &[&str] = &[
    "close", "open", "high", "low", "volume", "symbol", "in_position",
];

/// Known signal-producing function names.
const SIGNAL_FUNCTIONS: &[&str] = &["OPEN", "CLOSE"];

/// The code emitter accumulates Rust source code by walking the typed AST.
pub(crate) struct CodeEmitter<'a> {
    /// Reference to the input typed program.
    program: &'a TypedProgram,
    /// Accumulated output string buffer.
    output: String,
    /// Current indentation level (0 = top-level).
    indent_level: usize,
    /// Parameter names (accessed via `self.`).
    params: HashSet<String>,
    /// State variable names (accessed via `self.`).
    state_vars: HashSet<String>,
    /// Property names (accessed via `self.`).
    properties: HashSet<String>,
    /// Local variables declared in the current handler scope.
    local_vars: HashSet<String>,
    /// Imported function names (accessed as bare names).
    imported_functions: HashSet<String>,
    /// Context requirements for user-defined functions (ctx and/or signals forwarding).
    fn_contexts: HashMap<String, FnContext>,
    /// Whether we are currently emitting a user-defined function body.
    /// When true, identifiers are resolved without `self.` prefix and
    /// market data vars resolve to `ctx.X` only if the function receives ctx.
    in_fn_body: bool,
    /// Parameter names of the current user function being emitted.
    fn_params: HashSet<String>,
}

impl<'a> CodeEmitter<'a> {
    /// Create a new CodeEmitter, collecting context from the TypedProgram.
    pub fn new(program: &'a TypedProgram) -> Self {
        let mut params = HashSet::new();
        let mut state_vars = HashSet::new();
        let mut properties = HashSet::new();
        let mut imported_functions = HashSet::new();

        // Collect imported function names
        for import in &program.imports {
            for name in &import.names {
                imported_functions.insert(name.clone());
            }
        }

        // Walk strategy body to collect params, state vars, and properties
        for item in &program.strategy.body {
            match item {
                TypedStrategyItem::ParamsBlock(pb) => {
                    for p in &pb.params {
                        params.insert(p.name.clone());
                    }
                }
                TypedStrategyItem::StateBlock(sb) => {
                    for v in &sb.variables {
                        state_vars.insert(v.name.clone());
                    }
                }
                TypedStrategyItem::Property(prop) => {
                    properties.insert(prop.name.clone());
                }
                TypedStrategyItem::EventHandler(_) => {}
            }
        }

        // Analyze user-defined function context requirements
        let fn_contexts = analyze_function_context(&program.functions);

        Self {
            program,
            output: String::new(),
            indent_level: 0,
            params,
            state_vars,
            properties,
            local_vars: HashSet::new(),
            imported_functions,
            fn_contexts,
            in_fn_body: false,
            fn_params: HashSet::new(),
        }
    }

    // ========================================================================
    // Public entry point
    // ========================================================================

    /// Main entry point: emit the full Rust source file.
    pub fn emit(&mut self) -> Result<String> {
        self.local_vars.clear();
        self.output.clear();
        self.emit_preamble();
        self.emit_enum_defs()?;
        self.emit_trait_defs()?;
        self.emit_struct_definitions()?;
        self.emit_impl_blocks()?;
        self.emit_user_functions()?;
        self.emit_struct()?;
        self.output.push('\n');
        self.emit_default_impl()?;
        self.output.push('\n');
        self.emit_strategy_impl()?;
        Ok(self.output.clone())
    }

    // ========================================================================
    // Top-level structure emission
    // ========================================================================

    /// Format type parameters with optional trait bounds as angle-bracket syntax.
    ///
    /// Translates Flux square-bracket generics `[T]` and `[T: Trait]` to
    /// Rust angle-bracket syntax `<T>` and `<T: Trait>`.
    ///
    /// Returns an empty string if `type_params` is empty.
    fn format_type_params(type_params: &[String], bounds: &[Option<String>]) -> String {
        if type_params.is_empty() {
            return String::new();
        }
        let params: Vec<String> = type_params
            .iter()
            .enumerate()
            .map(|(i, name)| {
                if let Some(Some(bound)) = bounds.get(i) {
                    format!("{}: {}", name, bound)
                } else {
                    name.clone()
                }
            })
            .collect();
        format!("<{}>", params.join(", "))
    }

    /// Format type parameters without bounds (for structs and enums that only
    /// carry type parameter names without trait constraints).
    fn format_type_params_no_bounds(type_params: &[String]) -> String {
        if type_params.is_empty() {
            return String::new();
        }
        format!("<{}>", type_params.join(", "))
    }

    /// Emit the preamble: `use flux_runtime::*;\n\n`
    fn emit_preamble(&mut self) {
        self.output.push_str("use flux_runtime::*;\n\n");
    }

    /// Emit all enum definitions from the typed program.
    /// Each enum gets `#[derive(Debug, Clone, PartialEq)]` and uses struct-style
    /// variants for data variants (`Variant { field: Type }`) and unit variants
    /// for variants with no fields.
    fn emit_enum_defs(&mut self) -> Result<()> {
        if self.program.enums.is_empty() {
            return Ok(());
        }

        for enum_def in &self.program.enums {
            self.output
                .push_str("#[derive(Debug, Clone, PartialEq)]\n");
            let generics = Self::format_type_params_no_bounds(&enum_def.type_params);
            self.output
                .push_str(&format!("enum {}{} {{\n", enum_def.name, generics));

            for variant in &enum_def.variants {
                if variant.fields.is_empty() {
                    // Unit variant
                    self.output
                        .push_str(&format!("    {},\n", variant.name));
                } else {
                    // Struct variant with named fields
                    self.output
                        .push_str(&format!("    {} {{\n", variant.name));
                    for (field_name, field_type) in &variant.fields {
                        let rust_type = map_type(field_type, variant.span.start)?;
                        self.output.push_str(&format!(
                            "        {}: {},\n",
                            field_name, rust_type
                        ));
                    }
                    self.output.push_str("    },\n");
                }
            }

            self.output.push_str("}\n\n");
        }

        Ok(())
    }

    /// Emit all trait definitions from the typed program.
    ///
    /// Each trait is emitted as:
    /// ```rust
    /// trait TraitName {
    ///     fn method(&self, ...) -> Type;
    /// }
    /// ```
    /// Method signatures end with `;` (no body).
    fn emit_trait_defs(&mut self) -> Result<()> {
        if self.program.traits.is_empty() {
            return Ok(());
        }

        for trait_def in &self.program.traits {
            self.output
                .push_str(&format!("trait {} {{\n", trait_def.name));

            for method in &trait_def.methods {
                self.output.push_str("    fn ");
                self.output.push_str(&method.name);
                self.output.push('(');

                let mut first = true;
                if method.has_self {
                    self.output.push_str("&self");
                    first = false;
                }

                for param_type in &method.param_types {
                    if !first {
                        self.output.push_str(", ");
                    }
                    first = false;
                    let rust_type = map_type(param_type, trait_def.span.start)?;
                    // Use generic parameter names for non-self params
                    self.output.push_str(&format!("_: {}", rust_type));
                }

                self.output.push(')');

                // Emit return type if not void/null
                match &method.return_type {
                    FluxType::Null | FluxType::Void => {}
                    other => {
                        let rust_type = map_type(other, trait_def.span.start)?;
                        self.output.push_str(&format!(" -> {}", rust_type));
                    }
                }

                self.output.push_str(";\n");
            }

            self.output.push_str("}\n\n");
        }

        Ok(())
    }

    /// Emit `impl` blocks for structs — both inherent and trait impls.
    ///
    /// For inherent impls (where `trait_name` is `None`), emits
    /// `impl StructName { ... }`. For trait impls, emits
    /// `impl TraitName for StructName { ... }`.
    fn emit_impl_blocks(&mut self) -> Result<()> {
        let impl_blocks = self.program.impl_blocks.clone();
        // Look up type params from struct definitions to emit generic impl blocks
        let struct_type_params: HashMap<String, Vec<String>> = self
            .program
            .structs
            .iter()
            .filter(|s| !s.type_params.is_empty())
            .map(|s| (s.name.clone(), s.type_params.clone()))
            .collect();

        for impl_block in &impl_blocks {
            // Determine if the target type is generic
            let target_generics = struct_type_params
                .get(&impl_block.target_type)
                .cloned()
                .unwrap_or_default();
            let generics_str = Self::format_type_params_no_bounds(&target_generics);

            if let Some(trait_name) = &impl_block.trait_name {
                // Trait impl: `impl<T> TraitName for StructName<T> { ... }`
                self.output.push_str(&format!(
                    "impl{} {} for {}{} {{\n",
                    generics_str, trait_name, impl_block.target_type, generics_str
                ));
            } else {
                // Inherent impl: `impl<T> StructName<T> { ... }`
                self.output
                    .push_str(&format!("impl{} {}{} {{\n", generics_str, impl_block.target_type, generics_str));
            }

            for method in &impl_block.methods {
                self.emit_impl_method(method)?;
                self.output.push('\n');
            }

            self.output.push_str("}\n\n");
        }

        Ok(())
    }

    /// Emit a single method inside an impl block.
    ///
    /// If the first parameter is "self", it is emitted as `&self` in Rust.
    /// Remaining parameters are emitted with their type annotations.
    fn emit_impl_method(&mut self, method: &TypedFnDef) -> Result<()> {
        self.in_fn_body = true;
        self.fn_params.clear();
        self.local_vars.clear();

        // Register non-self params so identifier resolution works
        for param in &method.params {
            if param != "self" {
                self.fn_params.insert(param.clone());
            }
        }

        // Emit signature
        self.indent_level = 1;
        self.write_indent();
        let generics = Self::format_type_params(&method.type_params, &method.type_param_bounds);
        self.output.push_str(&format!("fn {}{}(", method.name, generics));

        let mut first = true;
        for (i, param) in method.params.iter().enumerate() {
            if !first {
                self.output.push_str(", ");
            }
            first = false;

            if param == "self" {
                // Map Flux `self` to Rust `&self`
                self.output.push_str("&self");
            } else {
                let rust_type = if i < method.param_types.len() {
                    map_type(&method.param_types[i], method.span.start)?
                } else {
                    "f64".to_string()
                };
                self.output.push_str(&format!("{}: {}", param, rust_type));
            }
        }

        self.output.push(')');

        // Emit return type annotation
        match &method.return_type {
            FluxType::Null | FluxType::Void => {}
            other => {
                let rust_type = map_type(other, method.span.start)?;
                self.output.push_str(&format!(" -> {}", rust_type));
            }
        }

        self.output.push_str(" {\n");

        // Emit body statements
        self.indent_level = 2;
        for stmt in &method.body {
            self.emit_stmt(stmt)?;
        }

        self.indent_level = 1;
        self.write_indent();
        self.output.push_str("}\n");

        // Restore context
        self.indent_level = 0;
        self.in_fn_body = false;
        self.fn_params.clear();
        self.local_vars.clear();

        Ok(())
    }

    /// Return struct definitions in dependency order (topological sort).
    ///
    /// If struct A contains a field of type B, B appears before A in the
    /// returned vec. The TypedProgram.structs field is already dependency-sorted
    /// by the typechecker, but this method also performs its own topological sort
    /// to ensure correctness regardless of input ordering.
    fn dependency_sorted_structs(&self) -> Vec<&TypedStructDef> {
        let structs = &self.program.structs;
        if structs.is_empty() {
            return vec![];
        }

        // Build name → index map
        let name_to_idx: HashMap<&str, usize> = structs
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i))
            .collect();

        // Build dependency edges: deps[i] = indices of structs that i depends on
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); structs.len()];
        for (i, struct_def) in structs.iter().enumerate() {
            for field in &struct_def.fields {
                for dep_name in Self::struct_type_refs(&field.resolved_type) {
                    if let Some(&dep_idx) = name_to_idx.get(dep_name) {
                        deps[i].push(dep_idx);
                    }
                }
            }
        }

        // Kahn's algorithm for topological sort
        let n = structs.len();
        let mut rev_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, node_deps) in deps.iter().enumerate() {
            for &dep in node_deps {
                rev_adj[dep].push(i);
            }
        }

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
            for &dependent in &rev_adj[node] {
                in_degree[dependent] -= 1;
                if in_degree[dependent] == 0 {
                    queue.push_back(dependent);
                }
            }
        }

        // If cycle detected (shouldn't happen — typechecker catches this), fall back to input order
        if order.len() != n {
            return structs.iter().collect();
        }

        order.iter().map(|&i| &structs[i]).collect()
    }

    /// Extract struct name references from a resolved FluxType.
    fn struct_type_refs(ty: &FluxType) -> Vec<&str> {
        match ty {
            FluxType::Struct(name) => vec![name.as_str()],
            FluxType::FixedArray(elem, _) => Self::struct_type_refs(elem),
            _ => vec![],
        }
    }

    /// Return the zero-initialization expression for a given FluxType.
    /// Used by `@zero_init` decorator to generate `impl Default`.
    fn zero_init_value(ty: &FluxType) -> String {
        match ty {
            FluxType::Float => "0.0".to_string(),
            FluxType::Int => "0".to_string(),
            FluxType::Bool => "false".to_string(),
            FluxType::String => "String::new()".to_string(),
            FluxType::Struct(name) => format!("{}::default()", name),
            FluxType::FixedArray(elem, size) => {
                let elem_zero = Self::zero_init_value(elem);
                format!("[{}; {}]", elem_zero, size)
            }
            FluxType::List(_) => "Vec::new()".to_string(),
            FluxType::VecFloat => "Vec::new()".to_string(),
            FluxType::MatFloat => "Vec::new()".to_string(),
            FluxType::Signal => "Signal::default()".to_string(),
            FluxType::Null | FluxType::Void => "()".to_string(),
            FluxType::Fn { .. } => "/* unsupported */".to_string(),
            // Enum and Generic types will be properly implemented in Phase 1B and Phase 4
            FluxType::Enum(_) => "/* enum zero-init not yet supported */".to_string(),
            FluxType::TypeParam(_) => "/* type param zero-init not yet supported */".to_string(),
            FluxType::Generic(_, _) => "/* generic zero-init not yet supported */".to_string(),
        }
    }

    /// Emit all Flux struct definitions in dependency order.
    /// Each struct gets `#[derive(Clone, Copy)]` and `pub` visibility on all fields.
    fn emit_struct_definitions(&mut self) -> Result<()> {
        let sorted_structs = self.dependency_sorted_structs();
        if sorted_structs.is_empty() {
            return Ok(());
        }

        // Collect struct info to release the borrow on self
        struct StructEmitInfo {
            name: String,
            type_params: Vec<String>,
            fields: Vec<(String, String)>,
            /// Original FluxType for each field (used for zero-init value generation).
            field_types: Vec<(String, FluxType)>,
            /// Bit widths for @bitfield fields: (field_name, bit_width).
            bit_widths: Vec<(String, usize)>,
            /// Field-level decorator names per field: (field_name, vec_of_decorator_names).
            field_decorators: Vec<(String, Vec<String>)>,
            is_heap: bool,
            is_packed: bool,
            aligned_n: Option<u32>,
            simd_n: Option<u32>,
            is_prefetch: bool,
            is_streaming: bool,
            is_soa: bool,
            is_pool: Option<u32>,
            is_volatile: bool,
            is_bitfield: bool,
            is_zero_init: bool,
            is_immutable: bool,
        }

        let struct_data: Vec<StructEmitInfo> = sorted_structs
            .iter()
            .map(|s| {
                let fields: std::result::Result<Vec<_>, _> = s
                    .fields
                    .iter()
                    .map(|f| {
                        map_type(&f.resolved_type, f.span.start)
                            .map(|rust_type| (f.name.clone(), rust_type))
                    })
                    .collect();
                let field_types: Vec<(String, FluxType)> = s
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.resolved_type.clone()))
                    .collect();
                let bit_widths: Vec<(String, usize)> = s
                    .fields
                    .iter()
                    .filter_map(|f| f.bit_width.map(|w| (f.name.clone(), w)))
                    .collect();
                let field_decorators: Vec<(String, Vec<String>)> = s
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.field_decorator_names.clone()))
                    .collect();
                let is_heap = s.decorators.iter().any(|d| d.kind == DecoratorKind::Heap);
                let is_packed = s.decorators.iter().any(|d| d.kind == DecoratorKind::Packed);
                let aligned_n = s.decorators.iter().find_map(|d| {
                    if let DecoratorKind::Aligned(n) = d.kind { Some(n) } else { None }
                });
                let simd_n = s.decorators.iter().find_map(|d| {
                    if let DecoratorKind::Simd(n) = d.kind { Some(n) } else { None }
                });
                let is_prefetch = s.decorators.iter().any(|d| d.kind == DecoratorKind::Prefetch);
                let is_streaming = s.decorators.iter().any(|d| d.kind == DecoratorKind::Streaming);
                let is_soa = s.decorators.iter().any(|d| d.kind == DecoratorKind::Soa);
                let is_pool = s.decorators.iter().find_map(|d| {
                    if let DecoratorKind::Pool(n) = d.kind { Some(n) } else { None }
                });
                let is_volatile = s.decorators.iter().any(|d| d.kind == DecoratorKind::Volatile);
                let is_bitfield = s.decorators.iter().any(|d| d.kind == DecoratorKind::Bitfield);
                let is_zero_init = s.decorators.iter().any(|d| d.kind == DecoratorKind::ZeroInit);
                let is_immutable = s.decorators.iter().any(|d| d.kind == DecoratorKind::Immutable);
                fields.map(|fs| StructEmitInfo {
                    name: s.name.clone(),
                    type_params: s.type_params.clone(),
                    fields: fs,
                    field_types,
                    bit_widths,
                    field_decorators,
                    is_heap,
                    is_packed,
                    aligned_n,
                    simd_n,
                    is_prefetch,
                    is_streaming,
                    is_soa,
                    is_pool,
                    is_volatile,
                    is_bitfield,
                    is_zero_init,
                    is_immutable,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        for info in &struct_data {
            // Emit decorator comments for advanced transformations
            if info.is_prefetch {
                self.output.push_str("// @prefetch: CPU prefetch hints for this struct\n");
            }
            if info.is_streaming {
                self.output.push_str("// @streaming: non-temporal stores for field writes\n");
            }
            if info.is_soa {
                self.output.push_str("// @soa: struct-of-arrays layout — companion SoA container emitted below\n");
            }
            if let Some(n) = info.is_pool {
                self.output.push_str(&format!("// @pool({}): pre-allocated slab with free-list\n", n));
            }
            if info.is_volatile {
                self.output.push_str("// @volatile: read_volatile/write_volatile for all field access\n");
            }
            if info.is_bitfield {
                self.output.push_str("// @bitfield: bit-level packing with shift/mask operations\n");
            }
            if info.is_zero_init {
                self.output.push_str("// @zero_init: all fields zero-initialized by default\n");
            }
            if info.is_immutable {
                self.output.push_str("// @immutable: no field mutation after construction\n");
            }

            // Emit #[derive(...)]
            if info.is_heap {
                self.output.push_str("#[derive(Clone)]\n");
            } else {
                self.output.push_str("#[derive(Clone, Copy)]\n");
            }

            // Emit #[repr(...)] attributes
            if info.is_packed && !info.is_bitfield {
                self.output.push_str("#[repr(packed)]\n");
            }
            if let Some(n) = info.aligned_n {
                self.output.push_str(&format!("#[repr(align({}))]\n", n));
            }
            if let Some(n) = info.simd_n {
                // @simd(N) emits align(N/8)
                self.output.push_str(&format!("#[repr(align({}))]\n", n / 8));
            }

            if info.is_bitfield {
                // --- @bitfield: emit packed u64 storage with bitwise accessors ---
                let generics = Self::format_type_params_no_bounds(&info.type_params);
                self.output.push_str(&format!("pub struct {}{} {{\n", info.name, generics));
                self.output.push_str("    _bits: u64,\n");
                self.output.push_str("}\n\n");

                self.output.push_str(&format!("impl{} {}{} {{\n", generics, info.name, generics));
                let mut bit_offset: usize = 0;
                for (field_name, width) in &info.bit_widths {
                    let mask = (1u64 << width) - 1;
                    let field_type = info.field_types.iter().find(|(n, _)| n == field_name);
                    let is_bool = field_type.map(|(_, t)| matches!(t, FluxType::Bool)).unwrap_or(false);

                    if is_bool {
                        // Bool getter: returns bool
                        self.output.push_str(&format!(
                            "    // {}: bit {}, width {}\n",
                            field_name, bit_offset, width
                        ));
                        self.output.push_str(&format!(
                            "    pub fn get_{}(&self) -> bool {{ (self._bits >> {}) & 0x{:X} != 0 }}\n",
                            field_name, bit_offset, mask
                        ));
                        // Bool setter
                        self.output.push_str(&format!(
                            "    pub fn set_{}(&mut self, val: bool) {{\n",
                            field_name
                        ));
                        self.output.push_str(&format!(
                            "        if val {{ self._bits |= 0x{:X} << {}; }} else {{ self._bits &= !(0x{:X} << {}); }}\n",
                            mask, bit_offset, mask, bit_offset
                        ));
                        self.output.push_str("    }\n");
                    } else {
                        // Int getter: returns i64
                        self.output.push_str(&format!(
                            "    // {}: bits {}..{}, width {}\n",
                            field_name, bit_offset, bit_offset + width, width
                        ));
                        self.output.push_str(&format!(
                            "    pub fn get_{}(&self) -> i64 {{ ((self._bits >> {}) & 0x{:X}) as i64 }}\n",
                            field_name, bit_offset, mask
                        ));
                        // Int setter
                        self.output.push_str(&format!(
                            "    pub fn set_{}(&mut self, val: i64) {{\n",
                            field_name
                        ));
                        self.output.push_str(&format!(
                            "        self._bits = (self._bits & !(0x{:X} << {})) | (((val as u64) & 0x{:X}) << {});\n",
                            mask, bit_offset, mask, bit_offset
                        ));
                        self.output.push_str("    }\n");
                    }

                    bit_offset += width;
                }
                self.output.push_str("}\n\n");
            } else {
                // --- Normal struct emission ---
                let generics = Self::format_type_params_no_bounds(&info.type_params);
                self.output.push_str(&format!("pub struct {}{} {{\n", info.name, generics));
                for (field_name, rust_type) in &info.fields {
                    self.output
                        .push_str(&format!("    pub {}: {},\n", field_name, rust_type));
                }
                self.output.push_str("}\n\n");
            }

            // Emit `impl Default` for @zero_init structs (not applicable for @bitfield)
            if info.is_zero_init && !info.is_bitfield {
                self.output.push_str(&format!("impl Default for {} {{\n", info.name));
                self.output.push_str("    fn default() -> Self {\n");
                self.output.push_str("        Self {\n");
                for (field_name, field_type) in &info.field_types {
                    let zero_value = Self::zero_init_value(field_type);
                    self.output.push_str(&format!("            {}: {},\n", field_name, zero_value));
                }
                self.output.push_str("        }\n");
                self.output.push_str("    }\n");
                self.output.push_str("}\n\n");
            }

            // Emit volatile accessor impl block for @volatile structs (not applicable for @bitfield)
            if info.is_volatile && !info.is_bitfield {
                self.output.push_str(&format!("impl {} {{\n", info.name));
                for (field_name, rust_type) in &info.fields {
                    // Getter: read_volatile
                    self.output.push_str("    #[inline(always)]\n");
                    self.output.push_str(&format!(
                        "    pub fn get_{}(&self) -> {} {{\n",
                        field_name, rust_type
                    ));
                    self.output.push_str(&format!(
                        "        unsafe {{ std::ptr::read_volatile(&self.{}) }}\n",
                        field_name
                    ));
                    self.output.push_str("    }\n");
                    // Setter: write_volatile
                    self.output.push_str("    #[inline(always)]\n");
                    self.output.push_str(&format!(
                        "    pub fn set_{}(&mut self, val: {}) {{\n",
                        field_name, rust_type
                    ));
                    self.output.push_str(&format!(
                        "        unsafe {{ std::ptr::write_volatile(&mut self.{}, val) }}\n",
                        field_name
                    ));
                    self.output.push_str("    }\n");
                }
                self.output.push_str("}\n\n");
            }

            // Emit prefetch helper impl block for @prefetch structs
            if info.is_prefetch {
                self.output.push_str(&format!("impl {} {{\n", info.name));
                self.output.push_str("    /// Prefetch this struct into L1 cache for read access.\n");
                self.output.push_str("    #[inline(always)]\n");
                self.output.push_str("    pub fn prefetch(&self) {\n");
                self.output.push_str("        #[cfg(target_arch = \"x86_64\")]\n");
                self.output.push_str("        unsafe {\n");
                self.output.push_str("            std::arch::x86_64::_mm_prefetch(\n");
                self.output.push_str("                self as *const Self as *const i8,\n");
                self.output.push_str("                std::arch::x86_64::_MM_HINT_T0,\n");
                self.output.push_str("            );\n");
                self.output.push_str("        }\n");
                self.output.push_str("        #[cfg(target_arch = \"aarch64\")]\n");
                self.output.push_str("        unsafe {\n");
                self.output.push_str("            std::arch::aarch64::_prefetch(\n");
                self.output.push_str("                self as *const Self as *const i8,\n");
                self.output.push_str("                0, // read\n");
                self.output.push_str("                3, // L1 cache\n");
                self.output.push_str("            );\n");
                self.output.push_str("        }\n");
                self.output.push_str("    }\n");
                self.output.push_str("}\n\n");
            }

            // Emit streaming (non-temporal store) impl block for @streaming structs
            if info.is_streaming {
                self.output.push_str(&format!("impl {} {{\n", info.name));
                for (field_name, rust_type) in &info.fields {
                    self.output.push_str(&format!(
                        "    /// Write `{}` using non-temporal store (bypasses cache).\n",
                        field_name
                    ));
                    self.output.push_str("    #[inline(always)]\n");
                    self.output.push_str(&format!(
                        "    pub fn stream_{}(&mut self, val: {}) {{\n",
                        field_name, rust_type
                    ));
                    self.output.push_str("        unsafe {\n");
                    self.output.push_str(&format!(
                        "            let ptr = &mut self.{} as *mut {};\n",
                        field_name, rust_type
                    ));
                    self.output.push_str("            #[cfg(target_arch = \"x86_64\")]\n");
                    self.output.push_str("            {\n");
                    self.output.push_str("                // Use write_volatile as a portable approximation of NT store.\n");
                    self.output.push_str("                // True NT stores require SSE streaming intrinsics on aligned data.\n");
                    self.output.push_str("                std::ptr::write_volatile(ptr, val);\n");
                    self.output.push_str("            }\n");
                    self.output.push_str("            #[cfg(not(target_arch = \"x86_64\"))]\n");
                    self.output.push_str("            {\n");
                    self.output.push_str("                std::ptr::write_volatile(ptr, val);\n");
                    self.output.push_str("            }\n");
                    self.output.push_str("        }\n");
                    self.output.push_str("    }\n");
                }
                self.output.push_str("\n");
                self.output.push_str("    /// Memory fence ensuring all streaming writes are visible.\n");
                self.output.push_str("    #[inline(always)]\n");
                self.output.push_str("    pub fn stream_fence() {\n");
                self.output.push_str("        #[cfg(target_arch = \"x86_64\")]\n");
                self.output.push_str("        unsafe {\n");
                self.output.push_str("            std::arch::x86_64::_mm_sfence();\n");
                self.output.push_str("        }\n");
                self.output.push_str("    }\n");
                self.output.push_str("}\n\n");
            }

            // Emit pool module for @pool(N) structs: pre-allocated slab with O(1) alloc/free
            if let Some(pool_size) = info.is_pool {
                self.output.push_str(&format!("mod pool_{} {{\n", info.name));
                self.output.push_str(&format!("    use super::{};\n", info.name));
                self.output.push_str("    use std::sync::atomic::{AtomicUsize, Ordering};\n");
                self.output.push_str("\n");
                self.output.push_str(&format!("    const POOL_SIZE: usize = {};\n", pool_size));
                self.output.push_str(&format!(
                    "    static mut SLAB: [std::mem::MaybeUninit<{}>; POOL_SIZE] =\n",
                    info.name
                ));
                self.output.push_str("        unsafe { std::mem::MaybeUninit::uninit().assume_init() };\n");
                self.output.push_str("    static mut FREE_LIST: [usize; POOL_SIZE] = [0; POOL_SIZE];\n");
                self.output.push_str("    static FREE_TOP: AtomicUsize = AtomicUsize::new(POOL_SIZE);\n");
                self.output.push_str("\n");
                self.output.push_str("    /// Initialize the pool (must be called once before use).\n");
                self.output.push_str("    pub fn init() {\n");
                self.output.push_str("        unsafe {\n");
                self.output.push_str("            for i in 0..POOL_SIZE {\n");
                self.output.push_str("                FREE_LIST[i] = i;\n");
                self.output.push_str("            }\n");
                self.output.push_str("        }\n");
                self.output.push_str("    }\n");
                self.output.push_str("\n");
                self.output.push_str("    /// Allocate a slot from the pool. Returns None if pool is exhausted.\n");
                self.output.push_str(&format!(
                    "    pub fn alloc() -> Option<&'static mut {}> {{\n",
                    info.name
                ));
                self.output.push_str("        let top = FREE_TOP.fetch_sub(1, Ordering::Relaxed);\n");
                self.output.push_str("        if top == 0 {\n");
                self.output.push_str("            FREE_TOP.fetch_add(1, Ordering::Relaxed);\n");
                self.output.push_str("            return None;\n");
                self.output.push_str("        }\n");
                self.output.push_str("        unsafe {\n");
                self.output.push_str("            let idx = FREE_LIST[top - 1];\n");
                self.output.push_str("            Some(&mut *SLAB[idx].as_mut_ptr())\n");
                self.output.push_str("        }\n");
                self.output.push_str("    }\n");
                self.output.push_str("\n");
                self.output.push_str("    /// Return a slot to the pool.\n");
                self.output.push_str(&format!(
                    "    pub fn free(slot: &mut {}) {{\n",
                    info.name
                ));
                self.output.push_str("        unsafe {\n");
                self.output.push_str("            let base = SLAB.as_ptr() as usize;\n");
                self.output.push_str(&format!(
                    "            let ptr = slot as *mut {} as usize;\n",
                    info.name
                ));
                self.output.push_str(&format!(
                    "            let idx = (ptr - base) / std::mem::size_of::<{}>();\n",
                    info.name
                ));
                self.output.push_str("            let top = FREE_TOP.fetch_add(1, Ordering::Relaxed);\n");
                self.output.push_str("            FREE_LIST[top] = idx;\n");
                self.output.push_str("        }\n");
                self.output.push_str("    }\n");
                self.output.push_str("}\n\n");
            }

            // Emit SoA (Struct-of-Arrays) companion container for @soa structs
            if info.is_soa {
                let soa_name = format!("{}SoA", info.name);

                // Doc comment
                self.output.push_str(&format!(
                    "/// SoA (Struct-of-Arrays) container for {}.\n",
                    info.name
                ));
                self.output.push_str(&format!(
                    "/// Instead of `[{}; N]`, use `{}::with_capacity(N)` for\n",
                    info.name, soa_name
                ));
                self.output.push_str("/// cache-friendly per-field iteration and SIMD vectorization.\n");

                // Struct definition
                self.output.push_str(&format!("pub struct {} {{\n", soa_name));
                for (field_name, rust_type) in &info.fields {
                    self.output.push_str(&format!(
                        "    pub {}: Vec<{}>,\n",
                        field_name, rust_type
                    ));
                }
                self.output.push_str("    pub len: usize,\n");
                self.output.push_str("}\n\n");

                // impl block
                self.output.push_str(&format!("impl {} {{\n", soa_name));

                // with_capacity constructor
                self.output.push_str("    pub fn with_capacity(cap: usize) -> Self {\n");
                self.output.push_str("        Self {\n");
                for (field_name, _rust_type) in &info.fields {
                    let field_type = info.field_types.iter().find(|(n, _)| n == field_name);
                    let zero_val = match field_type {
                        Some((_, FluxType::Float)) => "0.0",
                        Some((_, FluxType::Int)) => "0",
                        Some((_, FluxType::Bool)) => "false",
                        _ => "Default::default()",
                    };
                    self.output.push_str(&format!(
                        "            {}: vec![{}; cap],\n",
                        field_name, zero_val
                    ));
                }
                self.output.push_str("            len: 0,\n");
                self.output.push_str("        }\n");
                self.output.push_str("    }\n\n");

                // push method
                self.output.push_str(&format!(
                    "    pub fn push(&mut self, item: {}) {{\n",
                    info.name
                ));
                self.output.push_str("        let idx = self.len;\n");
                for (field_name, _) in &info.fields {
                    self.output.push_str(&format!(
                        "        self.{}[idx] = item.{};\n",
                        field_name, field_name
                    ));
                }
                self.output.push_str("        self.len += 1;\n");
                self.output.push_str("    }\n\n");

                // get method
                self.output.push_str(&format!(
                    "    pub fn get(&self, idx: usize) -> {} {{\n",
                    info.name
                ));
                self.output.push_str(&format!("        {} {{\n", info.name));
                for (field_name, _) in &info.fields {
                    self.output.push_str(&format!(
                        "            {}: self.{}[idx],\n",
                        field_name, field_name
                    ));
                }
                self.output.push_str("        }\n");
                self.output.push_str("    }\n");

                self.output.push_str("}\n\n");
            }

            // Emit @hot/@cold field-level split: cache-line-separated sub-structs
            let hot_fields: Vec<&(String, String)> = info
                .fields
                .iter()
                .filter(|(name, _)| {
                    info.field_decorators
                        .iter()
                        .any(|(n, decs)| n == name && decs.contains(&"hot".to_string()))
                })
                .collect();
            let cold_fields: Vec<&(String, String)> = info
                .fields
                .iter()
                .filter(|(name, _)| {
                    info.field_decorators
                        .iter()
                        .any(|(n, decs)| n == name && decs.contains(&"cold".to_string()))
                })
                .collect();

            if !hot_fields.is_empty() || !cold_fields.is_empty() {
                let hot_name = format!("{}_Hot", info.name);
                let cold_name = format!("{}_Cold", info.name);

                self.output.push_str(
                    "// @hot/@cold split: frequently-accessed fields grouped for cache efficiency\n"
                );

                // Emit _Hot sub-struct (aligned to 64-byte cache line)
                if !hot_fields.is_empty() {
                    self.output.push_str("#[derive(Clone, Copy)]\n");
                    self.output.push_str("#[repr(align(64))]\n");
                    self.output.push_str(&format!("pub struct {} {{\n", hot_name));
                    for (field_name, rust_type) in &hot_fields {
                        self.output
                            .push_str(&format!("    pub {}: {},\n", field_name, rust_type));
                    }
                    self.output.push_str("}\n\n");
                }

                // Emit _Cold sub-struct
                if !cold_fields.is_empty() {
                    self.output.push_str("#[derive(Clone, Copy)]\n");
                    self.output.push_str(&format!("pub struct {} {{\n", cold_name));
                    for (field_name, rust_type) in &cold_fields {
                        self.output
                            .push_str(&format!("    pub {}: {},\n", field_name, rust_type));
                    }
                    self.output.push_str("}\n\n");
                }

                // Emit split() method on original struct
                if !hot_fields.is_empty() && !cold_fields.is_empty() {
                    self.output.push_str(&format!("impl {} {{\n", info.name));
                    self.output.push_str(&format!(
                        "    pub fn split(&self) -> ({}, {}) {{\n",
                        hot_name, cold_name
                    ));
                    self.output.push_str("        (\n");
                    // Hot struct init
                    self.output.push_str(&format!("            {} {{\n", hot_name));
                    for (field_name, _) in &hot_fields {
                        self.output.push_str(&format!(
                            "                {}: self.{},\n",
                            field_name, field_name
                        ));
                    }
                    self.output.push_str("            },\n");
                    // Cold struct init
                    self.output.push_str(&format!("            {} {{\n", cold_name));
                    for (field_name, _) in &cold_fields {
                        self.output.push_str(&format!(
                            "                {}: self.{},\n",
                            field_name, field_name
                        ));
                    }
                    self.output.push_str("            },\n");
                    self.output.push_str("        )\n");
                    self.output.push_str("    }\n");
                    self.output.push_str("}\n\n");
                } else if !hot_fields.is_empty() {
                    // Only hot fields — emit split that returns just the hot sub-struct
                    self.output.push_str(&format!("impl {} {{\n", info.name));
                    self.output.push_str(&format!(
                        "    pub fn split_hot(&self) -> {} {{\n",
                        hot_name
                    ));
                    self.output.push_str(&format!("        {} {{\n", hot_name));
                    for (field_name, _) in &hot_fields {
                        self.output.push_str(&format!(
                            "            {}: self.{},\n",
                            field_name, field_name
                        ));
                    }
                    self.output.push_str("        }\n");
                    self.output.push_str("    }\n");
                    self.output.push_str("}\n\n");
                } else {
                    // Only cold fields — emit split that returns just the cold sub-struct
                    self.output.push_str(&format!("impl {} {{\n", info.name));
                    self.output.push_str(&format!(
                        "    pub fn split_cold(&self) -> {} {{\n",
                        cold_name
                    ));
                    self.output.push_str(&format!("        {} {{\n", cold_name));
                    for (field_name, _) in &cold_fields {
                        self.output.push_str(&format!(
                            "            {}: self.{},\n",
                            field_name, field_name
                        ));
                    }
                    self.output.push_str("        }\n");
                    self.output.push_str("    }\n");
                    self.output.push_str("}\n\n");
                }
            }
        }

        Ok(())
    }

    /// Emit all user-defined functions before the strategy struct.
    fn emit_user_functions(&mut self) -> Result<()> {
        let functions = self.program.functions.clone();
        for fn_def in &functions {
            let fn_ctx = self.fn_contexts.get(&fn_def.name).cloned().unwrap_or(FnContext {
                needs_bar_context: false,
                needs_signals: false,
            });
            self.emit_fn_def(fn_def, &fn_ctx)?;
            self.output.push('\n');
        }
        Ok(())
    }

    /// Emit a single user-defined function as a Rust `fn` definition.
    ///
    /// Generates the function signature with typed parameters, optional
    /// `ctx: &BarContext` and `signals: &mut Vec<Signal>` params based on
    /// the function's context requirements, and a return type annotation.
    fn emit_fn_def(&mut self, fn_def: &TypedFnDef, fn_ctx: &FnContext) -> Result<()> {
        // Set up function body context
        self.in_fn_body = true;
        self.fn_params.clear();
        self.local_vars.clear();
        for param in &fn_def.params {
            self.fn_params.insert(param.clone());
        }

        // Emit signature
        let generics = Self::format_type_params(&fn_def.type_params, &fn_def.type_param_bounds);
        self.output.push_str(&format!("fn {}{}(", fn_def.name, generics));
        for (i, param) in fn_def.params.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            let rust_type = if i < fn_def.param_types.len() {
                map_type(&fn_def.param_types[i], fn_def.span.start)?
            } else {
                "f64".to_string()
            };
            self.output.push_str(&format!("{}: {}", param, rust_type));
        }
        if fn_ctx.needs_bar_context {
            if !fn_def.params.is_empty() {
                self.output.push_str(", ");
            }
            self.output.push_str("ctx: &BarContext");
        }
        if fn_ctx.needs_signals {
            if !fn_def.params.is_empty() || fn_ctx.needs_bar_context {
                self.output.push_str(", ");
            }
            self.output.push_str("signals: &mut Vec<Signal>");
        }
        self.output.push(')');

        // Emit return type annotation
        match &fn_def.return_type {
            FluxType::Null | FluxType::Void => {}
            other => {
                let rust_type = map_type(other, fn_def.span.start)?;
                self.output.push_str(&format!(" -> {}", rust_type));
            }
        }

        self.output.push_str(" {\n");

        // Emit body statements
        self.indent_level = 1;
        for stmt in &fn_def.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level = 0;

        self.output.push_str("}\n");

        // Restore context
        self.in_fn_body = false;
        self.fn_params.clear();
        self.local_vars.clear();

        Ok(())
    }

    /// Emit the struct definition with property, param, and state fields.
    fn emit_struct(&mut self) -> Result<()> {
        let strategy_name = &self.program.strategy.name.clone();
        self.output
            .push_str(&format!("pub struct {} {{\n", strategy_name));

        // Collect fields in order: properties, params, state
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::Property(prop) = item {
                let type_str = map_type(&prop.value.resolved_type, prop.span.start)?;
                self.output
                    .push_str(&format!("    pub {}: {},\n", prop.name, type_str));
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::ParamsBlock(pb) = item {
                for param in &pb.params {
                    let type_str = map_type(&param.resolved_type, param.span.start)?;
                    self.output
                        .push_str(&format!("    pub {}: {},\n", param.name, type_str));
                }
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::StateBlock(sb) = item {
                for var in &sb.variables {
                    let type_str = map_type(&var.resolved_type, var.span.start)?;
                    self.output
                        .push_str(&format!("    {}: {},\n", var.name, type_str));
                }
            }
        }

        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit the `impl Default for N { fn default() -> Self { Self { ... } } }` block.
    fn emit_default_impl(&mut self) -> Result<()> {
        let strategy_name = &self.program.strategy.name.clone();
        self.output
            .push_str(&format!("impl Default for {} {{\n", strategy_name));
        self.output.push_str("    fn default() -> Self {\n");
        self.output.push_str("        Self {\n");

        // Emit default values in order: properties, params, state
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::Property(prop) = item {
                self.output
                    .push_str(&format!("            {}: ", prop.name));
                self.emit_expr(&prop.value.clone())?;
                self.output.push_str(",\n");
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::ParamsBlock(pb) = item {
                for param in &pb.params {
                    self.output
                        .push_str(&format!("            {}: ", param.name));
                    self.emit_expr(&param.default_value.clone())?;
                    self.output.push_str(",\n");
                }
            }
        }
        for item in &self.program.strategy.body.clone() {
            if let TypedStrategyItem::StateBlock(sb) = item {
                for var in &sb.variables {
                    self.output
                        .push_str(&format!("            {}: ", var.name));
                    self.emit_expr(&var.initial_value.clone())?;
                    self.output.push_str(",\n");
                }
            }
        }

        self.output.push_str("        }\n");
        self.output.push_str("    }\n");
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit the `impl Strategy for N { ... }` block with event handler methods.
    fn emit_strategy_impl(&mut self) -> Result<()> {
        let strategy_name = &self.program.strategy.name.clone();
        self.output
            .push_str(&format!("impl Strategy for {} {{\n", strategy_name));

        let handlers: Vec<TypedEventHandler> = self
            .program
            .strategy
            .body
            .iter()
            .filter_map(|item| {
                if let TypedStrategyItem::EventHandler(handler) = item {
                    Some(handler.clone())
                } else {
                    None
                }
            })
            .collect();

        for handler in &handlers {
            self.emit_event_handler(handler)?;
        }

        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a single event handler method.
    fn emit_event_handler(&mut self, handler: &TypedEventHandler) -> Result<()> {
        // Clear local vars for fresh handler scope
        self.local_vars.clear();

        // Method signature
        self.output.push_str(&format!(
            "    fn on_{}(&mut self, ctx: &BarContext) -> Vec<Signal> {{\n",
            handler.event_name
        ));

        // Signal declaration
        self.output
            .push_str("        let mut signals: Vec<Signal> = Vec::new();\n");

        // Body statements
        self.indent_level = 2;
        for stmt in &handler.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level = 0;

        // Return signals
        self.output.push_str("        signals\n");

        // Close method
        self.output.push_str("    }\n");
        Ok(())
    }

    // ========================================================================
    // Statement emission (task 3.1)
    // ========================================================================

    /// Dispatch statement emission based on the `TypedStmt` variant.
    fn emit_stmt(&mut self, stmt: &TypedStmt) -> Result<()> {
        match stmt {
            TypedStmt::Assignment(assign) => self.emit_assignment(assign),
            TypedStmt::If(if_stmt) => self.emit_if_stmt(if_stmt),
            TypedStmt::For(for_loop) => self.emit_for_loop(for_loop),
            TypedStmt::While(while_loop) => self.emit_while_loop(while_loop),
            TypedStmt::Return(ret) => self.emit_return(ret),
            TypedStmt::Expr(expr_stmt) => self.emit_expr_stmt(expr_stmt),
        }
    }

    /// Emit an assignment statement.
    ///
    /// Handles four cases:
    /// - Index target: `object[index] = value;`
    /// - State variable: `self.name = value;`
    /// - New local variable: `let mut name = value;`
    /// - Existing local variable: `name = value;`
    fn emit_assignment(&mut self, assign: &TypedAssignment) -> Result<()> {
        self.write_indent();
        match &assign.target.kind {
            TypedExprKind::IndexAccess { object, index } => {
                self.emit_expr(object)?;
                self.output.push('[');
                self.emit_expr(index)?;
                self.output.push_str("] = ");
                self.emit_expr(&assign.value)?;
                self.output.push_str(";\n");
            }
            TypedExprKind::Ident(name) => {
                if !self.in_fn_body && self.state_vars.contains(name) {
                    self.output.push_str(&format!("self.{} = ", name));
                    self.emit_expr(&assign.value)?;
                    self.output.push_str(";\n");
                } else if self.is_new_local(name) {
                    self.output.push_str(&format!("let mut {} = ", name));
                    self.emit_expr(&assign.value)?;
                    self.output.push_str(";\n");
                    self.local_vars.insert(name.clone());
                } else {
                    self.output.push_str(&format!("{} = ", name));
                    self.emit_expr(&assign.value)?;
                    self.output.push_str(";\n");
                }
            }
            _ => {
                // Fallback: emit target expression directly
                self.emit_expr(&assign.target)?;
                self.output.push_str(" = ");
                self.emit_expr(&assign.value)?;
                self.output.push_str(";\n");
            }
        }
        Ok(())
    }

    /// Emit an if/elif/else statement with proper indentation.
    fn emit_if_stmt(&mut self, if_stmt: &TypedIfStmt) -> Result<()> {
        // if condition {
        self.write_indent();
        self.output.push_str("if ");
        self.emit_expr(&if_stmt.condition)?;
        self.output.push_str(" {\n");

        // body
        self.indent_level += 1;
        for stmt in &if_stmt.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level -= 1;

        // elif branches
        for elif in &if_stmt.elif_branches {
            self.write_indent();
            self.output.push_str("} else if ");
            self.emit_expr(&elif.condition)?;
            self.output.push_str(" {\n");

            self.indent_level += 1;
            for stmt in &elif.body {
                self.emit_stmt(stmt)?;
            }
            self.indent_level -= 1;
        }

        // else branch
        if let Some(else_body) = &if_stmt.else_body {
            self.write_indent();
            self.output.push_str("} else {\n");

            self.indent_level += 1;
            for stmt in else_body {
                self.emit_stmt(stmt)?;
            }
            self.indent_level -= 1;
        }

        // closing brace
        self.write_indent();
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a for loop: `for variable in iterable { body }`.
    ///
    /// Registers the loop variable as a local before emitting the body.
    fn emit_for_loop(&mut self, for_loop: &TypedForLoop) -> Result<()> {
        self.write_indent();
        self.output.push_str(&format!("for {} in ", for_loop.variable));
        self.emit_expr(&for_loop.iterable)?;
        self.output.push_str(" {\n");

        // Register loop variable as local
        self.local_vars.insert(for_loop.variable.clone());

        self.indent_level += 1;
        for stmt in &for_loop.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level -= 1;

        self.write_indent();
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a while loop: `while condition { body }`.
    fn emit_while_loop(&mut self, while_loop: &TypedWhileLoop) -> Result<()> {
        self.write_indent();
        self.output.push_str("while ");
        self.emit_expr(&while_loop.condition)?;
        self.output.push_str(" {\n");

        self.indent_level += 1;
        for stmt in &while_loop.body {
            self.emit_stmt(stmt)?;
        }
        self.indent_level -= 1;

        self.write_indent();
        self.output.push_str("}\n");
        Ok(())
    }

    /// Emit a return statement: `return value;` or `return;`.
    fn emit_return(&mut self, ret: &TypedReturnStmt) -> Result<()> {
        self.write_indent();
        if let Some(value) = &ret.value {
            self.output.push_str("return ");
            self.emit_expr(value)?;
            self.output.push_str(";\n");
        } else {
            self.output.push_str("return;\n");
        }
        Ok(())
    }

    /// Emit an expression statement.
    ///
    /// Signal-producing expressions are wrapped in `signals.push(...)`.
    /// Other expressions are emitted as standalone statements.
    fn emit_expr_stmt(&mut self, expr_stmt: &TypedExprStmt) -> Result<()> {
        self.write_indent();
        if self.is_signal_expr(&expr_stmt.expr) {
            self.output.push_str("signals.push(");
            self.emit_expr(&expr_stmt.expr)?;
            self.output.push_str(");\n");
        } else {
            self.emit_expr(&expr_stmt.expr)?;
            self.output.push_str(";\n");
        }
        Ok(())
    }

    // ========================================================================
    // Expression emission
    // ========================================================================

    /// Emit a typed expression to the output buffer.
    pub(crate) fn emit_expr(&mut self, expr: &TypedExpr) -> Result<()> {
        match &expr.kind {
            TypedExprKind::IntLiteral(v) => {
                self.output.push_str(&v.to_string());
            }
            TypedExprKind::FloatLiteral(v) => {
                let s = v.to_string();
                self.output.push_str(&s);
                // Ensure the float literal always contains a decimal point
                if !s.contains('.') {
                    self.output.push_str(".0");
                }
            }
            TypedExprKind::StringLiteral(s) => {
                let escaped = s
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t");
                self.output.push_str(&format!("String::from(\"{}\")", escaped));
            }
            TypedExprKind::BoolLiteral(b) => {
                self.output.push_str(if *b { "true" } else { "false" });
            }
            TypedExprKind::NullLiteral => {
                self.output.push_str("()");
            }
            TypedExprKind::ListLiteral(elems) => {
                self.output.push_str("vec![");
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_expr(elem)?;
                }
                self.output.push(']');
            }
            TypedExprKind::Ident(name) => {
                let resolved = self.resolve_ident(name);
                self.output.push_str(&resolved);
            }
            TypedExprKind::BinaryOp { left, op, right } => {
                self.emit_binary_op(left, *op, right)?;
            }
            TypedExprKind::UnaryOp { op, operand } => {
                self.emit_unary_op(*op, operand)?;
            }
            TypedExprKind::FunctionCall { function, args } => {
                self.emit_function_call(function, args)?;
            }
            TypedExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                self.emit_method_call(receiver, method, args)?;
            }
            TypedExprKind::MemberAccess { object, field } => {
                self.emit_member_access(object, field)?;
            }
            TypedExprKind::IndexAccess { object, index } => {
                self.emit_index_access(object, index)?;
            }
            TypedExprKind::StructLiteral { struct_name, fields } => {
                // Struct literal codegen (task 6.3) — emit Rust struct literal syntax
                self.output.push_str(struct_name);
                self.output.push_str(" { ");
                for (i, (field_name, field_expr)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.output.push_str(field_name);
                    self.output.push_str(": ");
                    self.emit_expr(field_expr)?;
                }
                self.output.push_str(" }");
            }
            TypedExprKind::EnumConstruction {
                enum_name,
                variant_name,
                args,
            } => {
                // Emit Rust enum variant construction: EnumName::Variant { field: val, ... }
                // or EnumName::Variant for unit variants
                self.output.push_str(enum_name);
                self.output.push_str("::");
                self.output.push_str(variant_name);
                if !args.is_empty() {
                    // Look up field names from the enum definition
                    let field_names: Vec<String> = self
                        .program
                        .enums
                        .iter()
                        .find(|e| &e.name == enum_name)
                        .and_then(|e| e.variants.iter().find(|v| &v.name == variant_name))
                        .map(|v| v.fields.iter().map(|(name, _)| name.clone()).collect())
                        .unwrap_or_default();

                    self.output.push_str(" { ");
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.output.push_str(", ");
                        }
                        if let Some(field_name) = field_names.get(i) {
                            self.output.push_str(field_name);
                            self.output.push_str(": ");
                        }
                        self.emit_expr(arg)?;
                    }
                    self.output.push_str(" }");
                }
            }
            TypedExprKind::Match(match_expr) => {
                self.output.push_str("match ");
                self.emit_expr(&match_expr.scrutinee)?;
                self.output.push_str(" {\n");
                let base_indent = "    ".repeat(self.indent_level);
                let arm_indent = format!("{}    ", base_indent);
                let body_indent = format!("{}        ", base_indent);
                for arm in &match_expr.arms {
                    self.output.push_str(&arm_indent);
                    match &arm.pattern {
                        TypedPattern::Variant { enum_name, variant_name, bindings, .. } => {
                            self.output.push_str(enum_name);
                            self.output.push_str("::");
                            self.output.push_str(variant_name);
                            if !bindings.is_empty() {
                                self.output.push_str(" { ");
                                for (i, (name, _)) in bindings.iter().enumerate() {
                                    if i > 0 {
                                        self.output.push_str(", ");
                                    }
                                    self.output.push_str(name);
                                }
                                self.output.push_str(" }");
                            }
                        }
                        TypedPattern::Wildcard { .. } => {
                            self.output.push('_');
                        }
                    }
                    self.output.push_str(" => {\n");
                    for stmt in &arm.body {
                        self.output.push_str(&body_indent);
                        self.emit_stmt(stmt)?;
                        self.output.push('\n');
                    }
                    self.output.push_str(&arm_indent);
                    self.output.push_str("}\n");
                }
                self.output.push_str(&base_indent);
                self.output.push('}');
            }
        }
        Ok(())
    }

    /// Emit a binary operation expression.
    fn emit_binary_op(&mut self, left: &TypedExpr, op: BinOp, right: &TypedExpr) -> Result<()> {
        // Special case: String + String → format!("{}{}", left, right)
        if op == BinOp::Add
            && left.resolved_type == FluxType::String
            && right.resolved_type == FluxType::String
        {
            self.output.push_str("format!(\"{}{}\", ");
            self.emit_expr(left)?;
            self.output.push_str(", ");
            self.emit_expr(right)?;
            self.output.push(')');
            return Ok(());
        }

        let op_str = match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        };

        self.output.push('(');

        // Left operand: cast to f64 if needed
        if self.needs_cast_to_f64(left, &right.resolved_type) {
            self.output.push('(');
            self.emit_expr(left)?;
            self.output.push_str(" as f64)");
        } else {
            self.emit_expr(left)?;
        }

        self.output.push(' ');
        self.output.push_str(op_str);
        self.output.push(' ');

        // Right operand: cast to f64 if needed
        if self.needs_cast_to_f64(right, &left.resolved_type) {
            self.output.push('(');
            self.emit_expr(right)?;
            self.output.push_str(" as f64)");
        } else {
            self.emit_expr(right)?;
        }

        self.output.push(')');
        Ok(())
    }

    /// Emit a unary operation expression.
    fn emit_unary_op(&mut self, op: UnaryOp, operand: &TypedExpr) -> Result<()> {
        match op {
            UnaryOp::Neg => {
                self.output.push_str("(-");
                self.emit_expr(operand)?;
                self.output.push(')');
            }
            UnaryOp::Not => {
                self.output.push_str("(!");
                self.emit_expr(operand)?;
                self.output.push(')');
            }
        }
        Ok(())
    }

    /// Emit a function call expression.
    ///
    /// Handles signal functions (OPEN, CLOSE) specially, mapping them to
    /// the runtime Signal API. User-defined functions get `ctx` and/or
    /// `&mut signals` forwarded based on their FnContext. Other functions
    /// are emitted as direct calls.
    fn emit_function_call(&mut self, function: &TypedExpr, args: &[TypedExpr]) -> Result<()> {
        // Check if this is a signal function call or user-defined function
        if let TypedExprKind::Ident(ref name) = function.kind {
            match name.as_str() {
                "OPEN" => {
                    self.output.push_str("Signal::open(");
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.output.push_str(", ");
                        }
                        self.emit_expr(arg)?;
                    }
                    self.output.push(')');
                    return Ok(());
                }
                "CLOSE" => {
                    if args.len() == 1 {
                        self.output.push_str("Signal::close(");
                        self.emit_expr(&args[0])?;
                        self.output.push(')');
                    } else {
                        // 2 args → Signal::close_qty
                        self.output.push_str("Signal::close_qty(");
                        self.emit_expr(&args[0])?;
                        self.output.push_str(", ");
                        self.emit_expr(&args[1])?;
                        self.output.push(')');
                    }
                    return Ok(());
                }
                _ => {}
            }

            // Check if it's a user-defined function — forward ctx and/or &mut signals
            if let Some(fn_ctx) = self.fn_contexts.get(name).cloned() {
                self.output.push_str(name);
                self.output.push('(');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_expr(arg)?;
                }
                if fn_ctx.needs_bar_context {
                    if !args.is_empty() {
                        self.output.push_str(", ");
                    }
                    self.output.push_str("ctx");
                }
                if fn_ctx.needs_signals {
                    if !args.is_empty() || fn_ctx.needs_bar_context {
                        self.output.push_str(", ");
                    }
                    // Inside a fn body, `signals` is already `&mut Vec<Signal>`
                    // In an event handler, `signals` is `Vec<Signal>` so we need `&mut`
                    if self.in_fn_body {
                        self.output.push_str("signals");
                    } else {
                        self.output.push_str("&mut signals");
                    }
                }
                self.output.push(')');
                return Ok(());
            }
        }

        // Regular function call (built-ins, indicators): name(args...)
        self.emit_expr(function)?;
        self.output.push('(');
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            self.emit_expr(arg)?;
        }
        self.output.push(')');
        Ok(())
    }

    /// Emit a method call: `receiver.method(args...)`.
    ///
    /// Handles HashMap specially:
    /// - `HashMap.new()` → `std::collections::HashMap::new()`
    /// - `map.insert(k, v)` → `map.insert(k, v)` (direct pass-through)
    /// - `map.get(k)` → `map.get(&k).cloned().unwrap()`
    /// - `map.contains_key(k)` → `map.contains_key(&k)`
    /// - `map.remove(k)` → `map.remove(&k)`
    fn emit_method_call(
        &mut self,
        receiver: &TypedExpr,
        method: &str,
        args: &[TypedExpr],
    ) -> Result<()> {
        // Handle HashMap.new() static constructor
        if method == "new" {
            if let TypedExprKind::Ident(ref name) = receiver.kind {
                if name == "HashMap" {
                    self.output.push_str("std::collections::HashMap::new()");
                    return Ok(());
                }
            }
        }

        // Handle method calls on HashMap receivers
        if let FluxType::Generic(ref type_name, _) = receiver.resolved_type {
            if type_name == "HashMap" {
                return self.emit_hashmap_method_call(receiver, method, args);
            }
        }

        // Default: receiver.method(args...)
        self.emit_expr(receiver)?;
        self.output.push('.');
        self.output.push_str(method);
        self.output.push('(');
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            self.emit_expr(arg)?;
        }
        self.output.push(')');
        Ok(())
    }

    /// Emit a method call on a HashMap receiver with Rust-appropriate semantics.
    fn emit_hashmap_method_call(
        &mut self,
        receiver: &TypedExpr,
        method: &str,
        args: &[TypedExpr],
    ) -> Result<()> {
        match method {
            "insert" => {
                // Rust: map.insert(key, value) — takes owned key and value
                self.emit_expr(receiver)?;
                self.output.push_str(".insert(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_expr(arg)?;
                }
                self.output.push(')');
            }
            "get" => {
                // Rust: map.get(&key) returns Option<&V>, so we unwrap with cloned
                self.emit_expr(receiver)?;
                self.output.push_str(".get(&");
                if let Some(arg) = args.first() {
                    self.emit_expr(arg)?;
                }
                self.output.push_str(").cloned().unwrap()");
            }
            "contains_key" => {
                // Rust: map.contains_key(&key)
                self.emit_expr(receiver)?;
                self.output.push_str(".contains_key(&");
                if let Some(arg) = args.first() {
                    self.emit_expr(arg)?;
                }
                self.output.push(')');
            }
            "remove" => {
                // Rust: map.remove(&key)
                self.emit_expr(receiver)?;
                self.output.push_str(".remove(&");
                if let Some(arg) = args.first() {
                    self.emit_expr(arg)?;
                }
                self.output.push(')');
            }
            _ => {
                // Fallback for unknown HashMap methods
                self.emit_expr(receiver)?;
                self.output.push('.');
                self.output.push_str(method);
                self.output.push('(');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_expr(arg)?;
                }
                self.output.push(')');
            }
        }
        Ok(())
    }

    /// Emit a member access: `object.field`.
    fn emit_member_access(&mut self, object: &TypedExpr, field: &str) -> Result<()> {
        self.emit_expr(object)?;
        self.output.push('.');
        self.output.push_str(field);
        Ok(())
    }

    /// Emit an index access: `object[index]`.
    fn emit_index_access(&mut self, object: &TypedExpr, index: &TypedExpr) -> Result<()> {
        self.emit_expr(object)?;
        self.output.push('[');
        self.emit_expr(index)?;
        self.output.push(']');
        Ok(())
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Resolve an identifier to its correct Rust expression form.
    ///
    /// Priority: MARKET_DATA → local_vars → params/state/properties → bare name
    /// When in a function body: fn_params → MARKET_DATA → local_vars → bare name
    fn resolve_ident(&self, name: &str) -> String {
        if self.in_fn_body {
            // Inside a user-defined function body:
            // Function parameters are bare names
            if self.fn_params.contains(name) {
                return name.to_string();
            }
            // Market data resolves to ctx.X
            if MARKET_DATA.contains(&name) {
                return format!("ctx.{}", name);
            }
            // Local variables are bare names
            if self.local_vars.contains(name) {
                return name.to_string();
            }
            // Everything else is a bare name (imported functions, etc.)
            name.to_string()
        } else {
            // Inside strategy event handler:
            if MARKET_DATA.contains(&name) {
                format!("ctx.{}", name)
            } else if self.local_vars.contains(name) {
                name.to_string()
            } else if self.params.contains(name)
                || self.state_vars.contains(name)
                || self.properties.contains(name)
            {
                format!("self.{}", name)
            } else {
                // Imported function or unknown — emit bare name
                name.to_string()
            }
        }
    }

    /// Write indentation (4 spaces per level) to the output buffer.
    #[allow(dead_code)]
    fn write_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.output.push_str("    ");
        }
    }

    /// Check if an expression needs to be cast to f64.
    ///
    /// Returns true if `expr` has type Int and the `other_type` is Float,
    /// meaning this operand needs an `as f64` cast for the binary op.
    fn needs_cast_to_f64(&self, expr: &TypedExpr, other_type: &FluxType) -> bool {
        expr.resolved_type == FluxType::Int && *other_type == FluxType::Float
    }

    /// Check if an expression produces a Signal value.
    #[allow(dead_code)]
    pub(crate) fn is_signal_expr(&self, expr: &TypedExpr) -> bool {
        expr.resolved_type == FluxType::Signal
    }

    /// Check if a name is a new local variable (not yet declared).
    ///
    /// Returns true if the name is NOT in params, state_vars, properties,
    /// fn_params, or local_vars — i.e., it's being assigned for the first time.
    #[allow(dead_code)]
    pub(crate) fn is_new_local(&self, name: &str) -> bool {
        !self.params.contains(name)
            && !self.state_vars.contains(name)
            && !self.properties.contains(name)
            && !self.local_vars.contains(name)
            && !self.fn_params.contains(name)
    }

    /// Construct a codegen error with byte offset.
    #[allow(dead_code)]
    pub(crate) fn error(&self, message: &str, span_start: usize) -> CompileError {
        CompileError::Codegen(format!("at byte {}: {}", span_start, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;
    use crate::parser::ast::Import;

    /// Helper: build a minimal TypedProgram with no params, state, or handlers.
    fn minimal_program() -> TypedProgram {
        TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "Test".to_string(),
                body: vec![],
                span: Span::new(0, 10),
            },
            span: Span::new(0, 10),
        }
    }

    /// Helper: build a TypedProgram with params, state, properties, and imports.
    fn full_context_program() -> TypedProgram {
        TypedProgram {
            imports: vec![Import {
                module_path: "indicators".to_string(),
                names: vec!["sma".to_string(), "ema".to_string()],
                span: Span::new(0, 30),
            }],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "MyStrategy".to_string(),
                body: vec![
                    TypedStrategyItem::Property(TypedProperty {
                        name: "book_side".to_string(),
                        value: TypedExpr {
                            kind: TypedExprKind::IntLiteral(1),
                            resolved_type: FluxType::Int,
                            span: Span::new(40, 41),
                        },
                        span: Span::new(35, 45),
                    }),
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "period".to_string(),
                            default_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(20),
                                resolved_type: FluxType::Int,
                                span: Span::new(60, 62),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(50, 62),
                        }],
                        span: Span::new(48, 65),
                    }),
                    TypedStrategyItem::StateBlock(TypedStateBlock {
                        variables: vec![TypedStateVar {
                            name: "count".to_string(),
                            initial_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(0),
                                resolved_type: FluxType::Int,
                                span: Span::new(80, 81),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(70, 81),
                        }],
                        span: Span::new(68, 85),
                    }),
                ],
                span: Span::new(32, 100),
            },
            span: Span::new(0, 100),
        }
    }

    /// Helper: create a typed expression with a given kind and type.
    fn typed_expr(kind: TypedExprKind, resolved_type: FluxType) -> TypedExpr {
        TypedExpr {
            kind,
            resolved_type,
            span: Span::new(0, 1),
        }
    }

    // ===== Constructor tests =====

    #[test]
    fn new_collects_params() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.params.contains("period"));
    }

    #[test]
    fn new_collects_state_vars() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.state_vars.contains("count"));
    }

    #[test]
    fn new_collects_properties() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.properties.contains("book_side"));
    }

    #[test]
    fn new_collects_imported_functions() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.imported_functions.contains("sma"));
        assert!(emitter.imported_functions.contains("ema"));
    }

    // ===== resolve_ident tests =====

    #[test]
    fn resolve_ident_market_data() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("close"), "ctx.close");
        assert_eq!(emitter.resolve_ident("open"), "ctx.open");
        assert_eq!(emitter.resolve_ident("high"), "ctx.high");
        assert_eq!(emitter.resolve_ident("low"), "ctx.low");
        assert_eq!(emitter.resolve_ident("volume"), "ctx.volume");
        assert_eq!(emitter.resolve_ident("symbol"), "ctx.symbol");
        assert_eq!(emitter.resolve_ident("in_position"), "ctx.in_position");
    }

    #[test]
    fn resolve_ident_local_var() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("zscore".to_string());
        assert_eq!(emitter.resolve_ident("zscore"), "zscore");
    }

    #[test]
    fn resolve_ident_param() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("period"), "self.period");
    }

    #[test]
    fn resolve_ident_state_var() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("count"), "self.count");
    }

    #[test]
    fn resolve_ident_property() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("book_side"), "self.book_side");
    }

    #[test]
    fn resolve_ident_imported_function() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("sma"), "sma");
    }

    #[test]
    fn resolve_ident_unknown() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        assert_eq!(emitter.resolve_ident("unknown_thing"), "unknown_thing");
    }

    #[test]
    fn resolve_ident_local_takes_priority_over_param() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        // If a local shadows a param, local wins
        emitter.local_vars.insert("period".to_string());
        assert_eq!(emitter.resolve_ident("period"), "period");
    }

    // ===== emit_expr tests: Literals =====

    #[test]
    fn emit_int_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "42");
    }

    #[test]
    fn emit_int_literal_negative() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(-7), FluxType::Int);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "-7");
    }

    #[test]
    fn emit_float_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::FloatLiteral(3.14), FluxType::Float);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "3.14");
    }

    #[test]
    fn emit_float_literal_whole_number() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::FloatLiteral(2.0), FluxType::Float);
        emitter.emit_expr(&expr).unwrap();
        // Must contain a decimal point
        assert!(emitter.output.contains('.'), "Float must contain decimal point");
        assert_eq!(emitter.output, "2.0");
    }

    #[test]
    fn emit_string_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::StringLiteral("hello".to_string()),
            FluxType::String,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "String::from(\"hello\")");
    }

    #[test]
    fn emit_string_literal_with_escaping() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::StringLiteral("say \"hi\"\nnewline".to_string()),
            FluxType::String,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "String::from(\"say \\\"hi\\\"\\nnewline\")"
        );
    }

    #[test]
    fn emit_bool_true() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "true");
    }

    #[test]
    fn emit_bool_false() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "false");
    }

    #[test]
    fn emit_null_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::NullLiteral, FluxType::Null);
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "()");
    }

    #[test]
    fn emit_list_literal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::ListLiteral(vec![
                typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
            ]),
            FluxType::List(Box::new(FluxType::Int)),
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "vec![1, 2, 3]");
    }

    #[test]
    fn emit_list_literal_empty() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::ListLiteral(vec![]),
            FluxType::List(Box::new(FluxType::Null)),
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "vec![]");
    }

    // ===== emit_expr tests: Identifiers =====

    #[test]
    fn emit_ident_market_data() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("close".to_string()),
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "ctx.close");
    }

    #[test]
    fn emit_ident_param() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("period".to_string()),
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "self.period");
    }

    #[test]
    fn emit_ident_state_var() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("count".to_string()),
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "self.count");
    }

    #[test]
    fn emit_ident_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("zscore".to_string());
        let expr = typed_expr(
            TypedExprKind::Ident("zscore".to_string()),
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "zscore");
    }

    #[test]
    fn emit_ident_imported_function() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::Ident("sma".to_string()),
            FluxType::Fn {
                params: crate::typeck::types::FnParams::VariadicNumeric,
                ret: Box::new(FluxType::Float),
            },
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "sma");
    }

    // ===== emit_expr tests: Binary Operations =====

    #[test]
    fn emit_binary_add() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Add,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 + 2)");
    }

    #[test]
    fn emit_binary_sub() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int)),
                op: BinOp::Sub,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(5 - 3)");
    }

    #[test]
    fn emit_binary_mul() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(4), FluxType::Int)),
                op: BinOp::Mul,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(6), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(4 * 6)");
    }

    #[test]
    fn emit_binary_div() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int)),
                op: BinOp::Div,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(10 / 2)");
    }

    #[test]
    fn emit_binary_mod() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int)),
                op: BinOp::Mod,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(10 % 3)");
    }

    #[test]
    fn emit_binary_eq() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Eq,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 == 1)");
    }

    #[test]
    fn emit_binary_ne() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Ne,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 != 2)");
    }

    #[test]
    fn emit_binary_lt() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Lt,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 < 2)");
    }

    #[test]
    fn emit_binary_le() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Le,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1 <= 2)");
    }

    #[test]
    fn emit_binary_gt() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
                op: BinOp::Gt,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(2 > 1)");
    }

    #[test]
    fn emit_binary_ge() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int)),
                op: BinOp::Ge,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(2 >= 1)");
    }

    #[test]
    fn emit_binary_and() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool)),
                op: BinOp::And,
                right: Box::new(typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(true && false)");
    }

    #[test]
    fn emit_binary_or() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool)),
                op: BinOp::Or,
                right: Box::new(typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(true || false)");
    }

    // ===== Numeric coercion =====

    #[test]
    fn emit_binary_int_plus_float_casts_left() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
                op: BinOp::Add,
                right: Box::new(typed_expr(TypedExprKind::FloatLiteral(2.5), FluxType::Float)),
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "((1 as f64) + 2.5)");
    }

    #[test]
    fn emit_binary_float_plus_int_casts_right() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::FloatLiteral(2.5), FluxType::Float)),
                op: BinOp::Add,
                right: Box::new(typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int)),
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(2.5 + (1 as f64))");
    }

    #[test]
    fn emit_binary_same_type_no_cast() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(TypedExprKind::FloatLiteral(1.0), FluxType::Float)),
                op: BinOp::Mul,
                right: Box::new(typed_expr(TypedExprKind::FloatLiteral(2.0), FluxType::Float)),
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(1.0 * 2.0)");
        assert!(!emitter.output.contains("as f64"));
    }

    // ===== String concatenation =====

    #[test]
    fn emit_string_concat_uses_format() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::BinaryOp {
                left: Box::new(typed_expr(
                    TypedExprKind::StringLiteral("hello".to_string()),
                    FluxType::String,
                )),
                op: BinOp::Add,
                right: Box::new(typed_expr(
                    TypedExprKind::StringLiteral(" world".to_string()),
                    FluxType::String,
                )),
            },
            FluxType::String,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "format!(\"{}{}\", String::from(\"hello\"), String::from(\" world\"))"
        );
        assert!(!emitter.output.contains(" + "));
    }

    // ===== Unary operations =====

    #[test]
    fn emit_unary_neg() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::UnaryOp {
                op: UnaryOp::Neg,
                operand: Box::new(typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(-5)");
    }

    #[test]
    fn emit_unary_not() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool)),
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "(!true)");
    }

    // ===== Function calls =====

    #[test]
    fn emit_function_call_indicator() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("sma".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::VariadicNumeric,
                        ret: Box::new(FluxType::Float),
                    },
                )),
                args: vec![
                    typed_expr(TypedExprKind::Ident("close".to_string()), FluxType::Float),
                    typed_expr(TypedExprKind::IntLiteral(20), FluxType::Int),
                ],
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "sma(ctx.close, 20)");
    }

    #[test]
    fn emit_function_call_open() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("OPEN".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::Fixed(vec![
                            FluxType::String,
                            FluxType::Int,
                        ]),
                        ret: Box::new(FluxType::Signal),
                    },
                )),
                args: vec![
                    typed_expr(TypedExprKind::Ident("symbol".to_string()), FluxType::String),
                    typed_expr(TypedExprKind::IntLiteral(100), FluxType::Int),
                ],
            },
            FluxType::Signal,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "Signal::open(ctx.symbol, 100)");
    }

    #[test]
    fn emit_function_call_close_one_arg() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("CLOSE".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::Fixed(vec![FluxType::String]),
                        ret: Box::new(FluxType::Signal),
                    },
                )),
                args: vec![typed_expr(
                    TypedExprKind::Ident("symbol".to_string()),
                    FluxType::String,
                )],
            },
            FluxType::Signal,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "Signal::close(ctx.symbol)");
    }

    #[test]
    fn emit_function_call_close_two_args() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::FunctionCall {
                function: Box::new(typed_expr(
                    TypedExprKind::Ident("CLOSE".to_string()),
                    FluxType::Fn {
                        params: crate::typeck::types::FnParams::Fixed(vec![
                            FluxType::String,
                            FluxType::Int,
                        ]),
                        ret: Box::new(FluxType::Signal),
                    },
                )),
                args: vec![
                    typed_expr(TypedExprKind::Ident("symbol".to_string()), FluxType::String),
                    typed_expr(TypedExprKind::IntLiteral(50), FluxType::Int),
                ],
            },
            FluxType::Signal,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "Signal::close_qty(ctx.symbol, 50)");
    }

    // ===== Method calls, member access, index access =====

    #[test]
    fn emit_method_call() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("count".to_string()),
                    FluxType::Int,
                )),
                method: "abs".to_string(),
                args: vec![],
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "self.count.abs()");
    }

    #[test]
    fn emit_method_call_with_args() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("prices".to_string());
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("prices".to_string()),
                    FluxType::List(Box::new(FluxType::Float)),
                )),
                method: "append".to_string(),
                args: vec![typed_expr(TypedExprKind::FloatLiteral(1.5), FluxType::Float)],
            },
            FluxType::Void,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "prices.append(1.5)");
    }

    #[test]
    fn emit_member_access() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("obj".to_string());
        let expr = typed_expr(
            TypedExprKind::MemberAccess {
                object: Box::new(typed_expr(
                    TypedExprKind::Ident("obj".to_string()),
                    FluxType::Int, // type doesn't matter here
                )),
                field: "x".to_string(),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "obj.x");
    }

    #[test]
    fn emit_index_access() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("arr".to_string());
        let expr = typed_expr(
            TypedExprKind::IndexAccess {
                object: Box::new(typed_expr(
                    TypedExprKind::Ident("arr".to_string()),
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                index: Box::new(typed_expr(TypedExprKind::IntLiteral(0), FluxType::Int)),
            },
            FluxType::Int,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "arr[0]");
    }

    // ===== Helper method tests =====

    #[test]
    fn is_new_local_true_for_unknown() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(emitter.is_new_local("brand_new"));
    }

    #[test]
    fn is_new_local_false_for_param() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(!emitter.is_new_local("period"));
    }

    #[test]
    fn is_new_local_false_for_state() {
        let prog = full_context_program();
        let emitter = CodeEmitter::new(&prog);
        assert!(!emitter.is_new_local("count"));
    }

    #[test]
    fn is_new_local_false_for_existing_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("x".to_string());
        assert!(!emitter.is_new_local("x"));
    }

    #[test]
    fn is_signal_expr_true() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::NullLiteral, FluxType::Signal);
        assert!(emitter.is_signal_expr(&expr));
    }

    #[test]
    fn is_signal_expr_false() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int);
        assert!(!emitter.is_signal_expr(&expr));
    }

    #[test]
    fn error_helper_format() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let err = emitter.error("something went wrong", 42);
        match err {
            CompileError::Codegen(msg) => {
                assert_eq!(msg, "at byte 42: something went wrong");
            }
            _ => panic!("Expected Codegen error"),
        }
    }

    #[test]
    fn needs_cast_to_f64_int_vs_float() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int);
        assert!(emitter.needs_cast_to_f64(&expr, &FluxType::Float));
    }

    #[test]
    fn needs_cast_to_f64_float_vs_float() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::FloatLiteral(1.0), FluxType::Float);
        assert!(!emitter.needs_cast_to_f64(&expr, &FluxType::Float));
    }

    #[test]
    fn needs_cast_to_f64_int_vs_int() {
        let prog = minimal_program();
        let emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int);
        assert!(!emitter.needs_cast_to_f64(&expr, &FluxType::Int));
    }

    // ===== Statement emission tests (task 3.1) =====

    #[test]
    fn emit_stmt_dispatches_assignment() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let stmt = TypedStmt::Assignment(TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int),
            span: Span::new(0, 10),
        });
        emitter.emit_stmt(&stmt).unwrap();
        assert_eq!(emitter.output, "let mut x = 42;\n");
    }

    #[test]
    fn emit_assignment_new_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("zscore".to_string()), FluxType::Float),
            value: typed_expr(TypedExprKind::FloatLiteral(1.5), FluxType::Float),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "let mut zscore = 1.5;\n");
        assert!(emitter.local_vars.contains("zscore"));
    }

    #[test]
    fn emit_assignment_existing_local() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("x".to_string());
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "x = 10;\n");
    }

    #[test]
    fn emit_assignment_state_var() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("count".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "self.count = 5;\n");
    }

    #[test]
    fn emit_assignment_index_target() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("arr".to_string());
        let assign = TypedAssignment {
            target: typed_expr(
                TypedExprKind::IndexAccess {
                    object: Box::new(typed_expr(
                        TypedExprKind::Ident("arr".to_string()),
                        FluxType::List(Box::new(FluxType::Int)),
                    )),
                    index: Box::new(typed_expr(TypedExprKind::IntLiteral(0), FluxType::Int)),
                },
                FluxType::Int,
            ),
            value: typed_expr(TypedExprKind::IntLiteral(99), FluxType::Int),
            span: Span::new(0, 10),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "arr[0] = 99;\n");
    }

    #[test]
    fn emit_if_stmt_simple() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let if_stmt = TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(0, 20),
        };
        emitter.emit_if_stmt(&if_stmt).unwrap();
        assert_eq!(emitter.output, "if true {\n    1;\n}\n");
    }

    #[test]
    fn emit_if_stmt_with_else() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let if_stmt = TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            elif_branches: vec![],
            else_body: Some(vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                span: Span::new(0, 1),
            })]),
            span: Span::new(0, 30),
        };
        emitter.emit_if_stmt(&if_stmt).unwrap();
        assert_eq!(
            emitter.output,
            "if true {\n    1;\n} else {\n    2;\n}\n"
        );
    }

    #[test]
    fn emit_if_stmt_with_elif() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let if_stmt = TypedIfStmt {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            elif_branches: vec![TypedElifBranch {
                condition: typed_expr(TypedExprKind::BoolLiteral(false), FluxType::Bool),
                body: vec![TypedStmt::Expr(TypedExprStmt {
                    expr: typed_expr(TypedExprKind::IntLiteral(2), FluxType::Int),
                    span: Span::new(0, 1),
                })],
                span: Span::new(0, 10),
            }],
            else_body: Some(vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(3), FluxType::Int),
                span: Span::new(0, 1),
            })]),
            span: Span::new(0, 40),
        };
        emitter.emit_if_stmt(&if_stmt).unwrap();
        assert_eq!(
            emitter.output,
            "if true {\n    1;\n} else if false {\n    2;\n} else {\n    3;\n}\n"
        );
    }

    #[test]
    fn emit_for_loop_basic() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("items".to_string());
        let for_loop = TypedForLoop {
            variable: "item".to_string(),
            variable_type: FluxType::Int,
            iterable: typed_expr(
                TypedExprKind::Ident("items".to_string()),
                FluxType::List(Box::new(FluxType::Int)),
            ),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::Ident("item".to_string()), FluxType::Int),
                span: Span::new(0, 1),
            })],
            span: Span::new(0, 30),
        };
        emitter.emit_for_loop(&for_loop).unwrap();
        assert_eq!(emitter.output, "for item in items {\n    item;\n}\n");
        assert!(emitter.local_vars.contains("item"));
    }

    #[test]
    fn emit_while_loop_basic() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let while_loop = TypedWhileLoop {
            condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
            body: vec![TypedStmt::Expr(TypedExprStmt {
                expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                span: Span::new(0, 1),
            })],
            span: Span::new(0, 20),
        };
        emitter.emit_while_loop(&while_loop).unwrap();
        assert_eq!(emitter.output, "while true {\n    1;\n}\n");
    }

    #[test]
    fn emit_return_with_value() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let ret = TypedReturnStmt {
            value: Some(typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int)),
            span: Span::new(0, 10),
        };
        emitter.emit_return(&ret).unwrap();
        assert_eq!(emitter.output, "return 42;\n");
    }

    #[test]
    fn emit_return_without_value() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let ret = TypedReturnStmt {
            value: None,
            span: Span::new(0, 7),
        };
        emitter.emit_return(&ret).unwrap();
        assert_eq!(emitter.output, "return;\n");
    }

    #[test]
    fn emit_expr_stmt_non_signal() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr_stmt = TypedExprStmt {
            expr: typed_expr(TypedExprKind::IntLiteral(42), FluxType::Int),
            span: Span::new(0, 2),
        };
        emitter.emit_expr_stmt(&expr_stmt).unwrap();
        assert_eq!(emitter.output, "42;\n");
    }

    #[test]
    fn emit_expr_stmt_signal_wrapped() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr_stmt = TypedExprStmt {
            expr: typed_expr(
                TypedExprKind::FunctionCall {
                    function: Box::new(typed_expr(
                        TypedExprKind::Ident("OPEN".to_string()),
                        FluxType::Fn {
                            params: crate::typeck::types::FnParams::Fixed(vec![
                                FluxType::String,
                                FluxType::Int,
                            ]),
                            ret: Box::new(FluxType::Signal),
                        },
                    )),
                    args: vec![
                        typed_expr(
                            TypedExprKind::Ident("symbol".to_string()),
                            FluxType::String,
                        ),
                        typed_expr(TypedExprKind::IntLiteral(100), FluxType::Int),
                    ],
                },
                FluxType::Signal,
            ),
            span: Span::new(0, 20),
        };
        emitter.emit_expr_stmt(&expr_stmt).unwrap();
        assert_eq!(emitter.output, "signals.push(Signal::open(ctx.symbol, 100));\n");
    }

    #[test]
    fn emit_assignment_with_indentation() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.indent_level = 2;
        let assign = TypedAssignment {
            target: typed_expr(TypedExprKind::Ident("x".to_string()), FluxType::Int),
            value: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
            span: Span::new(0, 5),
        };
        emitter.emit_assignment(&assign).unwrap();
        assert_eq!(emitter.output, "        let mut x = 1;\n");
    }

    #[test]
    fn emit_nested_if_in_for_indentation() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("items".to_string());
        let for_loop = TypedForLoop {
            variable: "item".to_string(),
            variable_type: FluxType::Int,
            iterable: typed_expr(
                TypedExprKind::Ident("items".to_string()),
                FluxType::List(Box::new(FluxType::Int)),
            ),
            body: vec![TypedStmt::If(TypedIfStmt {
                condition: typed_expr(TypedExprKind::BoolLiteral(true), FluxType::Bool),
                body: vec![TypedStmt::Expr(TypedExprStmt {
                    expr: typed_expr(TypedExprKind::IntLiteral(1), FluxType::Int),
                    span: Span::new(0, 1),
                })],
                elif_branches: vec![],
                else_body: None,
                span: Span::new(0, 20),
            })],
            span: Span::new(0, 40),
        };
        emitter.emit_for_loop(&for_loop).unwrap();
        let expected = "for item in items {\n    if true {\n        1;\n    }\n}\n";
        assert_eq!(emitter.output, expected);
    }

    // ===== Top-level emission tests (task 4.2) =====

    #[test]
    fn emit_preamble_content() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();
        assert!(
            output.starts_with("use flux_runtime::*;\n\n"),
            "Output must start with preamble, got: {:?}",
            &output[..output.len().min(40)]
        );
    }

    #[test]
    fn emit_full_with_params_and_state() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Struct fields
        assert!(
            output.contains("pub period: i64,"),
            "Struct should have pub param field"
        );
        assert!(
            output.contains("    count: i64,"),
            "Struct should have non-pub state field"
        );
        // state fields should NOT have pub
        assert!(
            !output.contains("pub count:"),
            "State field should not be pub"
        );

        // Default impl values
        assert!(
            output.contains("period: 20,"),
            "Default impl should have period: 20"
        );
        assert!(
            output.contains("count: 0,"),
            "Default impl should have count: 0"
        );

        // Strategy impl
        assert!(
            output.contains("impl Strategy for MyStrategy {"),
            "Should have Strategy impl"
        );
    }

    #[test]
    fn emit_empty_strategy_struct() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Empty struct
        assert!(
            output.contains("pub struct Test {\n}\n"),
            "Empty struct should have empty braces, got: {:?}",
            output
        );
    }

    #[test]
    fn emit_empty_strategy_default_impl() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("impl Default for Test {"),
            "Should have Default impl"
        );
        assert!(
            output.contains("Self {\n        }\n"),
            "Empty Default should have empty Self block"
        );
    }

    #[test]
    fn emit_empty_strategy_impl_block() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("impl Strategy for Test {\n}\n"),
            "Empty Strategy impl should have no methods"
        );
    }

    #[test]
    fn emit_event_handler_with_signal() {
        // Build a program with an on_bar handler containing OPEN(symbol, 100)
        let prog = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "Sig".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![TypedStmt::Expr(TypedExprStmt {
                        expr: TypedExpr {
                            kind: TypedExprKind::FunctionCall {
                                function: Box::new(typed_expr(
                                    TypedExprKind::Ident("OPEN".to_string()),
                                    FluxType::Fn {
                                        params: crate::typeck::types::FnParams::Fixed(vec![
                                            FluxType::String,
                                            FluxType::Int,
                                        ]),
                                        ret: Box::new(FluxType::Signal),
                                    },
                                )),
                                args: vec![
                                    typed_expr(
                                        TypedExprKind::Ident("symbol".to_string()),
                                        FluxType::String,
                                    ),
                                    typed_expr(TypedExprKind::IntLiteral(100), FluxType::Int),
                                ],
                            },
                            resolved_type: FluxType::Signal,
                            span: Span::new(0, 20),
                        },
                        span: Span::new(0, 20),
                    })],
                    span: Span::new(0, 50),
                })],
                span: Span::new(0, 60),
            },
            span: Span::new(0, 60),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Signal declaration
        assert!(
            output.contains("let mut signals: Vec<Signal> = Vec::new();"),
            "Handler should declare signals vector"
        );
        // Signal push
        assert!(
            output.contains("signals.push(Signal::open(ctx.symbol, 100));"),
            "Handler should push signal"
        );
        // Return signals
        assert!(
            output.contains("        signals\n"),
            "Handler should return signals"
        );
    }

    #[test]
    fn emit_event_handler_empty_body() {
        // Handler with no statements should only have signal decl and return
        let prog = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "Empty".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![],
                    span: Span::new(0, 30),
                })],
                span: Span::new(0, 40),
            },
            span: Span::new(0, 40),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Should contain the method with just signal decl and return
        let expected_handler = concat!(
            "    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {\n",
            "        let mut signals: Vec<Signal> = Vec::new();\n",
            "        signals\n",
            "    }\n",
        );
        assert!(
            output.contains(expected_handler),
            "Empty handler should only have signal decl and return, got:\n{}",
            output
        );
    }

    #[test]
    fn emit_properties_before_params_in_struct() {
        // Build program with both properties and params, verify ordering
        let prog = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "Order".to_string(),
                body: vec![
                    TypedStrategyItem::Property(TypedProperty {
                        name: "version".to_string(),
                        value: TypedExpr {
                            kind: TypedExprKind::IntLiteral(1),
                            resolved_type: FluxType::Int,
                            span: Span::new(10, 11),
                        },
                        span: Span::new(5, 15),
                    }),
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "period".to_string(),
                            default_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(20),
                                resolved_type: FluxType::Int,
                                span: Span::new(30, 32),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(25, 35),
                        }],
                        span: Span::new(20, 40),
                    }),
                ],
                span: Span::new(0, 50),
            },
            span: Span::new(0, 50),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Properties appear before params in struct
        let version_pos = output.find("pub version: i64,").unwrap();
        let period_pos = output.find("pub period: i64,").unwrap();
        assert!(
            version_pos < period_pos,
            "Properties should appear before params in struct"
        );
    }

    #[test]
    fn emit_blank_lines_between_sections() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // There should be a blank line between struct and Default impl
        assert!(
            output.contains("}\n\nimpl Default for"),
            "Should have blank line between struct and Default impl"
        );
        // There should be a blank line between Default impl and Strategy impl
        assert!(
            output.contains("}\n\nimpl Strategy for"),
            "Should have blank line between Default impl and Strategy impl"
        );
    }

    #[test]
    fn emit_four_space_indentation() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Struct fields use 4-space indent
        assert!(
            output.contains("    pub period: i64,"),
            "Struct fields should use 4-space indentation"
        );
        // Default impl fn uses 4-space indent
        assert!(
            output.contains("    fn default() -> Self {"),
            "Default fn should use 4-space indentation"
        );
        // Inner Self block uses 8-space indent
        assert!(
            output.contains("        Self {"),
            "Self block should use 8-space (2-level) indentation"
        );
        // Field defaults use 12-space indent
        assert!(
            output.contains("            period: 20,"),
            "Default field values should use 12-space (3-level) indentation"
        );
    }

    #[test]
    fn emit_knr_braces() {
        let prog = full_context_program();
        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // K&R style: opening brace on same line
        assert!(
            output.contains("pub struct MyStrategy {\n"),
            "Struct opening brace should be K&R style"
        );
        assert!(
            output.contains("impl Default for MyStrategy {\n"),
            "Default impl opening brace should be K&R style"
        );
        assert!(
            output.contains("impl Strategy for MyStrategy {\n"),
            "Strategy impl opening brace should be K&R style"
        );
    }

    #[test]
    fn emit_full_end_to_end() {
        // Build a realistic TypedProgram similar to the design doc example
        let prog = TypedProgram {
            imports: vec![Import {
                module_path: "indicators".to_string(),
                names: vec!["sma".to_string()],
                span: Span::new(0, 30),
            }],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "MomentumStrategy".to_string(),
                body: vec![
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![
                            TypedParam {
                                name: "period".to_string(),
                                default_value: TypedExpr {
                                    kind: TypedExprKind::IntLiteral(20),
                                    resolved_type: FluxType::Int,
                                    span: Span::new(50, 52),
                                },
                                resolved_type: FluxType::Int,
                                span: Span::new(40, 55),
                            },
                            TypedParam {
                                name: "threshold".to_string(),
                                default_value: TypedExpr {
                                    kind: TypedExprKind::FloatLiteral(2.0),
                                    resolved_type: FluxType::Float,
                                    span: Span::new(65, 68),
                                },
                                resolved_type: FluxType::Float,
                                span: Span::new(58, 70),
                            },
                        ],
                        span: Span::new(35, 75),
                    }),
                    TypedStrategyItem::StateBlock(TypedStateBlock {
                        variables: vec![TypedStateVar {
                            name: "count".to_string(),
                            initial_value: TypedExpr {
                                kind: TypedExprKind::IntLiteral(0),
                                resolved_type: FluxType::Int,
                                span: Span::new(90, 91),
                            },
                            resolved_type: FluxType::Int,
                            span: Span::new(80, 95),
                        }],
                        span: Span::new(78, 100),
                    }),
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![
                            // count = count + 1
                            TypedStmt::Assignment(TypedAssignment {
                                target: typed_expr(
                                    TypedExprKind::Ident("count".to_string()),
                                    FluxType::Int,
                                ),
                                value: typed_expr(
                                    TypedExprKind::BinaryOp {
                                        left: Box::new(typed_expr(
                                            TypedExprKind::Ident("count".to_string()),
                                            FluxType::Int,
                                        )),
                                        op: BinOp::Add,
                                        right: Box::new(typed_expr(
                                            TypedExprKind::IntLiteral(1),
                                            FluxType::Int,
                                        )),
                                    },
                                    FluxType::Int,
                                ),
                                span: Span::new(110, 125),
                            }),
                            // if close > sma(close, period) { OPEN(symbol, 100) }
                            TypedStmt::If(TypedIfStmt {
                                condition: typed_expr(
                                    TypedExprKind::BinaryOp {
                                        left: Box::new(typed_expr(
                                            TypedExprKind::Ident("close".to_string()),
                                            FluxType::Float,
                                        )),
                                        op: BinOp::Gt,
                                        right: Box::new(typed_expr(
                                            TypedExprKind::FunctionCall {
                                                function: Box::new(typed_expr(
                                                    TypedExprKind::Ident("sma".to_string()),
                                                    FluxType::Fn {
                                                        params:
                                                            crate::typeck::types::FnParams::VariadicNumeric,
                                                        ret: Box::new(FluxType::Float),
                                                    },
                                                )),
                                                args: vec![
                                                    typed_expr(
                                                        TypedExprKind::Ident("close".to_string()),
                                                        FluxType::Float,
                                                    ),
                                                    typed_expr(
                                                        TypedExprKind::Ident("period".to_string()),
                                                        FluxType::Int,
                                                    ),
                                                ],
                                            },
                                            FluxType::Float,
                                        )),
                                    },
                                    FluxType::Bool,
                                ),
                                body: vec![TypedStmt::Expr(TypedExprStmt {
                                    expr: TypedExpr {
                                        kind: TypedExprKind::FunctionCall {
                                            function: Box::new(typed_expr(
                                                TypedExprKind::Ident("OPEN".to_string()),
                                                FluxType::Fn {
                                                    params:
                                                        crate::typeck::types::FnParams::Fixed(vec![
                                                            FluxType::String,
                                                            FluxType::Int,
                                                        ]),
                                                    ret: Box::new(FluxType::Signal),
                                                },
                                            )),
                                            args: vec![
                                                typed_expr(
                                                    TypedExprKind::Ident("symbol".to_string()),
                                                    FluxType::String,
                                                ),
                                                typed_expr(
                                                    TypedExprKind::IntLiteral(100),
                                                    FluxType::Int,
                                                ),
                                            ],
                                        },
                                        resolved_type: FluxType::Signal,
                                        span: Span::new(140, 160),
                                    },
                                    span: Span::new(140, 160),
                                })],
                                elif_branches: vec![],
                                else_body: None,
                                span: Span::new(130, 170),
                            }),
                        ],
                        span: Span::new(105, 175),
                    }),
                ],
                span: Span::new(32, 180),
            },
            span: Span::new(0, 180),
        };

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // Verify complete output structure
        let expected = concat!(
            "use flux_runtime::*;\n",
            "\n",
            "pub struct MomentumStrategy {\n",
            "    pub period: i64,\n",
            "    pub threshold: f64,\n",
            "    count: i64,\n",
            "}\n",
            "\n",
            "impl Default for MomentumStrategy {\n",
            "    fn default() -> Self {\n",
            "        Self {\n",
            "            period: 20,\n",
            "            threshold: 2.0,\n",
            "            count: 0,\n",
            "        }\n",
            "    }\n",
            "}\n",
            "\n",
            "impl Strategy for MomentumStrategy {\n",
            "    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {\n",
            "        let mut signals: Vec<Signal> = Vec::new();\n",
            "        self.count = (self.count + 1);\n",
            "        if (ctx.close > sma(ctx.close, self.period)) {\n",
            "            signals.push(Signal::open(ctx.symbol, 100));\n",
            "        }\n",
            "        signals\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(output, expected);
    }

    // ===== @hot/@cold field-level decorator codegen test =====

    #[test]
    fn hot_cold_field_decorators_emit_split_structs() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "MarketTick".to_string(),
            type_params: vec![],
            fields: vec![
                TypedStructField {
                    name: "price".to_string(),
                    resolved_type: FluxType::Float,
                    bit_width: None,
                    field_decorator_names: vec!["hot".to_string()],
                    span: Span::new(10, 20),
                },
                TypedStructField {
                    name: "volume".to_string(),
                    resolved_type: FluxType::Float,
                    bit_width: None,
                    field_decorator_names: vec!["hot".to_string()],
                    span: Span::new(20, 30),
                },
                TypedStructField {
                    name: "exchange_id".to_string(),
                    resolved_type: FluxType::Int,
                    bit_width: None,
                    field_decorator_names: vec!["cold".to_string()],
                    span: Span::new(30, 40),
                },
                TypedStructField {
                    name: "debug_tag".to_string(),
                    resolved_type: FluxType::Int,
                    bit_width: None,
                    field_decorator_names: vec!["cold".to_string()],
                    span: Span::new(40, 50),
                },
            ],
            decorators: vec![],
            span: Span::new(0, 60),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        // The original struct is still emitted
        assert!(output.contains("pub struct MarketTick {"), "original struct missing");
        assert!(output.contains("pub price: f64,"), "price field missing");

        // Hot sub-struct with cache-line alignment
        assert!(output.contains("#[repr(align(64))]"), "cache-line alignment missing");
        assert!(output.contains("pub struct MarketTick_Hot {"), "hot sub-struct missing");
        assert!(output.contains("pub struct MarketTick_Cold {"), "cold sub-struct missing");

        // Split method emitted
        assert!(output.contains("pub fn split(&self) -> (MarketTick_Hot, MarketTick_Cold)"), "split method missing");

        // Verify hot struct contains only hot fields
        let hot_start = output.find("pub struct MarketTick_Hot {").unwrap();
        let hot_end = output[hot_start..].find('}').unwrap() + hot_start;
        let hot_body = &output[hot_start..hot_end];
        assert!(hot_body.contains("pub price: f64,"), "hot struct missing price");
        assert!(hot_body.contains("pub volume: f64,"), "hot struct missing volume");
        assert!(!hot_body.contains("exchange_id"), "hot struct should not have cold field");

        // Verify cold struct contains only cold fields
        let cold_start = output.find("pub struct MarketTick_Cold {").unwrap();
        let cold_end = output[cold_start..].find('}').unwrap() + cold_start;
        let cold_body = &output[cold_start..cold_end];
        assert!(cold_body.contains("pub exchange_id: i64,"), "cold struct missing exchange_id");
        assert!(cold_body.contains("pub debug_tag: i64,"), "cold struct missing debug_tag");
        assert!(!cold_body.contains("price"), "cold struct should not have hot field");
    }

    #[test]
    fn no_hot_cold_decorators_emits_no_split() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "Plain".to_string(),
            type_params: vec![],
            fields: vec![
                TypedStructField {
                    name: "x".to_string(),
                    resolved_type: FluxType::Float,
                    bit_width: None,
                    field_decorator_names: vec![],
                    span: Span::new(10, 20),
                },
                TypedStructField {
                    name: "y".to_string(),
                    resolved_type: FluxType::Float,
                    bit_width: None,
                    field_decorator_names: vec![],
                    span: Span::new(20, 30),
                },
            ],
            decorators: vec![],
            span: Span::new(0, 40),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(output.contains("pub struct Plain {"), "struct missing");
        assert!(!output.contains("Plain_Hot"), "should not emit hot split");
        assert!(!output.contains("Plain_Cold"), "should not emit cold split");
    }

    #[test]
    fn hot_only_decorators_emit_split_hot() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "Tick".to_string(),
            type_params: vec![],
            fields: vec![
                TypedStructField {
                    name: "price".to_string(),
                    resolved_type: FluxType::Float,
                    bit_width: None,
                    field_decorator_names: vec!["hot".to_string()],
                    span: Span::new(10, 20),
                },
                TypedStructField {
                    name: "extra".to_string(),
                    resolved_type: FluxType::Float,
                    bit_width: None,
                    field_decorator_names: vec![],
                    span: Span::new(20, 30),
                },
            ],
            decorators: vec![],
            span: Span::new(0, 40),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(output.contains("pub struct Tick_Hot {"), "hot sub-struct missing");
        assert!(!output.contains("Tick_Cold"), "should not emit cold split when no cold fields");
        assert!(output.contains("pub fn split_hot(&self) -> Tick_Hot"), "split_hot method missing");
    }

    // ===== Generics codegen tests =====

    /// Verifies: struct Container[T] { ... } → struct Container<T> { ... }
    #[test]
    fn emit_generic_struct_single_param() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "Container".to_string(),
            type_params: vec!["T".to_string()],
            fields: vec![TypedStructField {
                name: "value".to_string(),
                resolved_type: FluxType::TypeParam("T".to_string()),
                bit_width: None,
                field_decorator_names: vec![],
                span: Span::new(5, 10),
            }],
            decorators: vec![],
            span: Span::new(0, 20),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("pub struct Container<T>"),
            "Generic struct should use angle brackets. Got: {}",
            output
        );
        assert!(
            output.contains("value: T"),
            "Field type should use type param T. Got: {}",
            output
        );
    }

    /// Verifies: struct Pair[K, V] { ... } → struct Pair<K, V> { ... }
    #[test]
    fn emit_generic_struct_multiple_params() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "Pair".to_string(),
            type_params: vec!["K".to_string(), "V".to_string()],
            fields: vec![
                TypedStructField {
                    name: "key".to_string(),
                    resolved_type: FluxType::TypeParam("K".to_string()),
                    bit_width: None,
                    field_decorator_names: vec![],
                    span: Span::new(5, 10),
                },
                TypedStructField {
                    name: "val".to_string(),
                    resolved_type: FluxType::TypeParam("V".to_string()),
                    bit_width: None,
                    field_decorator_names: vec![],
                    span: Span::new(10, 15),
                },
            ],
            decorators: vec![],
            span: Span::new(0, 20),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("pub struct Pair<K, V>"),
            "Multi-param generic struct should emit <K, V>. Got: {}",
            output
        );
        assert!(
            output.contains("key: K"),
            "Field should use type param K. Got: {}",
            output
        );
        assert!(
            output.contains("val: V"),
            "Field should use type param V. Got: {}",
            output
        );
    }

    /// Verifies: fn push[T](v: Vec[T], item: T) → fn push<T>(v: Vec<T>, item: T)
    #[test]
    fn emit_generic_function_no_bounds() {
        let mut prog = minimal_program();
        prog.functions = vec![TypedFnDef {
            name: "push".to_string(),
            type_params: vec!["T".to_string()],
            type_param_bounds: vec![None],
            params: vec!["v".to_string(), "item".to_string()],
            param_types: vec![
                FluxType::Generic("Vec".to_string(), vec![FluxType::TypeParam("T".to_string())]),
                FluxType::TypeParam("T".to_string()),
            ],
            body: vec![],
            return_type: FluxType::Generic("Vec".to_string(), vec![FluxType::TypeParam("T".to_string())]),
            span: Span::new(0, 30),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("fn push<T>("),
            "Generic function should emit <T>. Got: {}",
            output
        );
        assert!(
            output.contains("v: Vec<T>"),
            "Param type should be Vec<T>. Got: {}",
            output
        );
        assert!(
            output.contains("item: T"),
            "Param type should be T. Got: {}",
            output
        );
        assert!(
            output.contains("-> Vec<T>"),
            "Return type should be Vec<T>. Got: {}",
            output
        );
    }

    /// Verifies: fn process[T: DataFeed](feed: T) → fn process<T: DataFeed>(feed: T)
    #[test]
    fn emit_generic_function_with_trait_bound() {
        let mut prog = minimal_program();
        prog.functions = vec![TypedFnDef {
            name: "process".to_string(),
            type_params: vec!["T".to_string()],
            type_param_bounds: vec![Some("DataFeed".to_string())],
            params: vec!["feed".to_string()],
            param_types: vec![FluxType::TypeParam("T".to_string())],
            body: vec![],
            return_type: FluxType::Float,
            span: Span::new(0, 30),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("fn process<T: DataFeed>("),
            "Trait-bounded generic should emit <T: DataFeed>. Got: {}",
            output
        );
        assert!(
            output.contains("feed: T"),
            "Param should be typed as T. Got: {}",
            output
        );
    }

    /// Verifies: fn transform[A, B: Clone](a: A, b: B) → fn transform<A, B: Clone>(a: A, b: B)
    #[test]
    fn emit_generic_function_mixed_bounds() {
        let mut prog = minimal_program();
        prog.functions = vec![TypedFnDef {
            name: "transform".to_string(),
            type_params: vec!["A".to_string(), "B".to_string()],
            type_param_bounds: vec![None, Some("Clone".to_string())],
            params: vec!["a".to_string(), "b".to_string()],
            param_types: vec![
                FluxType::TypeParam("A".to_string()),
                FluxType::TypeParam("B".to_string()),
            ],
            body: vec![],
            return_type: FluxType::Void,
            span: Span::new(0, 30),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("fn transform<A, B: Clone>("),
            "Mixed bounds should emit <A, B: Clone>. Got: {}",
            output
        );
    }

    /// Verifies: impl block on a generic struct emits impl<T> Name<T> { ... }
    #[test]
    fn emit_impl_block_on_generic_struct() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "Stack".to_string(),
            type_params: vec!["T".to_string()],
            fields: vec![TypedStructField {
                name: "items".to_string(),
                resolved_type: FluxType::Generic("Vec".to_string(), vec![FluxType::TypeParam("T".to_string())]),
                bit_width: None,
                field_decorator_names: vec![],
                span: Span::new(5, 10),
            }],
            decorators: vec![],
            span: Span::new(0, 20),
        }];
        prog.impl_blocks = vec![TypedImplBlock {
            trait_name: None,
            target_type: "Stack".to_string(),
            methods: vec![TypedFnDef {
                name: "peek".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec!["self".to_string()],
                param_types: vec![FluxType::Struct("Stack".to_string())],
                body: vec![],
                return_type: FluxType::TypeParam("T".to_string()),
                span: Span::new(0, 20),
            }],
            span: Span::new(0, 50),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("impl<T> Stack<T>"),
            "Impl block should emit generic params. Got: {}",
            output
        );
        assert!(
            output.contains("-> T"),
            "Return type should be T. Got: {}",
            output
        );
    }

    /// Verifies: impl Trait for generic struct emits impl<T> Trait for Name<T>
    #[test]
    fn emit_trait_impl_on_generic_struct() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "MyVec".to_string(),
            type_params: vec!["T".to_string()],
            fields: vec![],
            decorators: vec![],
            span: Span::new(0, 20),
        }];
        prog.impl_blocks = vec![TypedImplBlock {
            trait_name: Some("Iterable".to_string()),
            target_type: "MyVec".to_string(),
            methods: vec![TypedFnDef {
                name: "next".to_string(),
                type_params: vec![],
                type_param_bounds: vec![],
                params: vec!["self".to_string()],
                param_types: vec![FluxType::Struct("MyVec".to_string())],
                body: vec![],
                return_type: FluxType::TypeParam("T".to_string()),
                span: Span::new(0, 20),
            }],
            span: Span::new(0, 50),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("impl<T> Iterable for MyVec<T>"),
            "Trait impl on generic struct should include type params. Got: {}",
            output
        );
    }

    /// Verifies: non-generic function has no angle brackets
    #[test]
    fn emit_non_generic_function_no_angle_brackets() {
        let mut prog = minimal_program();
        prog.functions = vec![TypedFnDef {
            name: "add".to_string(),
            type_params: vec![],
            type_param_bounds: vec![],
            params: vec!["x".to_string(), "y".to_string()],
            param_types: vec![FluxType::Float, FluxType::Float],
            body: vec![],
            return_type: FluxType::Float,
            span: Span::new(0, 20),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("fn add("),
            "Non-generic function should not have angle brackets. Got: {}",
            output
        );
        assert!(
            !output.contains("fn add<"),
            "Non-generic function should NOT have <. Got: {}",
            output
        );
    }

    /// Verifies: field typed as Generic("Vec", [Float]) emits Vec<f64>
    #[test]
    fn emit_struct_field_with_concrete_generic_type() {
        let mut prog = minimal_program();
        prog.structs = vec![TypedStructDef {
            name: "Portfolio".to_string(),
            type_params: vec![],
            fields: vec![TypedStructField {
                name: "positions".to_string(),
                resolved_type: FluxType::Generic("Vec".to_string(), vec![FluxType::Float]),
                bit_width: None,
                field_decorator_names: vec![],
                span: Span::new(5, 15),
            }],
            decorators: vec![],
            span: Span::new(0, 30),
        }];

        let mut emitter = CodeEmitter::new(&prog);
        let output = emitter.emit().unwrap();

        assert!(
            output.contains("positions: Vec<f64>"),
            "Concrete generic field should emit Vec<f64>. Got: {}",
            output
        );
    }

    // ===== HashMap codegen tests =====

    /// Verifies: HashMap.new() emits std::collections::HashMap::new()
    #[test]
    fn emit_hashmap_new() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("HashMap".to_string()),
                    FluxType::Void,
                )),
                method: "new".to_string(),
                args: vec![],
            },
            FluxType::Generic(
                "HashMap".to_string(),
                vec![FluxType::String, FluxType::Float],
            ),
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(emitter.output, "std::collections::HashMap::new()");
    }

    /// Verifies: map.insert(k, v) emits map.insert(k, v)
    #[test]
    fn emit_hashmap_insert() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("registry".to_string());
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("registry".to_string()),
                    FluxType::Generic(
                        "HashMap".to_string(),
                        vec![FluxType::String, FluxType::Float],
                    ),
                )),
                method: "insert".to_string(),
                args: vec![
                    typed_expr(
                        TypedExprKind::StringLiteral("AAPL".to_string()),
                        FluxType::String,
                    ),
                    typed_expr(TypedExprKind::FloatLiteral(150.0), FluxType::Float),
                ],
            },
            FluxType::Generic(
                "HashMap".to_string(),
                vec![FluxType::String, FluxType::Float],
            ),
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "registry.insert(String::from(\"AAPL\"), 150.0)"
        );
    }

    /// Verifies: map.get(k) emits map.get(&k).cloned().unwrap()
    #[test]
    fn emit_hashmap_get() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("registry".to_string());
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("registry".to_string()),
                    FluxType::Generic(
                        "HashMap".to_string(),
                        vec![FluxType::String, FluxType::Float],
                    ),
                )),
                method: "get".to_string(),
                args: vec![typed_expr(
                    TypedExprKind::StringLiteral("AAPL".to_string()),
                    FluxType::String,
                )],
            },
            FluxType::Float,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "registry.get(&String::from(\"AAPL\")).cloned().unwrap()"
        );
    }

    /// Verifies: map.contains_key(k) emits map.contains_key(&k)
    #[test]
    fn emit_hashmap_contains_key() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("registry".to_string());
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("registry".to_string()),
                    FluxType::Generic(
                        "HashMap".to_string(),
                        vec![FluxType::String, FluxType::Float],
                    ),
                )),
                method: "contains_key".to_string(),
                args: vec![typed_expr(
                    TypedExprKind::StringLiteral("AAPL".to_string()),
                    FluxType::String,
                )],
            },
            FluxType::Bool,
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "registry.contains_key(&String::from(\"AAPL\"))"
        );
    }

    /// Verifies: map.remove(k) emits map.remove(&k)
    #[test]
    fn emit_hashmap_remove() {
        let prog = minimal_program();
        let mut emitter = CodeEmitter::new(&prog);
        emitter.local_vars.insert("registry".to_string());
        let expr = typed_expr(
            TypedExprKind::MethodCall {
                receiver: Box::new(typed_expr(
                    TypedExprKind::Ident("registry".to_string()),
                    FluxType::Generic(
                        "HashMap".to_string(),
                        vec![FluxType::String, FluxType::Float],
                    ),
                )),
                method: "remove".to_string(),
                args: vec![typed_expr(
                    TypedExprKind::StringLiteral("AAPL".to_string()),
                    FluxType::String,
                )],
            },
            FluxType::Generic(
                "HashMap".to_string(),
                vec![FluxType::String, FluxType::Float],
            ),
        );
        emitter.emit_expr(&expr).unwrap();
        assert_eq!(
            emitter.output,
            "registry.remove(&String::from(\"AAPL\"))"
        );
    }
}
