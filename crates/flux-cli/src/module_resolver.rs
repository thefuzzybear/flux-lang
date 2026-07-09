//! Module resolver for cross-file function imports.
//!
//! Resolves `::` path-separated imports to filesystem paths, parses library files,
//! walks the call graph for selective function inclusion, and merges `FnDef` nodes
//! into the main `Program` AST before typechecking.

use flux_compiler::parser::ast::{Expr, ExprKind, FnDef, Program, Stmt};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

/// Errors produced by module resolution.
#[derive(Debug, Clone)]
pub enum ModuleError {
    /// The resolved file path does not exist.
    FileNotFound {
        import_path: String,
        resolved_path: PathBuf,
    },
    /// Parse error in a library file.
    ParseError {
        file_path: PathBuf,
        message: String,
    },
    /// A requested function was not found in the library.
    FunctionNotFound {
        function_name: String,
        file_path: PathBuf,
    },
    /// Circular import detected.
    CircularImport { chain: Vec<PathBuf> },
    /// Duplicate function definition.
    DuplicateFunction {
        name: String,
        first_file: PathBuf,
        second_file: PathBuf,
    },
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleError::FileNotFound {
                import_path,
                resolved_path,
            } => {
                write!(
                    f,
                    "module '{}' not found: expected file at '{}'",
                    import_path,
                    resolved_path.display()
                )
            }
            ModuleError::ParseError { file_path, message } => {
                write!(f, "error parsing '{}': {}", file_path.display(), message)
            }
            ModuleError::FunctionNotFound {
                function_name,
                file_path,
            } => {
                write!(
                    f,
                    "function '{}' not found in '{}'",
                    function_name,
                    file_path.display()
                )
            }
            ModuleError::CircularImport { chain } => {
                let chain_str: Vec<String> =
                    chain.iter().map(|p| p.display().to_string()).collect();
                write!(
                    f,
                    "circular import detected: {}",
                    chain_str.join(" → ")
                )
            }
            ModuleError::DuplicateFunction {
                name,
                first_file,
                second_file,
            } => {
                write!(
                    f,
                    "duplicate function '{}': defined in '{}' and '{}'",
                    name,
                    first_file.display(),
                    second_file.display()
                )
            }
        }
    }
}

impl std::error::Error for ModuleError {}

/// Resolve all file-module imports in the given program.
///
/// - `program`: The parsed main-file Program AST
/// - `main_file_dir`: Directory containing the main .flux file
///
/// Returns a new Program with file-module imports removed from `imports`
/// and their resolved FnDefs appended to `functions`.
pub fn resolve_modules(program: Program, main_file_dir: &Path) -> Result<Program, ModuleError> {
    let mut resolver = ModuleResolver::new(main_file_dir);
    resolver.resolve(program)
}

/// Internal resolver state.
struct ModuleResolver {
    /// Base directory for resolving the initial imports (main file's dir).
    base_dir: PathBuf,
    /// Cache: canonical path → parsed function definitions.
    cache: HashMap<PathBuf, Vec<FnDef>>,
    /// Import stack for circular dependency detection.
    import_stack: Vec<PathBuf>,
}

impl ModuleResolver {
    fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
            cache: HashMap::new(),
            import_stack: Vec::new(),
        }
    }

    fn resolve(&mut self, mut program: Program) -> Result<Program, ModuleError> {
        // Partition imports into file-module (contains `::`) and built-in (no `::`)
        let (file_imports, builtin_imports): (Vec<_>, Vec<_>) = program
            .imports
            .into_iter()
            .partition(|imp| imp.module_path.contains("::"));

        // Resolve each file-module import
        let mut merged_functions: Vec<FnDef> = Vec::new();
        let mut known_names: HashMap<String, PathBuf> = HashMap::new();

        // Register main file's own functions as known
        for fn_def in &program.functions {
            known_names.insert(fn_def.name.clone(), self.base_dir.join("main"));
        }

        for import in &file_imports {
            let base_dir = self.base_dir.clone();
            let resolved_path = self.resolve_path(&import.module_path, &base_dir)?;
            let all_fns = self.load_file(&resolved_path)?;

            // Selective inclusion: walk call graph from requested names
            let selected = self.select_functions(&import.names, &all_fns, &resolved_path)?;

            // Check for duplicates
            for fn_def in &selected {
                if let Some(existing_file) = known_names.get(&fn_def.name) {
                    return Err(ModuleError::DuplicateFunction {
                        name: fn_def.name.clone(),
                        first_file: existing_file.clone(),
                        second_file: resolved_path.clone(),
                    });
                }
                known_names.insert(fn_def.name.clone(), resolved_path.clone());
            }

            merged_functions.extend(selected);
        }

        // Assemble final program: retain only built-in imports, merge functions
        program.imports = builtin_imports;
        program.functions.extend(merged_functions);
        Ok(program)
    }

    /// Load and parse a library file, using cache and detecting cycles.
    fn load_file(&mut self, path: &Path) -> Result<Vec<FnDef>, ModuleError> {
        // Check cache first
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }

        // Circular import check
        if self.import_stack.iter().any(|p| p.as_path() == path) {
            let mut chain = self.import_stack.clone();
            chain.push(path.to_path_buf());
            return Err(ModuleError::CircularImport { chain });
        }

        self.import_stack.push(path.to_path_buf());

        // Read and parse the file
        let source = std::fs::read_to_string(path).map_err(|_| ModuleError::FileNotFound {
            import_path: path.display().to_string(),
            resolved_path: path.to_path_buf(),
        })?;

        let tokens =
            flux_compiler::lexer::lex_with_spans(&source).map_err(|e| ModuleError::ParseError {
                file_path: path.to_path_buf(),
                message: e.to_string(),
            })?;

        let program =
            flux_compiler::parser::parse(tokens).map_err(|e| ModuleError::ParseError {
                file_path: path.to_path_buf(),
                message: e.to_string(),
            })?;

        // Recursively resolve file-module imports in this library file
        let file_dir = path.parent().unwrap_or(Path::new("."));
        let file_imports: Vec<_> = program
            .imports
            .iter()
            .filter(|imp| imp.module_path.contains("::"))
            .cloned()
            .collect();

        let mut all_fns = program.functions;

        for import in &file_imports {
            let dep_path = self.resolve_path(&import.module_path, file_dir)?;
            let dep_fns = self.load_file(&dep_path)?;
            let selected = self.select_functions(&import.names, &dep_fns, &dep_path)?;
            all_fns.extend(selected);
        }

        // Pop import stack and cache result
        self.import_stack.pop();
        self.cache.insert(path.to_path_buf(), all_fns.clone());

        Ok(all_fns)
    }

    /// Select only the explicitly requested functions plus their transitive call dependencies.
    fn select_functions(
        &self,
        requested_names: &[String],
        available_fns: &[FnDef],
        file_path: &Path,
    ) -> Result<Vec<FnDef>, ModuleError> {
        let fn_map: HashMap<&str, &FnDef> = available_fns
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();

        // Verify all requested names exist
        for name in requested_names {
            if !fn_map.contains_key(name.as_str()) {
                return Err(ModuleError::FunctionNotFound {
                    function_name: name.clone(),
                    file_path: file_path.to_path_buf(),
                });
            }
        }

        // BFS from requested names to find transitive dependencies
        let mut included: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = requested_names.iter().map(|s| s.as_str()).collect();

        while let Some(name) = queue.pop_front() {
            if included.contains(name) {
                continue;
            }
            included.insert(name);

            if let Some(fn_def) = fn_map.get(name) {
                // Extract function calls from the body
                let callees = extract_call_names(&fn_def.body);
                for callee in &callees {
                    if fn_map.contains_key(callee.as_str())
                        && !included.contains(callee.as_str())
                    {
                        queue.push_back(fn_map.get(callee.as_str()).unwrap().name.as_str());
                    }
                }
            }
        }

        Ok(available_fns
            .iter()
            .filter(|f| included.contains(f.name.as_str()))
            .cloned()
            .collect())
    }

    /// Convert a `::` module path to a filesystem path.
    /// `a::b::c` → `{base_dir}/a/b/c.flux`
    fn resolve_path(&self, module_path: &str, base_dir: &Path) -> Result<PathBuf, ModuleError> {
        let segments: Vec<&str> = module_path.split("::").collect();
        let mut path = base_dir.to_path_buf();
        for segment in &segments {
            path.push(segment);
        }
        path.set_extension("flux");

        // Canonicalize for cache key consistency
        let canonical = path.canonicalize().map_err(|_| ModuleError::FileNotFound {
            import_path: module_path.to_string(),
            resolved_path: path.clone(),
        })?;

        Ok(canonical)
    }
}

/// Extract all function call names from a statement list (walks AST).
fn extract_call_names(stmts: &[Stmt]) -> Vec<String> {
    let mut names = Vec::new();
    for stmt in stmts {
        collect_calls_from_stmt(stmt, &mut names);
    }
    names
}

/// Recursively collect function call names from a single statement.
fn collect_calls_from_stmt(stmt: &Stmt, names: &mut Vec<String>) {
    match stmt {
        Stmt::Assignment(assign) => {
            collect_calls_from_expr(&assign.target, names);
            collect_calls_from_expr(&assign.value, names);
        }
        Stmt::If(if_stmt) => {
            collect_calls_from_expr(&if_stmt.condition, names);
            for s in &if_stmt.body {
                collect_calls_from_stmt(s, names);
            }
            for elif in &if_stmt.elif_branches {
                collect_calls_from_expr(&elif.condition, names);
                for s in &elif.body {
                    collect_calls_from_stmt(s, names);
                }
            }
            if let Some(else_body) = &if_stmt.else_body {
                for s in else_body {
                    collect_calls_from_stmt(s, names);
                }
            }
        }
        Stmt::For(for_loop) => {
            collect_calls_from_expr(&for_loop.iterable, names);
            for s in &for_loop.body {
                collect_calls_from_stmt(s, names);
            }
        }
        Stmt::While(while_loop) => {
            collect_calls_from_expr(&while_loop.condition, names);
            for s in &while_loop.body {
                collect_calls_from_stmt(s, names);
            }
        }
        Stmt::Return(ret) => {
            if let Some(value) = &ret.value {
                collect_calls_from_expr(value, names);
            }
        }
        Stmt::Expr(expr_stmt) => {
            collect_calls_from_expr(&expr_stmt.expr, names);
        }
    }
}

/// Recursively collect function call names from an expression.
fn collect_calls_from_expr(expr: &Expr, names: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::FunctionCall { function, args } => {
            // If the function is a simple identifier, record it as a call
            if let ExprKind::Ident(name) = &function.kind {
                names.push(name.clone());
            } else {
                // Walk the function expression for nested calls
                collect_calls_from_expr(function, names);
            }
            // Walk arguments for nested calls
            for arg in args {
                collect_calls_from_expr(arg, names);
            }
        }
        ExprKind::BinaryOp { left, right, .. } => {
            collect_calls_from_expr(left, names);
            collect_calls_from_expr(right, names);
        }
        ExprKind::UnaryOp { operand, .. } => {
            collect_calls_from_expr(operand, names);
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_calls_from_expr(receiver, names);
            for arg in args {
                collect_calls_from_expr(arg, names);
            }
        }
        ExprKind::IndexAccess { object, index } => {
            collect_calls_from_expr(object, names);
            collect_calls_from_expr(index, names);
        }
        ExprKind::MemberAccess { object, .. } => {
            collect_calls_from_expr(object, names);
        }
        ExprKind::ListLiteral(elements) => {
            for elem in elements {
                collect_calls_from_expr(elem, names);
            }
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_calls_from_expr(value, names);
            }
        }
        // Leaf nodes: no calls to extract
        ExprKind::Ident(_)
        | ExprKind::IntLiteral(_)
        | ExprKind::FloatLiteral(_)
        | ExprKind::StringLiteral(_)
        | ExprKind::BoolLiteral(_)
        | ExprKind::NullLiteral => {}
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::collection::hash_set;
    use std::collections::HashSet;
    use std::fs;
    use tempfile::TempDir;

    /// Strategy to generate a valid module path segment: lowercase alpha, 1-8 chars.
    fn segment_strategy() -> impl Strategy<Value = String> {
        "[a-z]{1,8}".prop_map(|s| s)
    }

    /// Strategy to generate a vec of 1-5 path segments.
    fn segments_strategy() -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(segment_strategy(), 1..=5)
    }

    proptest! {
        /// **Validates: Requirements 4.1, 4.2, 4.3**
        ///
        /// Property 4: Module path resolution mapping
        /// For any `::` module path with N segments `s1::s2::...::sN` and a base directory `dir`,
        /// the module resolver SHALL resolve the path to `{dir}/s1/s2/.../sN.flux`.
        #[test]
        fn module_path_maps_to_filesystem(segments in segments_strategy()) {
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path().to_path_buf();

            // Build the module path string: s1::s2::...::sN
            let module_path = segments.join("::");

            // Create the directory structure and .flux file on disk
            let mut file_path = base_dir.clone();
            for seg in &segments[..segments.len() - 1] {
                file_path.push(seg);
            }
            fs::create_dir_all(&file_path).unwrap();
            file_path.push(format!("{}.flux", segments.last().unwrap()));
            fs::write(&file_path, "").unwrap();

            // Expected path is the canonicalized version of {base_dir}/s1/s2/.../sN.flux
            let expected = file_path.canonicalize().unwrap();

            // Call resolve_path
            let resolver = ModuleResolver::new(&base_dir);
            let result = resolver.resolve_path(&module_path, &base_dir);

            prop_assert!(result.is_ok(), "resolve_path failed for module_path='{}': {:?}", module_path, result.err());
            prop_assert_eq!(result.unwrap(), expected);
        }
    }

    // =========================================================================
    // Property 7: Circular import detection
    // =========================================================================

    /// Generate a cycle length between 2 and 5 (number of files in the cycle).
    fn cycle_length_strategy() -> impl Strategy<Value = usize> {
        2..=5usize
    }

    /// Generate unique file names for a cycle of given length.
    /// Returns names like ["mod_a", "mod_b", "mod_c", ...] to avoid collisions.
    fn file_names_for_cycle(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("mod_{}", (b'a' + i as u8) as char))
            .collect()
    }

    /// Create N flux library files in a `lib/` subdirectory that form a cycle.
    /// Uses a symlink `lib/peer -> lib/` so that imports like `from peer::mod_b`
    /// inside `lib/mod_a.flux` resolve back to `lib/mod_b.flux` (same directory
    /// via symlink), creating a genuine cycle detectable by the resolver.
    ///
    /// Returns the import path that the main file should use (to start the chain).
    fn create_cyclic_files(dir: &Path, n: usize) -> String {
        let names = file_names_for_cycle(n);
        let lib_dir = dir.join("lib");
        fs::create_dir_all(&lib_dir).unwrap();

        // Create symlink: lib/peer -> lib/ (allows sibling imports via peer::X)
        let peer_link = lib_dir.join("peer");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&lib_dir, &peer_link).unwrap();

        for i in 0..n {
            let next = (i + 1) % n; // wraps around to create cycle
            let next_name = &names[next];
            let current_name = &names[i];

            // Import from peer::mod_X resolves via symlink to lib/mod_X.flux
            let content = format!(
                "from peer::{next_name} import {{fn_{next_name}}}\n\
                 fn fn_{current_name}() {{\n\
                     return fn_{next_name}()\n\
                 }}\n"
            );

            let file_path = lib_dir.join(format!("{}.flux", current_name));
            fs::write(&file_path, content).unwrap();
        }

        // Main file imports from lib::mod_a
        format!("lib::{}", names[0])
    }

    /// Create N flux library files in a `lib/` subdirectory that form a linear chain (acyclic).
    /// Uses the same symlink structure as the cyclic case, but the last file has no imports
    /// (breaking the cycle).
    ///
    /// Returns the import path that the main file should use (the first in the chain).
    fn create_acyclic_files(dir: &Path, n: usize) -> String {
        let names = file_names_for_cycle(n);
        let lib_dir = dir.join("lib");
        fs::create_dir_all(&lib_dir).unwrap();

        // Create symlink: lib/peer -> lib/
        let peer_link = lib_dir.join("peer");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&lib_dir, &peer_link).unwrap();

        for i in 0..n {
            let current_name = &names[i];

            let content = if i < n - 1 {
                // Non-terminal: imports from the next file via peer:: symlink
                let next_name = &names[i + 1];
                format!(
                    "from peer::{next_name} import {{fn_{next_name}}}\n\
                     fn fn_{current_name}() {{\n\
                         return fn_{next_name}()\n\
                     }}\n"
                )
            } else {
                // Terminal: no imports, leaf function
                format!(
                    "fn fn_{current_name}() {{\n\
                         return 42\n\
                     }}\n"
                )
            };

            let file_path = lib_dir.join(format!("{}.flux", current_name));
            fs::write(&file_path, content).unwrap();
        }

        // Main file imports from lib::mod_a
        format!("lib::{}", names[0])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        /// **Validates: Requirements 7.1, 7.2, 7.3**
        ///
        /// Property 7: Circular import detection
        /// For any set of files whose import graph contains a cycle of any length,
        /// the module resolver SHALL detect the cycle and produce a `CircularImport` error.
        #[test]
        fn circular_imports_detected(n in cycle_length_strategy()) {
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path().to_path_buf();

            // Create cyclic library files
            let first_module_path = create_cyclic_files(&base_dir, n);
            let first_fn_name = format!("fn_mod_a");

            // Create a main file that imports from the first file in the cycle
            let main_content = format!(
                "from {first_module_path} import {{{first_fn_name}}}\n\
                 strategy Test {{\n\
                     on bar {{\n\
                         {first_fn_name}()\n\
                     }}\n\
                 }}\n"
            );
            let main_path = base_dir.join("main.flux");
            fs::write(&main_path, &main_content).unwrap();

            // Parse the main file
            let source = fs::read_to_string(&main_path).unwrap();
            let tokens = flux_compiler::lexer::lex_with_spans(&source).unwrap();
            let program = flux_compiler::parser::parse(tokens).unwrap();

            // Resolve modules — should fail with CircularImport
            let result = resolve_modules(program, &base_dir);

            prop_assert!(
                result.is_err(),
                "Expected CircularImport error for cycle of length {}, but resolution succeeded",
                n
            );

            match result.unwrap_err() {
                ModuleError::CircularImport { chain } => {
                    // The chain should have at least 2 entries (the repeated file appears at start and end)
                    prop_assert!(
                        chain.len() >= 2,
                        "CircularImport chain too short: {:?}",
                        chain
                    );
                    // The chain should show the cycle: last entry matches an earlier entry
                    let last = chain.last().unwrap();
                    prop_assert!(
                        chain[..chain.len() - 1].contains(last),
                        "Last entry in chain should repeat an earlier entry to show the cycle. Chain: {:?}",
                        chain
                    );
                }
                other => {
                    prop_assert!(false, "Expected CircularImport error, got: {:?}", other);
                }
            }
        }

        /// **Validates: Requirements 7.1, 7.2, 7.3**
        ///
        /// Property 7: Circular import detection (acyclic case)
        /// For any acyclic import graph, resolution SHALL succeed without error.
        #[test]
        fn acyclic_imports_succeed(n in cycle_length_strategy()) {
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path().to_path_buf();

            // Create acyclic (linear chain) library files
            let first_module_path = create_acyclic_files(&base_dir, n);
            let first_fn_name = format!("fn_mod_a");

            // Create a main file that imports from the first file in the chain
            let main_content = format!(
                "from {first_module_path} import {{{first_fn_name}}}\n\
                 strategy Test {{\n\
                     on bar {{\n\
                         {first_fn_name}()\n\
                     }}\n\
                 }}\n"
            );
            let main_path = base_dir.join("main.flux");
            fs::write(&main_path, &main_content).unwrap();

            // Parse the main file
            let source = fs::read_to_string(&main_path).unwrap();
            let tokens = flux_compiler::lexer::lex_with_spans(&source).unwrap();
            let program = flux_compiler::parser::parse(tokens).unwrap();

            // Resolve modules — should succeed
            let result = resolve_modules(program, &base_dir);

            prop_assert!(
                result.is_ok(),
                "Expected acyclic chain of length {} to resolve successfully, but got error: {:?}",
                n,
                result.err()
            );

            // Verify the imported function is included in the merged program
            let merged = result.unwrap();
            let fn_names: Vec<&str> = merged.functions.iter().map(|f| f.name.as_str()).collect();
            prop_assert!(
                fn_names.contains(&first_fn_name.as_str()),
                "Expected function '{}' in merged program, found: {:?}",
                first_fn_name,
                fn_names
            );
        }
    }

    // =========================================================================
    // Property 6: Selective inclusion completeness and minimality
    // =========================================================================

    /// The fixed library file template used for testing selective inclusion.
    /// Contains 5 functions with known call relationships:
    ///   - leaf1(): no calls
    ///   - leaf2(): no calls
    ///   - calls_leaf1(): calls leaf1
    ///   - calls_both(): calls leaf1 and leaf2
    ///   - isolated(): no calls
    ///
    /// Call graph:
    ///   calls_leaf1 -> leaf1
    ///   calls_both -> leaf1, leaf2
    ///   leaf1, leaf2, isolated are leaf nodes
    const LIB_SOURCE: &str = r#"fn leaf1() {
    return 1
}

fn leaf2() {
    return 2
}

fn calls_leaf1() {
    x = leaf1()
    return x
}

fn calls_both() {
    a = leaf1()
    b = leaf2()
    return a + b
}

fn isolated() {
    return 42
}
"#;

    /// All function names in the library
    const ALL_FN_NAMES: &[&str] = &["leaf1", "leaf2", "calls_leaf1", "calls_both", "isolated"];

    /// Compute the expected reachable set from a set of imported names using the known call graph.
    fn expected_reachable(imported: &HashSet<&str>) -> HashSet<&'static str> {
        let mut reachable: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = imported.iter().copied().collect();

        while let Some(name) = queue.pop_front() {
            if reachable.contains(name) {
                continue;
            }
            reachable.insert(match name {
                "leaf1" => "leaf1",
                "leaf2" => "leaf2",
                "calls_leaf1" => "calls_leaf1",
                "calls_both" => "calls_both",
                "isolated" => "isolated",
                _ => continue,
            });

            // Add transitive dependencies based on known call graph
            match name {
                "calls_leaf1" => {
                    queue.push_back("leaf1");
                }
                "calls_both" => {
                    queue.push_back("leaf1");
                    queue.push_back("leaf2");
                }
                _ => {}
            }
        }

        reachable
    }

    /// Strategy: generate a non-empty subset of function names to import.
    fn import_subset_strategy() -> impl Strategy<Value = Vec<String>> {
        // Generate a non-empty subset of indices into ALL_FN_NAMES
        hash_set(0..ALL_FN_NAMES.len(), 1..=ALL_FN_NAMES.len()).prop_map(|indices| {
            indices
                .into_iter()
                .map(|i| ALL_FN_NAMES[i].to_string())
                .collect::<Vec<_>>()
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Validates: Requirements 6.1, 6.2, 6.3, 6.4**
        ///
        /// Property 6: Selective inclusion completeness and minimality.
        /// For any library file with functions {f1, f2, ..., fN} and a set of explicitly
        /// imported names S, the merged program SHALL contain exactly those functions
        /// reachable from S via the call graph and no others.
        #[test]
        fn selective_inclusion_completeness_and_minimality(
            imported_names in import_subset_strategy()
        ) {
            // Set up temp directory with library file
            let tmp_dir = TempDir::new().unwrap();
            let lib_dir = tmp_dir.path().join("lib");
            std::fs::create_dir_all(&lib_dir).unwrap();
            let lib_file = lib_dir.join("helpers.flux");
            std::fs::write(&lib_file, LIB_SOURCE).unwrap();

            // Create main file that imports the selected subset
            let import_list = imported_names.join(", ");
            let main_source = format!(
                "from lib::helpers import {{{}}}\n\nstrategy Test {{\n    on bar {{\n        x = 1\n    }}\n}}\n",
                import_list
            );
            let main_file = tmp_dir.path().join("strategy.flux");
            std::fs::write(&main_file, &main_source).unwrap();

            // Resolve modules
            let tokens = flux_compiler::lexer::lex_with_spans(&main_source).unwrap();
            let program = flux_compiler::parser::parse(tokens).unwrap();
            let resolved = resolve_modules(program, tmp_dir.path()).unwrap();

            // Compute expected reachable set
            let imported_set: HashSet<&str> = imported_names.iter().map(|s| s.as_str()).collect();
            let expected = expected_reachable(&imported_set);

            // Get actual included function names (excluding main file functions)
            // The main file has no functions defined, so all functions in resolved.functions
            // come from the import.
            let actual: HashSet<&str> = resolved
                .functions
                .iter()
                .map(|f| f.name.as_str())
                .collect();

            // Completeness: all expected functions are included
            for name in &expected {
                prop_assert!(
                    actual.contains(name),
                    "Completeness violation: function '{}' should be included (reachable from {:?}) but was not. Actual: {:?}",
                    name,
                    imported_names,
                    actual
                );
            }

            // Minimality: no extra functions are included
            for name in &actual {
                prop_assert!(
                    expected.contains(name),
                    "Minimality violation: function '{}' was included but is not reachable from {:?}. Expected: {:?}",
                    name,
                    imported_names,
                    expected
                );
            }
        }
    }

    // =========================================================================
    // Property 8: Merge preserves function identity
    // =========================================================================

    /// Strategy to generate a valid Flux identifier for function/param names.
    /// Starts with a lowercase letter, followed by lowercase letters/digits/underscores.
    fn ident_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,7}".prop_map(|s| {
            if FLUX_KEYWORDS_IDENT.contains(&s.as_str()) {
                format!("{}_v", s)
            } else {
                s
            }
        })
    }

    /// Flux keywords that must not be generated as identifiers (used by ident_strategy).
    const FLUX_KEYWORDS_IDENT: &[&str] = &[
        "strategy", "params", "state", "on", "if", "elif", "else", "while",
        "return", "fn", "from", "import", "and", "or", "not", "true", "false",
        "null", "data", "connector", "bar", "for", "in",
    ];

    /// Strategy to generate a list of unique parameter names (0-4 params).
    fn params_strategy() -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(ident_strategy(), 0..=4).prop_map(|params| {
            // Deduplicate by appending index suffix
            let mut seen = HashSet::new();
            let mut unique = Vec::new();
            for (i, p) in params.into_iter().enumerate() {
                let name = if seen.contains(&p) {
                    format!("{}{}", p, i)
                } else {
                    p
                };
                seen.insert(name.clone());
                unique.push(name);
            }
            unique
        })
    }

    /// A generated function definition for testing identity preservation.
    #[derive(Debug, Clone)]
    struct GenFnDef {
        name: String,
        params: Vec<String>,
    }

    /// Strategy to generate 1-3 function definitions with unique names.
    fn fn_defs_strategy() -> impl Strategy<Value = Vec<GenFnDef>> {
        prop::collection::vec((ident_strategy(), params_strategy()), 1..=3).prop_map(|fns| {
            let mut seen = HashSet::new();
            let mut result = Vec::new();
            for (i, (name, params)) in fns.into_iter().enumerate() {
                let fn_name = if seen.contains(&name) {
                    format!("{}_{}", name, i)
                } else {
                    name
                };
                seen.insert(fn_name.clone());
                result.push(GenFnDef {
                    name: fn_name,
                    params,
                });
            }
            result
        })
    }

    /// Generate Flux source for a function with a simple return body.
    fn gen_fn_source(f: &GenFnDef) -> String {
        let params_str = f.params.join(", ");
        let body = if f.params.is_empty() {
            "    return 0".to_string()
        } else {
            format!("    return {}", f.params[0])
        };
        format!("fn {}({}) {{\n{}\n}}\n", f.name, params_str, body)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Validates: Requirements 8.1, 8.2**
        ///
        /// Property 8: Merge preserves function identity
        /// For any imported FnDef, it SHALL appear in the merged Program.functions vector
        /// with its original name, parameters, and body unchanged. The merge operation SHALL
        /// not rename, prefix, or modify imported functions.
        #[test]
        fn merge_preserves_function_identity(fn_defs in fn_defs_strategy()) {
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path().to_path_buf();

            // Create a library subdirectory
            let lib_dir = base_dir.join("lib");
            fs::create_dir_all(&lib_dir).unwrap();

            // Generate library file content with the random function definitions
            let lib_source: String = fn_defs.iter()
                .map(|f| gen_fn_source(f))
                .collect::<Vec<_>>()
                .join("\n");
            let lib_path = lib_dir.join("helpers.flux");
            fs::write(&lib_path, &lib_source).unwrap();

            // Build the import names (all functions from the library)
            let import_list = fn_defs.iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>()
                .join(", ");

            // Create main file that imports all library functions
            let main_source = format!(
                "from lib::helpers import {{{}}}\n\nstrategy Test {{\n    on bar {{\n        x = 1\n    }}\n}}\n",
                import_list
            );
            fs::write(base_dir.join("main.flux"), &main_source).unwrap();

            // Parse and resolve
            let tokens = flux_compiler::lexer::lex_with_spans(&main_source).unwrap();
            let program = flux_compiler::parser::parse(tokens).unwrap();
            let result = resolve_modules(program, &base_dir);

            prop_assert!(result.is_ok(), "resolve_modules failed: {:?}", result.err());
            let merged = result.unwrap();

            // Verify each imported function preserves identity
            for expected_fn in &fn_defs {
                let found = merged.functions.iter().find(|f| f.name == expected_fn.name);
                prop_assert!(
                    found.is_some(),
                    "Function '{}' not found in merged program. Found: {:?}",
                    expected_fn.name,
                    merged.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
                );

                let found_fn = found.unwrap();

                // Name preserved exactly (no prefixing, renaming, or modification)
                prop_assert_eq!(
                    &found_fn.name,
                    &expected_fn.name,
                    "Function name was modified during merge"
                );

                // Parameters preserved in exact order
                let found_param_names: Vec<&str> =
                    found_fn.params.iter().map(|p| p.name.as_str()).collect();
                let expected_param_names: Vec<&str> =
                    expected_fn.params.iter().map(|p| p.as_str()).collect();
                prop_assert_eq!(
                    &found_param_names,
                    &expected_param_names,
                    "Function '{}' params were modified during merge. Expected {:?}, got {:?}",
                    expected_fn.name,
                    expected_fn.params,
                    found_param_names
                );

                // Body structure preserved (should be a return statement)
                prop_assert!(
                    !found_fn.body.is_empty(),
                    "Function '{}' body is empty after merge",
                    expected_fn.name
                );
                match &found_fn.body[0] {
                    Stmt::Return(ret) => {
                        prop_assert!(
                            ret.value.is_some(),
                            "Function '{}' return value was lost during merge",
                            expected_fn.name
                        );
                    }
                    other => {
                        prop_assert!(
                            false,
                            "Function '{}' body structure modified during merge: expected Return, got {:?}",
                            expected_fn.name,
                            other
                        );
                    }
                }
            }
        }
    }

    // =========================================================================
    // Property 9: Built-in import passthrough
    // =========================================================================

    /// Strategy to generate a valid non-keyword identifier for module path segments.
    /// Uses a "mod_" prefix to guarantee it never collides with keywords.
    fn non_keyword_ident_strategy() -> impl Strategy<Value = String> {
        "[a-z]{2,6}".prop_map(|s| format!("mod_{}", s))
    }

    /// Strategy to generate a dot-separated module path (e.g., "mod_indicators", "mod_math.mod_stats").
    /// These are built-in imports that should NOT be resolved on the filesystem.
    fn builtin_module_path_strategy() -> impl Strategy<Value = String> {
        // 1-3 segments joined by dots
        prop::collection::vec(non_keyword_ident_strategy(), 1..=3)
            .prop_map(|segments| segments.join("."))
    }

    /// Strategy to generate a valid non-keyword function name for import lists.
    fn non_keyword_fn_name_strategy() -> impl Strategy<Value = String> {
        "[a-z]{2,6}".prop_map(|s| format!("fn_{}", s))
    }

    /// Strategy to generate a list of 1-4 unique function names to import.
    fn import_names_strategy() -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(non_keyword_fn_name_strategy(), 1..=4).prop_map(|names| {
            // Deduplicate
            let mut seen = HashSet::new();
            let mut unique = Vec::new();
            for (i, name) in names.into_iter().enumerate() {
                let n = if seen.contains(&name) {
                    format!("{}_{}", name, i)
                } else {
                    name
                };
                seen.insert(n.clone());
                unique.push(n);
            }
            unique
        })
    }

    /// A generated built-in import for testing passthrough.
    #[derive(Debug, Clone)]
    struct GenBuiltinImport {
        module_path: String,
        names: Vec<String>,
    }

    /// Strategy to generate 1-4 built-in imports.
    fn builtin_imports_strategy() -> impl Strategy<Value = Vec<GenBuiltinImport>> {
        prop::collection::vec(
            (builtin_module_path_strategy(), import_names_strategy()),
            1..=4,
        )
        .prop_map(|pairs| {
            pairs
                .into_iter()
                .map(|(module_path, names)| GenBuiltinImport { module_path, names })
                .collect()
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Validates: Requirements 9.1, 9.3**
        ///
        /// Property 9: Built-in import passthrough
        /// For any dot-separated import (no `::` in module_path), the module resolver
        /// SHALL leave it in Program.imports unchanged and not attempt filesystem resolution.
        #[test]
        fn builtin_imports_passthrough(
            builtin_imports in builtin_imports_strategy()
        ) {
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path().to_path_buf();

            // Build import statements for built-in (dot-separated) imports
            let import_lines: Vec<String> = builtin_imports.iter().map(|imp| {
                format!("from {} import {{{}}}", imp.module_path, imp.names.join(", "))
            }).collect();

            // Build a main file with only built-in imports (no file-module imports)
            let main_source = format!(
                "{}\n\nstrategy Test {{\n    on bar {{\n        x = 1\n    }}\n}}\n",
                import_lines.join("\n")
            );

            // Parse the main file
            let tokens = flux_compiler::lexer::lex_with_spans(&main_source).unwrap();
            let program = flux_compiler::parser::parse(tokens).unwrap();

            // Record the original imports before resolution
            let original_imports: Vec<(String, Vec<String>)> = program
                .imports
                .iter()
                .map(|imp| (imp.module_path.clone(), imp.names.clone()))
                .collect();

            // Resolve modules — should succeed without attempting filesystem resolution
            let result = resolve_modules(program, &base_dir);

            prop_assert!(
                result.is_ok(),
                "resolve_modules should succeed with only built-in imports, but got: {:?}",
                result.err()
            );

            let resolved = result.unwrap();

            // All built-in imports should remain in program.imports unchanged
            prop_assert_eq!(
                resolved.imports.len(),
                original_imports.len(),
                "Number of imports changed after resolution. Expected {}, got {}",
                original_imports.len(),
                resolved.imports.len()
            );

            for (i, (expected_path, expected_names)) in original_imports.iter().enumerate() {
                let actual = &resolved.imports[i];
                prop_assert_eq!(
                    &actual.module_path,
                    expected_path,
                    "Import {} module_path changed: expected '{}', got '{}'",
                    i,
                    expected_path,
                    actual.module_path
                );
                prop_assert_eq!(
                    &actual.names,
                    expected_names,
                    "Import {} names changed: expected {:?}, got {:?}",
                    i,
                    expected_names,
                    actual.names
                );
            }

            // No functions should have been added (no file-module imports to resolve)
            prop_assert!(
                resolved.functions.is_empty(),
                "No functions should be added when only built-in imports are present. Got: {:?}",
                resolved.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
            );
        }

        /// **Validates: Requirements 9.1, 9.3**
        ///
        /// Property 9: Built-in import passthrough (mixed with file-module imports)
        /// When a file contains both built-in and file-module imports, the module resolver
        /// SHALL process only the file-module imports and pass built-in imports through unchanged.
        #[test]
        fn builtin_imports_passthrough_mixed(
            builtin_imports in builtin_imports_strategy()
        ) {
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path().to_path_buf();

            // Create a library file so the file-module import succeeds
            let lib_dir = base_dir.join("lib");
            fs::create_dir_all(&lib_dir).unwrap();
            let lib_source = "fn helper() {\n    return 42\n}\n";
            fs::write(lib_dir.join("utils.flux"), lib_source).unwrap();

            // Build import lines: built-in imports + one file-module import
            let mut import_lines: Vec<String> = builtin_imports.iter().map(|imp| {
                format!("from {} import {{{}}}", imp.module_path, imp.names.join(", "))
            }).collect();
            import_lines.push("from lib::utils import {helper}".to_string());

            let main_source = format!(
                "{}\n\nstrategy Test {{\n    on bar {{\n        x = 1\n    }}\n}}\n",
                import_lines.join("\n")
            );

            // Parse the main file
            let tokens = flux_compiler::lexer::lex_with_spans(&main_source).unwrap();
            let program = flux_compiler::parser::parse(tokens).unwrap();

            // Record the built-in imports before resolution (those without `::`)
            let original_builtins: Vec<(String, Vec<String>)> = program
                .imports
                .iter()
                .filter(|imp| !imp.module_path.contains("::"))
                .map(|imp| (imp.module_path.clone(), imp.names.clone()))
                .collect();

            // Resolve modules
            let result = resolve_modules(program, &base_dir);

            prop_assert!(
                result.is_ok(),
                "resolve_modules failed: {:?}",
                result.err()
            );

            let resolved = result.unwrap();

            // After resolution, program.imports should contain only the built-in imports
            // (the file-module import should have been removed and its functions merged)
            prop_assert_eq!(
                resolved.imports.len(),
                original_builtins.len(),
                "After resolution, imports should only contain built-in imports. Expected {}, got {}. Imports: {:?}",
                original_builtins.len(),
                resolved.imports.len(),
                resolved.imports.iter().map(|i| &i.module_path).collect::<Vec<_>>()
            );

            // Each built-in import should be preserved unchanged
            for (i, (expected_path, expected_names)) in original_builtins.iter().enumerate() {
                let actual = &resolved.imports[i];
                prop_assert_eq!(
                    &actual.module_path,
                    expected_path,
                    "Built-in import {} module_path changed in mixed mode: expected '{}', got '{}'",
                    i,
                    expected_path,
                    actual.module_path
                );
                prop_assert_eq!(
                    &actual.names,
                    expected_names,
                    "Built-in import {} names changed in mixed mode: expected {:?}, got {:?}",
                    i,
                    expected_names,
                    actual.names
                );
            }

            // The file-module import's function should have been merged
            let fn_names: Vec<&str> = resolved.functions.iter().map(|f| f.name.as_str()).collect();
            prop_assert!(
                fn_names.contains(&"helper"),
                "File-module import's function 'helper' should be in merged functions. Got: {:?}",
                fn_names
            );
        }
    }
}
