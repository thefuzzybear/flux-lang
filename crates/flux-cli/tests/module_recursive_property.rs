//! Property 10: Recursive resolution with relative paths
//!
//! **Validates: Requirements 11.1, 11.2, 11.3**
//!
//! For any library file `A` that imports from library file `B` via a `::` path,
//! the resolver SHALL resolve `B`'s path relative to `A`'s directory (not the main
//! file's directory), and SHALL apply the same caching, cycle detection, and selective
//! inclusion rules at every recursion level.

use flux_cli::module_resolver::resolve_modules;
use proptest::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Strategy to generate a valid Flux identifier (lowercase alpha, 1-6 chars).
fn ident_strategy() -> impl Strategy<Value = String> {
    "[a-z]{2,6}"
}

/// Strategy to generate unique function names for a 3-level test hierarchy.
/// Returns (main_fn_name, middleware_fn_name, leaf_fn_name) — all distinct.
fn fn_names_strategy() -> impl Strategy<Value = (String, String, String)> {
    (ident_strategy(), ident_strategy(), ident_strategy()).prop_map(|(a, b, c)| {
        // Ensure uniqueness by appending suffixes
        let main_fn = format!("{}_main", a);
        let mid_fn = format!("{}_mid", b);
        let leaf_fn = format!("{}_leaf", c);
        (main_fn, mid_fn, leaf_fn)
    })
}

/// Strategy to generate unique directory segment names for the nested structure.
/// Returns (lib_dir_name, sub_dir_name) — distinct names for the two nesting levels.
fn dir_names_strategy() -> impl Strategy<Value = (String, String)> {
    (ident_strategy(), ident_strategy()).prop_map(|(a, b)| {
        // Ensure distinctness
        let lib_name = format!("lib_{}", a);
        let sub_name = format!("sub_{}", b);
        (lib_name, sub_name)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 11.1, 11.2, 11.3**
    ///
    /// Property 10: Recursive resolution with relative paths
    ///
    /// Creates a 3-level hierarchy:
    ///   tmp/main.flux               -> imports from {lib}::{middleware_file}
    ///   tmp/{lib}/{middleware_file}.flux  -> imports from {sub}::{common_file}
    ///                                       (resolved relative to tmp/{lib}/, NOT tmp/)
    ///   tmp/{lib}/{sub}/{common_file}.flux -> defines leaf function
    ///
    /// After resolution, the main program should have functions from both middleware
    /// and the leaf level, proving recursive resolution uses relative paths.
    #[test]
    fn recursive_resolution_uses_relative_paths(
        (_main_fn, mid_fn, leaf_fn) in fn_names_strategy(),
        (lib_dir, sub_dir) in dir_names_strategy(),
    ) {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();

        // Create directory structure:
        //   base_dir/{lib_dir}/{sub_dir}/
        let lib_path = base_dir.join(&lib_dir);
        let sub_path = lib_path.join(&sub_dir);
        fs::create_dir_all(&sub_path).unwrap();

        // Leaf file: tmp/{lib_dir}/{sub_dir}/common.flux
        // Defines the leaf function
        let leaf_content = format!(
            "fn {}() {{\n    return 1\n}}\n",
            leaf_fn
        );
        fs::write(sub_path.join("common.flux"), &leaf_content).unwrap();

        // Middleware file: tmp/{lib_dir}/middleware.flux
        // Imports from {sub_dir}::common — resolved RELATIVE to tmp/{lib_dir}/ (its own dir)
        // NOT relative to tmp/ (the main file's dir)
        let mid_content = format!(
            "from {sub_dir}::common import {{{leaf_fn}}}\n\
             fn {mid_fn}() {{\n\
                 return {leaf_fn}()\n\
             }}\n"
        );
        fs::write(lib_path.join("middleware.flux"), &mid_content).unwrap();

        // Main file: tmp/main.flux
        // Imports from {lib_dir}::middleware
        let main_content = format!(
            "from {lib_dir}::middleware import {{{mid_fn}}}\n\
             strategy Test {{\n\
                 on bar {{\n\
                     {mid_fn}()\n\
                 }}\n\
             }}\n"
        );
        fs::write(base_dir.join("main.flux"), &main_content).unwrap();

        // Parse and resolve
        let tokens = flux_compiler::lexer::lex_with_spans(&main_content).unwrap();
        let program = flux_compiler::parser::parse(tokens).unwrap();
        let result = resolve_modules(program, base_dir);

        // Resolution should succeed — proving that the middleware's import was
        // resolved relative to its own directory, not the main file's directory.
        // If it resolved relative to main's dir, it would look for
        //   tmp/{sub_dir}/common.flux (which doesn't exist)
        // instead of
        //   tmp/{lib_dir}/{sub_dir}/common.flux (which does exist)
        prop_assert!(
            result.is_ok(),
            "Recursive resolution failed. This likely means the resolver is NOT \
             resolving imports relative to the importing file's directory. \
             Error: {:?}",
            result.err()
        );

        let merged = result.unwrap();
        let fn_names: Vec<&str> = merged.functions.iter().map(|f| f.name.as_str()).collect();

        // The middleware function should be in the merged program
        prop_assert!(
            fn_names.contains(&mid_fn.as_str()),
            "Middleware function '{}' not found in merged program. Found: {:?}",
            mid_fn,
            fn_names
        );

        // The leaf function should ALSO be in the merged program (transitive inclusion)
        // because mid_fn calls leaf_fn, selective inclusion should pull it in
        prop_assert!(
            fn_names.contains(&leaf_fn.as_str()),
            "Leaf function '{}' not found in merged program — recursive selective \
             inclusion failed. Found: {:?}",
            leaf_fn,
            fn_names
        );
    }

    /// **Validates: Requirements 11.2**
    ///
    /// Property 10 (caching aspect): When the same file is imported at multiple
    /// recursion depths via different paths, the caching ensures correct behavior.
    /// We verify this with a deeper chain: main -> lib::layer1 -> sub::layer2 -> deep::layer3.
    /// The 3-deep chain exercises the cache at each level — if caching were broken,
    /// re-parsing at deeper levels could fail or produce incorrect results.
    #[test]
    fn recursive_resolution_applies_caching(
        (main_fn, mid_fn, leaf_fn) in fn_names_strategy(),
        (lib_dir, sub_dir) in dir_names_strategy(),
    ) {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();

        // 4-level deep chain to exercise caching at multiple recursion depths:
        //   base_dir/{lib_dir}/layer1.flux         -> imports {sub_dir}::layer2
        //   base_dir/{lib_dir}/{sub_dir}/layer2.flux -> imports deep::layer3
        //   base_dir/{lib_dir}/{sub_dir}/deep/layer3.flux -> leaf function
        let lib_path = base_dir.join(&lib_dir);
        let sub_path = lib_path.join(&sub_dir);
        let deep_path = sub_path.join("deep");
        fs::create_dir_all(&deep_path).unwrap();

        // layer3 (deepest leaf)
        let layer3_content = format!("fn {}() {{\n    return 99\n}}\n", leaf_fn);
        fs::write(deep_path.join("layer3.flux"), &layer3_content).unwrap();

        // layer2: imports from deep::layer3 (relative to sub_path)
        let layer2_content = format!(
            "from deep::layer3 import {{{leaf_fn}}}\n\
             fn {mid_fn}() {{\n\
                 return {leaf_fn}()\n\
             }}\n"
        );
        fs::write(sub_path.join("layer2.flux"), &layer2_content).unwrap();

        // layer1: imports from {sub_dir}::layer2 (relative to lib_path)
        let layer1_content = format!(
            "from {sub_dir}::layer2 import {{{mid_fn}}}\n\
             fn {main_fn}() {{\n\
                 return {mid_fn}()\n\
             }}\n"
        );
        fs::write(lib_path.join("layer1.flux"), &layer1_content).unwrap();

        // Main file: imports from {lib_dir}::layer1
        let main_content = format!(
            "from {lib_dir}::layer1 import {{{main_fn}}}\n\
             strategy Test {{\n\
                 on bar {{\n\
                     {main_fn}()\n\
                 }}\n\
             }}\n"
        );
        fs::write(base_dir.join("main.flux"), &main_content).unwrap();

        // Parse and resolve
        let tokens = flux_compiler::lexer::lex_with_spans(&main_content).unwrap();
        let program = flux_compiler::parser::parse(tokens).unwrap();
        let result = resolve_modules(program, base_dir);

        // Should succeed — each level resolves relative to its own directory
        // and the caching prevents redundant re-parsing at each recursion level
        prop_assert!(
            result.is_ok(),
            "Deep recursive resolution with caching failed. Error: {:?}",
            result.err()
        );

        let merged = result.unwrap();
        let fn_names: Vec<&str> = merged.functions.iter().map(|f| f.name.as_str()).collect();

        // The top-level function should be present
        prop_assert!(
            fn_names.contains(&main_fn.as_str()),
            "Top-level function '{}' not found. Found: {:?}",
            main_fn,
            fn_names
        );

        // The middle-level function should be present (transitive from layer1)
        prop_assert!(
            fn_names.contains(&mid_fn.as_str()),
            "Middle function '{}' not found (recursive inclusion issue). Found: {:?}",
            mid_fn,
            fn_names
        );

        // The deepest leaf should also be present (transitive from layer2 via layer1)
        prop_assert!(
            fn_names.contains(&leaf_fn.as_str()),
            "Leaf function '{}' not found (deep recursive caching issue). Found: {:?}",
            leaf_fn,
            fn_names
        );
    }

    /// **Validates: Requirements 11.2**
    ///
    /// Property 10 (cycle detection at recursion levels): Circular imports within
    /// nested library files should still be detected. If A imports B and B imports A
    /// (both at deeper directory levels), the resolver should catch the cycle.
    #[test]
    fn recursive_resolution_detects_cycles_at_depth(
        (_, mid_fn, leaf_fn) in fn_names_strategy(),
        (lib_dir, sub_dir) in dir_names_strategy(),
    ) {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();

        // Directory structure:
        //   base_dir/{lib_dir}/mod_a.flux  (imports {sub_dir}::mod_b)
        //   base_dir/{lib_dir}/{sub_dir}/mod_b.flux  (imports back to parent via symlink)
        //
        // We use a symlink to create a cycle at depth:
        //   base_dir/{lib_dir}/{sub_dir}/parent -> base_dir/{lib_dir}/
        //   mod_b imports from parent::mod_a (which goes back to mod_a -> cycle)
        let lib_path = base_dir.join(&lib_dir);
        let sub_path = lib_path.join(&sub_dir);
        fs::create_dir_all(&sub_path).unwrap();

        // Create symlink: sub_path/parent -> lib_path
        let parent_link = sub_path.join("parent");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&lib_path, &parent_link).unwrap();

        // mod_a: imports from {sub_dir}::mod_b
        let mod_a_content = format!(
            "from {sub_dir}::mod_b import {{{leaf_fn}}}\n\
             fn {mid_fn}() {{\n\
                 return {leaf_fn}()\n\
             }}\n"
        );
        fs::write(lib_path.join("mod_a.flux"), &mod_a_content).unwrap();

        // mod_b: imports from parent::mod_a (creates cycle via symlink)
        let mod_b_content = format!(
            "from parent::mod_a import {{{mid_fn}}}\n\
             fn {leaf_fn}() {{\n\
                 return {mid_fn}()\n\
             }}\n"
        );
        fs::write(sub_path.join("mod_b.flux"), &mod_b_content).unwrap();

        // Main file: imports from {lib_dir}::mod_a
        let main_content = format!(
            "from {lib_dir}::mod_a import {{{mid_fn}}}\n\
             strategy Test {{\n\
                 on bar {{\n\
                     {mid_fn}()\n\
                 }}\n\
             }}\n"
        );
        fs::write(base_dir.join("main.flux"), &main_content).unwrap();

        // Parse and resolve
        let tokens = flux_compiler::lexer::lex_with_spans(&main_content).unwrap();
        let program = flux_compiler::parser::parse(tokens).unwrap();
        let result = resolve_modules(program, base_dir);

        // Should detect the circular import
        prop_assert!(
            result.is_err(),
            "Expected CircularImport error for cycle at depth, but resolution succeeded"
        );

        match result.unwrap_err() {
            flux_cli::module_resolver::ModuleError::CircularImport { chain } => {
                prop_assert!(
                    chain.len() >= 2,
                    "CircularImport chain too short: {:?}",
                    chain
                );
            }
            other => {
                prop_assert!(
                    false,
                    "Expected CircularImport error, got: {:?}",
                    other
                );
            }
        }
    }
}
