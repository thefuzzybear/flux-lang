//! Property-based tests for list mutation operations.
//!
//! These tests validate that the Flux interpreter correctly handles list
//! mutation methods (push, pop, remove, insert, sort_by, index assignment).
//!
//! Feature: list-mutation-ops, Property 1: Push appends element and preserves prefix

use proptest::prelude::*;
use std::collections::HashMap;

use flux_compiler::lexer::Span;
use flux_compiler::typeck::typed_ast::*;
use flux_compiler::typeck::types::FluxType;

use flux_cli::interpreter::{Interpreter, Value};

// =============================================================================
// Helpers
// =============================================================================

/// Shortcut to build a TypedExpr with a given kind, type, and dummy span.
fn texpr(kind: TypedExprKind, ty: FluxType) -> TypedExpr {
    TypedExpr {
        kind,
        resolved_type: ty,
        span: Span::new(0, 0),
    }
}

/// Build a minimal Interpreter with an empty program for direct method testing.
fn minimal_interpreter() -> Interpreter {
    let program = TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![],
        impl_blocks: vec![],
        traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "ListMutTest".to_string(),
            body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                event_name: "bar".to_string(),
                body: vec![],
                span: Span::new(0, 0),
            })],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    };
    Interpreter::new(&program)
}

/// Build an identifier expression with a given type.
fn ident_expr_typed(name: &str, ty: FluxType) -> TypedExpr {
    texpr(TypedExprKind::Ident(name.to_string()), ty)
}

/// Compare two Values for equality (Value doesn't derive PartialEq).
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => (x - y).abs() < 1e-10,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (Value::List(xs), Value::List(ys)) => {
            xs.len() == ys.len()
                && xs.iter().zip(ys.iter()).all(|(a, b)| values_equal(a, b))
        }
        _ => false,
    }
}

// =============================================================================
// Strategies for proptest
// =============================================================================

/// Generate a random simple Value (Int or Float) for list elements.
fn arb_element() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<i64>().prop_map(Value::Int),
        (-1e6f64..1e6f64)
            .prop_filter("must be finite", |f| f.is_finite())
            .prop_map(Value::Float),
    ]
}

/// Generate a random list of Value elements (0..20 items).
fn arb_list() -> impl Strategy<Value = Vec<Value>> {
    prop::collection::vec(arb_element(), 0..20)
}

// =============================================================================
// Property 1: Push appends element and preserves prefix
// Feature: list-mutation-ops, Property 1: Push appends element and preserves prefix
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    /// **Validates: Requirements 1.1, 1.2**
    ///
    /// Property 1: Push appends element and preserves prefix
    ///
    /// For any list of values and any valid element, calling `push(element)` SHALL
    /// produce a new list where the last element equals the pushed value, all preceding
    /// elements are unchanged, and the length is exactly one greater than the original.
    #[test]
    fn prop_push_appends_and_preserves_prefix(
        initial_items in arb_list(),
        new_element in arb_element(),
    ) {
        let mut interp = minimal_interpreter();

        // Set up locals with the list variable
        let locals: HashMap<String, Value> = {
            let mut m = HashMap::new();
            m.insert("__list".to_string(), Value::List(initial_items.clone()));
            m
        };

        // Build the push MethodCall expression:
        //   __list.push(new_element_literal)
        let element_expr = match &new_element {
            Value::Int(i) => texpr(TypedExprKind::IntLiteral(*i), FluxType::Int),
            Value::Float(f) => texpr(TypedExprKind::FloatLiteral(*f), FluxType::Float),
            _ => unreachable!("arb_element only generates Int or Float"),
        };

        let push_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "__list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "push".to_string(),
                args: vec![element_expr],
            },
            FluxType::List(Box::new(FluxType::Int)),
        );

        // Evaluate the push call
        let result = interp.eval_expr(&push_expr, &locals).unwrap();

        // Extract the resulting list
        let result_items = match &result {
            Value::List(items) => items,
            other => panic!("push should return Value::List, got {:?}", other),
        };

        // Property: length is exactly one greater
        prop_assert_eq!(
            result_items.len(),
            initial_items.len() + 1,
            "push should increase list length by exactly 1"
        );

        // Property: last element equals the pushed value
        let last = &result_items[result_items.len() - 1];
        prop_assert!(
            values_equal(last, &new_element),
            "last element should equal pushed value.\nExpected: {:?}\nGot: {:?}",
            new_element,
            last
        );

        // Property: all preceding elements are unchanged (prefix preserved)
        for i in 0..initial_items.len() {
            prop_assert!(
                values_equal(&result_items[i], &initial_items[i]),
                "element at index {} should be unchanged after push.\nExpected: {:?}\nGot: {:?}",
                i,
                initial_items[i],
                result_items[i]
            );
        }
    }
}

// =============================================================================
// Property 2: Pop removes last element and returns it
// Feature: list-mutation-ops, Property 2: Pop removes last element and returns it
// =============================================================================

/// Generate a non-empty list of Value elements (1..=20 items).
fn arb_non_empty_list() -> impl Strategy<Value = Vec<Value>> {
    prop::collection::vec(arb_element(), 1..=20)
}

/// Build an expression statement wrapping a TypedExpr.
fn expr_stmt(expr: TypedExpr) -> TypedStmt {
    TypedStmt::Expr(TypedExprStmt {
        expr,
        span: Span::new(0, 0),
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    /// **Validates: Requirements 2.1, 2.2**
    ///
    /// Property 2: Pop removes last element and returns it
    ///
    /// For any non-empty list, calling `pop()` SHALL return the original last element,
    /// and the resulting list SHALL equal the original list with the last element
    /// removed (i.e., the prefix of length n-1).
    #[test]
    fn prop_pop_removes_last_returns_it(
        initial_items in arb_non_empty_list(),
    ) {
        let mut interp = minimal_interpreter();

        // Place the list in interpreter state
        interp.state.insert("__list".to_string(), Value::List(initial_items.clone()));

        // Build the pop MethodCall expression: __list.pop()
        let pop_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "__list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "pop".to_string(),
                args: vec![],
            },
            FluxType::Int,
        );

        // Evaluate the pop call — should return the last element
        let locals = HashMap::new();
        let result = interp.eval_expr(&pop_expr, &locals).unwrap();

        // Property: return value equals the original last element
        let expected_last = &initial_items[initial_items.len() - 1];
        prop_assert!(
            values_equal(&result, expected_last),
            "pop() should return the last element.\nExpected: {:?}\nGot: {:?}",
            expected_last,
            result
        );

        // Property: pending_list_mutation holds the modified list (prefix of length n-1)
        let modified = interp.pending_list_mutation.take();
        prop_assert!(
            modified.is_some(),
            "pending_list_mutation should hold the modified list after pop"
        );

        let modified_items = match modified.unwrap() {
            Value::List(items) => items,
            other => {
                prop_assert!(false, "pending_list_mutation should be a List, got {:?}", other);
                unreachable!()
            }
        };

        // Property: resulting list has length n-1
        prop_assert_eq!(
            modified_items.len(),
            initial_items.len() - 1,
            "Pop should decrease list length by exactly 1"
        );

        // Property: resulting list equals the prefix of the original
        for i in 0..modified_items.len() {
            prop_assert!(
                values_equal(&modified_items[i], &initial_items[i]),
                "element at index {} should be unchanged after pop.\nExpected: {:?}\nGot: {:?}",
                i,
                initial_items[i],
                modified_items[i]
            );
        }

        // Also verify end-to-end via exec_stmt (implicit reassignment writes back to state)
        let mut interp2 = minimal_interpreter();
        interp2.state.insert("__list".to_string(), Value::List(initial_items.clone()));

        let pop_expr2 = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "__list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "pop".to_string(),
                args: vec![],
            },
            FluxType::Int,
        );
        let stmt = expr_stmt(pop_expr2);
        let mut locals2 = HashMap::new();
        let exec_result = interp2.exec_stmt(&stmt, &mut locals2);
        prop_assert!(exec_result.is_ok(), "exec_stmt for pop should not error: {:?}", exec_result.err());

        // After implicit reassignment, state should hold the prefix
        let updated = interp2.state.get("__list").unwrap();
        let updated_items = match updated {
            Value::List(items) => items,
            other => {
                prop_assert!(false, "State '__list' should be a List after pop, got {:?}", other);
                unreachable!()
            }
        };

        prop_assert_eq!(
            updated_items.len(),
            initial_items.len() - 1,
            "State list should have length n-1 after pop via exec_stmt"
        );

        for i in 0..updated_items.len() {
            prop_assert!(
                values_equal(&updated_items[i], &initial_items[i]),
                "state element at index {} should match original prefix after exec_stmt pop.\nExpected: {:?}\nGot: {:?}",
                i,
                initial_items[i],
                updated_items[i]
            );
        }
    }
}

// =============================================================================
// Property 3: Index assignment replaces only the target position
// Feature: list-mutation-ops, Property 3: Index assignment replaces only the target position
// =============================================================================

/// Build an integer literal expression.
fn int_lit(v: i64) -> TypedExpr {
    texpr(TypedExprKind::IntLiteral(v), FluxType::Int)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 3.1**
    ///
    /// Property 3: Index assignment replaces only the target position
    ///
    /// For any non-empty list, valid index i (0 ≤ i < len), and any value,
    /// assigning list[i] = value SHALL produce a list where position i contains
    /// the new value and all other positions are unchanged.
    #[test]
    fn prop_index_assign_replaces_only_target(
        initial_items in arb_non_empty_list(),
        replacement_raw in any::<i64>(),
        index_selector in 0usize..100,
    ) {
        let len = initial_items.len();
        let idx = index_selector % len; // valid index in 0..len
        let replacement = Value::Int(replacement_raw);

        let mut interp = minimal_interpreter();
        let mut locals: HashMap<String, Value> = HashMap::new();

        // Store the original list in locals
        locals.insert("my_list".to_string(), Value::List(initial_items.clone()));

        // Build the assignment statement: my_list[idx] = replacement_raw
        let target_expr = texpr(
            TypedExprKind::IndexAccess {
                object: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                index: Box::new(int_lit(idx as i64)),
            },
            FluxType::Int,
        );

        let assignment = TypedStmt::Assignment(TypedAssignment {
            target: target_expr,
            value: int_lit(replacement_raw),
            span: Span::new(0, 0),
        });

        // Execute the assignment
        let result = interp.exec_stmt(&assignment, &mut locals);
        prop_assert!(result.is_ok(), "exec_stmt failed: {:?}", result.err());

        // Retrieve the modified list
        let modified_items = match locals.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            other => {
                prop_assert!(false, "Expected list in locals, got: {:?}", other);
                return Ok(());
            }
        };

        // Verify: length is unchanged
        prop_assert_eq!(
            modified_items.len(),
            len,
            "List length changed after index assignment: expected {}, got {}",
            len,
            modified_items.len()
        );

        // Verify: position idx contains the replacement value
        prop_assert!(
            values_equal(&modified_items[idx], &replacement),
            "Position {} should contain {:?} but got {:?}",
            idx,
            replacement,
            modified_items[idx]
        );

        // Verify: all other positions are unchanged
        for j in 0..len {
            if j != idx {
                prop_assert!(
                    values_equal(&modified_items[j], &initial_items[j]),
                    "Position {} was modified (got {:?}, expected {:?}) but only position {} should change",
                    j,
                    modified_items[j],
                    initial_items[j],
                    idx
                );
            }
        }
    }
}

// =============================================================================
// Property 4: Remove splices out at index preserving order
// Feature: list-mutation-ops, Property 4: Remove splices out at index preserving order
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// Property 4: Remove splices out at index preserving order
    ///
    /// For any non-empty list and valid index i (0 ≤ i < len), calling remove(i)
    /// SHALL return the element that was at position i, and the resulting list SHALL
    /// equal the original with that single element removed (elements after i shift
    /// left by one).
    #[test]
    fn prop_remove_splices_out_preserving_order(
        initial_items in arb_non_empty_list(),
        index_selector in 0usize..100,
    ) {
        let len = initial_items.len();
        let idx = index_selector % len; // valid index in 0..len

        let mut interp = minimal_interpreter();
        interp.state.insert("my_list".to_string(), Value::List(initial_items.clone()));

        // Build the remove MethodCall expression: my_list.remove(idx)
        let remove_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "remove".to_string(),
                args: vec![int_lit(idx as i64)],
            },
            FluxType::Int,
        );

        // Evaluate the remove call — should return the removed element
        let locals = HashMap::new();
        let result = interp.eval_expr(&remove_expr, &locals).unwrap();

        // Property: return value equals the element originally at position i
        prop_assert!(
            values_equal(&result, &initial_items[idx]),
            "remove({}) should return element at that position.\nExpected: {:?}\nGot: {:?}",
            idx,
            initial_items[idx],
            result
        );

        // Property: pending_list_mutation holds the modified list
        let modified = interp.pending_list_mutation.take();
        prop_assert!(
            modified.is_some(),
            "pending_list_mutation should hold the modified list after remove"
        );

        let modified_items = match modified.unwrap() {
            Value::List(items) => items,
            other => {
                prop_assert!(false, "pending_list_mutation should be a List, got {:?}", other);
                unreachable!()
            }
        };

        // Property: resulting list has length n-1
        prop_assert_eq!(
            modified_items.len(),
            len - 1,
            "remove should decrease list length by exactly 1"
        );

        // Property: elements before idx are unchanged
        for j in 0..idx {
            prop_assert!(
                values_equal(&modified_items[j], &initial_items[j]),
                "element at index {} (before remove pos) should be unchanged.\nExpected: {:?}\nGot: {:?}",
                j,
                initial_items[j],
                modified_items[j]
            );
        }

        // Property: elements after idx are shifted left by 1
        for j in idx..modified_items.len() {
            prop_assert!(
                values_equal(&modified_items[j], &initial_items[j + 1]),
                "element at index {} (after remove) should equal original index {}.\nExpected: {:?}\nGot: {:?}",
                j,
                j + 1,
                initial_items[j + 1],
                modified_items[j]
            );
        }
    }
}

// =============================================================================
// Property 5: Insert splices in at index preserving order
// Feature: list-mutation-ops, Property 5: Insert splices in at index preserving order
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.1, 5.2**
    ///
    /// Property 5: Insert splices in at index preserving order
    ///
    /// For any list and valid insertion index i (0 ≤ i ≤ len), and any valid element,
    /// calling insert(i, element) SHALL produce a list where position i contains the
    /// inserted element, elements originally at positions 0..i are unchanged, and
    /// elements originally at positions i..len are shifted right by one.
    #[test]
    fn prop_insert_splices_in_at_index_preserving_order(
        initial_items in arb_list(),
        new_element in arb_element(),
        index_selector in 0usize..101,
    ) {
        let len = initial_items.len();
        // Valid insertion index: 0..=len
        let insert_idx = if len == 0 { 0 } else { index_selector % (len + 1) };

        let mut interp = minimal_interpreter();
        interp.state.insert("my_list".to_string(), Value::List(initial_items.clone()));

        // Build the insert MethodCall expression: my_list.insert(idx, element)
        let element_expr = match &new_element {
            Value::Int(i) => texpr(TypedExprKind::IntLiteral(*i), FluxType::Int),
            Value::Float(f) => texpr(TypedExprKind::FloatLiteral(*f), FluxType::Float),
            _ => unreachable!("arb_element only generates Int or Float"),
        };

        let insert_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "insert".to_string(),
                args: vec![int_lit(insert_idx as i64), element_expr],
            },
            FluxType::Void,
        );

        // Execute via exec_stmt to trigger implicit reassignment
        let stmt = expr_stmt(insert_expr);
        let mut locals = HashMap::new();
        let exec_result = interp.exec_stmt(&stmt, &mut locals);
        prop_assert!(exec_result.is_ok(), "exec_stmt for insert failed: {:?}", exec_result.err());

        // Retrieve the modified list from state (written back by implicit reassignment)
        let result_items = match interp.state.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            other => {
                prop_assert!(
                    false,
                    "Expected List in state 'my_list', got {:?}\ninitial: {:?}\ninsert_idx: {}\nnew_element: {:?}",
                    other, initial_items, insert_idx, new_element
                );
                unreachable!()
            }
        };

        // Check 1: Length is n+1
        prop_assert_eq!(
            result_items.len(),
            len + 1,
            "After insert, list length should be {} but got {}.\ninitial: {:?}\ninsert_idx: {}\nnew_element: {:?}",
            len + 1,
            result_items.len(),
            initial_items,
            insert_idx,
            new_element,
        );

        // Check 2: Element at position insert_idx is the inserted element
        prop_assert!(
            values_equal(&result_items[insert_idx], &new_element),
            "Element at position {} should be {:?} but got {:?}.\ninitial: {:?}\ninsert_idx: {}\nnew_element: {:?}",
            insert_idx,
            new_element,
            result_items[insert_idx],
            initial_items,
            insert_idx,
            new_element,
        );

        // Check 3: Elements before insert_idx are unchanged
        for pos in 0..insert_idx {
            prop_assert!(
                values_equal(&result_items[pos], &initial_items[pos]),
                "Element at position {} (before insert) should be {:?} but got {:?}.\ninitial: {:?}\ninsert_idx: {}",
                pos,
                initial_items[pos],
                result_items[pos],
                initial_items,
                insert_idx,
            );
        }

        // Check 4: Elements at insert_idx..len are shifted right by 1
        for pos in insert_idx..len {
            let shifted_pos = pos + 1;
            prop_assert!(
                values_equal(&result_items[shifted_pos], &initial_items[pos]),
                "Element at position {} (shifted from {}) should be {:?} but got {:?}.\ninitial: {:?}\ninsert_idx: {}",
                shifted_pos,
                pos,
                initial_items[pos],
                result_items[shifted_pos],
                initial_items,
                insert_idx,
            );
        }
    }
}

// =============================================================================
// Property 8: Push-pop round trip
// Feature: list-mutation-ops, Property 8: Push-pop round trip
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 1.1, 2.1**
    ///
    /// Property 8: Push-pop round trip
    ///
    /// For any list and any element, pushing an element and then immediately popping
    /// SHALL return the pushed element and restore the original list.
    #[test]
    fn prop_push_pop_round_trip(
        initial_items in arb_list(),
        new_element in arb_element(),
    ) {
        // Step 1: Start with a list of random elements
        let mut interp = minimal_interpreter();
        let locals: HashMap<String, Value> = {
            let mut m = HashMap::new();
            m.insert("__list".to_string(), Value::List(initial_items.clone()));
            m
        };

        // Build the push MethodCall expression: __list.push(new_element)
        let element_expr = match &new_element {
            Value::Int(i) => texpr(TypedExprKind::IntLiteral(*i), FluxType::Int),
            Value::Float(f) => texpr(TypedExprKind::FloatLiteral(*f), FluxType::Float),
            _ => unreachable!("arb_element only generates Int or Float"),
        };

        let push_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "__list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "push".to_string(),
                args: vec![element_expr],
            },
            FluxType::List(Box::new(FluxType::Int)),
        );

        // Step 2: Call push(element) — get the new list back
        let push_result = interp.eval_expr(&push_expr, &locals).unwrap();
        let pushed_list = match &push_result {
            Value::List(items) => items.clone(),
            other => panic!("push should return Value::List, got {:?}", other),
        };

        // Step 3: Call pop() on the new list — should return the element and restore original
        let mut interp2 = minimal_interpreter();
        interp2.state.insert("__list".to_string(), Value::List(pushed_list));

        let pop_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "__list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "pop".to_string(),
                args: vec![],
            },
            FluxType::Int,
        );

        let locals2 = HashMap::new();
        let pop_result = interp2.eval_expr(&pop_expr, &locals2).unwrap();

        // Step 4: Verify: pop return value == pushed element
        prop_assert!(
            values_equal(&pop_result, &new_element),
            "pop() after push should return the pushed element.\nPushed: {:?}\nPopped: {:?}",
            new_element,
            pop_result
        );

        // Step 5: Verify: resulting list == original list
        let restored = interp2.pending_list_mutation.take();
        prop_assert!(
            restored.is_some(),
            "pending_list_mutation should hold the restored list after pop"
        );

        let restored_items = match restored.unwrap() {
            Value::List(items) => items,
            other => {
                prop_assert!(false, "pending_list_mutation should be a List, got {:?}", other);
                unreachable!()
            }
        };

        // The restored list should have the same length as the original
        prop_assert_eq!(
            restored_items.len(),
            initial_items.len(),
            "After push then pop, list length should be restored to original.\nOriginal len: {}\nRestored len: {}",
            initial_items.len(),
            restored_items.len()
        );

        // The restored list should equal the original element-by-element
        for i in 0..initial_items.len() {
            prop_assert!(
                values_equal(&restored_items[i], &initial_items[i]),
                "After push then pop, element at index {} should match original.\nExpected: {:?}\nGot: {:?}",
                i,
                initial_items[i],
                restored_items[i]
            );
        }
    }
}

// =============================================================================
// Property 6: Sort_by produces a sorted permutation
// Feature: list-mutation-ops, Property 6: Sort_by produces a sorted permutation
// =============================================================================

/// Generate a random struct Value with a "price" field (finite f64).
fn arb_struct_with_price() -> impl Strategy<Value = Value> {
    (-1e6f64..1e6f64)
        .prop_filter("must be finite", |f| f.is_finite())
        .prop_map(|price| {
            let mut fields = HashMap::new();
            fields.insert("price".to_string(), Value::Float(price));
            Value::Struct {
                type_name: "Item".to_string(),
                fields,
            }
        })
}

/// Generate a random list of struct Values with a "price" field (0..20 items).
fn arb_struct_list() -> impl Strategy<Value = Vec<Value>> {
    prop::collection::vec(arb_struct_with_price(), 0..20)
}

/// Extract the price field from a Value::Struct for comparison.
fn get_price(val: &Value) -> f64 {
    match val {
        Value::Struct { fields, .. } => match fields.get("price") {
            Some(Value::Float(f)) => *f,
            _ => panic!("expected Float price field"),
        },
        _ => panic!("expected Value::Struct"),
    }
}

/// Check if two struct values are equal (same type_name and same fields).
fn structs_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (
            Value::Struct { type_name: tn_a, fields: f_a },
            Value::Struct { type_name: tn_b, fields: f_b },
        ) => {
            if tn_a != tn_b || f_a.len() != f_b.len() {
                return false;
            }
            for (key, val_a) in f_a {
                match f_b.get(key) {
                    Some(val_b) => {
                        if !values_equal(val_a, val_b) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        _ => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 6.1, 6.2**
    ///
    /// Property 6: Sort_by produces a sorted permutation
    ///
    /// For any list of structs with a numeric field `f`, calling `sort_by("f")` SHALL
    /// produce a list that is (a) sorted in ascending order by field `f`, and (b) a
    /// permutation of the original list (same elements, same multiset).
    #[test]
    fn prop_sort_by_produces_sorted_permutation(
        initial_items in arb_struct_list(),
    ) {
        let mut interp = minimal_interpreter();
        interp.state.insert("my_list".to_string(), Value::List(initial_items.clone()));

        // Build the sort_by MethodCall expression: my_list.sort_by("price")
        let sort_by_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Struct("Item".to_string()))),
                )),
                method: "sort_by".to_string(),
                args: vec![texpr(
                    TypedExprKind::StringLiteral("price".to_string()),
                    FluxType::String,
                )],
            },
            FluxType::Void,
        );

        // Evaluate the sort_by call — should return Value::Null
        let locals = HashMap::new();
        let result = interp.eval_expr(&sort_by_expr, &locals).unwrap();
        prop_assert!(
            matches!(result, Value::Null),
            "sort_by should return Value::Null, got {:?}",
            result
        );

        // The sorted list is stored in pending_list_mutation
        let sorted_val = interp.pending_list_mutation.take();

        if initial_items.is_empty() {
            // Empty list: pending_list_mutation should still contain an empty list
            let sorted_items = match sorted_val {
                Some(Value::List(items)) => items,
                _ => {
                    // An empty list sort might still set pending_list_mutation
                    prop_assert!(sorted_val.is_some(), "pending_list_mutation should be set even for empty list");
                    unreachable!()
                }
            };
            prop_assert_eq!(sorted_items.len(), 0, "sorted empty list should be empty");
            return Ok(());
        }

        prop_assert!(
            sorted_val.is_some(),
            "pending_list_mutation should hold the sorted list"
        );

        let sorted_items = match sorted_val.unwrap() {
            Value::List(items) => items,
            other => {
                prop_assert!(false, "pending_list_mutation should be a List, got {:?}", other);
                unreachable!()
            }
        };

        let original_len = initial_items.len();

        // Property (a): Same length
        prop_assert_eq!(
            sorted_items.len(),
            original_len,
            "sort_by should not change list length"
        );

        // Property (a): Sorted in ascending order by "price" field
        for i in 1..sorted_items.len() {
            let prev_price = get_price(&sorted_items[i - 1]);
            let curr_price = get_price(&sorted_items[i]);
            prop_assert!(
                prev_price <= curr_price,
                "List should be sorted ascending by price. At index {}: {} > {}",
                i,
                prev_price,
                curr_price
            );
        }

        // Property (b): Permutation — same multiset of elements
        // For each element in the original, there should be a matching element in sorted
        // (and vice versa). We use a greedy matching approach.
        let mut matched = vec![false; sorted_items.len()];
        for orig_item in &initial_items {
            let mut found = false;
            for (j, sorted_item) in sorted_items.iter().enumerate() {
                if !matched[j] && structs_equal(orig_item, sorted_item) {
                    matched[j] = true;
                    found = true;
                    break;
                }
            }
            prop_assert!(
                found,
                "Original element {:?} not found in sorted result (permutation violated)",
                orig_item
            );
        }

        // All sorted items should be matched
        for (j, m) in matched.iter().enumerate() {
            prop_assert!(
                *m,
                "Sorted element at index {} has no corresponding original element (permutation violated)",
                j
            );
        }
    }
}

// =============================================================================
// Property 7: Out-of-bounds operations produce errors
// Feature: list-mutation-ops, Property 7: Out-of-bounds operations produce errors
// =============================================================================

/// Generate an out-of-bounds index for a list of the given length.
/// For index assignment and remove, valid range is 0..len, so OOB is i < 0 or i >= len.
/// For insert, valid range is 0..=len, so OOB is i < 0 or i > len.
fn arb_oob_index_for_assign_remove(len: usize) -> impl Strategy<Value = i64> {
    // Either negative or >= len
    prop_oneof![
        // Negative indices: -1 to -100
        (-100i64..=-1i64),
        // Too large: len..len+100
        (len as i64..=(len as i64 + 100)),
    ]
}

fn arb_oob_index_for_insert(len: usize) -> impl Strategy<Value = i64> {
    // Either negative or > len
    prop_oneof![
        // Negative indices: -1 to -100
        (-100i64..=-1i64),
        // Too large: len+1..len+101
        ((len as i64 + 1)..=(len as i64 + 101)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 3.3, 4.3, 5.3**
    ///
    /// Property 7: Out-of-bounds operations produce errors
    ///
    /// For any list and any index outside the valid range, index assignment
    /// (i < 0 or i ≥ len), remove (i < 0 or i ≥ len), and insert (i < 0 or i > len)
    /// SHALL return a runtime error. The list SHALL remain unmodified.
    #[test]
    fn prop_out_of_bounds_index_assign_errors(
        initial_items in arb_list(),
        replacement_val in any::<i64>(),
        negative_idx in -100i64..=-1i64,
    ) {
        let len = initial_items.len();
        let mut interp = minimal_interpreter();
        let mut locals: HashMap<String, Value> = HashMap::new();
        locals.insert("my_list".to_string(), Value::List(initial_items.clone()));

        // Test 1: negative index
        let target_expr = texpr(
            TypedExprKind::IndexAccess {
                object: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                index: Box::new(int_lit(negative_idx)),
            },
            FluxType::Int,
        );

        let assignment = TypedStmt::Assignment(TypedAssignment {
            target: target_expr,
            value: int_lit(replacement_val),
            span: Span::new(0, 0),
        });

        let result = interp.exec_stmt(&assignment, &mut locals);
        prop_assert!(result.is_err(), "Index assignment with negative index {} should error", negative_idx);
        let err_msg = result.unwrap_err();
        prop_assert!(
            err_msg.contains("runtime error: index"),
            "Error should contain 'runtime error: index', got: {}",
            err_msg
        );

        // Verify list is unchanged
        let after_items = match locals.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            _ => panic!("my_list should still exist in locals"),
        };
        prop_assert_eq!(after_items.len(), len, "List should remain unmodified after OOB index assign");
        for i in 0..len {
            prop_assert!(
                values_equal(&after_items[i], &initial_items[i]),
                "Element at {} should be unchanged after failed index assign",
                i
            );
        }

        // Test 2: too-large index (len or greater)
        let too_large_idx = len as i64; // i >= len is OOB for assign
        let target_expr2 = texpr(
            TypedExprKind::IndexAccess {
                object: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                index: Box::new(int_lit(too_large_idx)),
            },
            FluxType::Int,
        );

        let assignment2 = TypedStmt::Assignment(TypedAssignment {
            target: target_expr2,
            value: int_lit(replacement_val),
            span: Span::new(0, 0),
        });

        let result2 = interp.exec_stmt(&assignment2, &mut locals);
        prop_assert!(result2.is_err(), "Index assignment with index {} (>= len {}) should error", too_large_idx, len);
        let err_msg2 = result2.unwrap_err();
        prop_assert!(
            err_msg2.contains("runtime error: index"),
            "Error should contain 'runtime error: index', got: {}",
            err_msg2
        );

        // Verify list is still unchanged
        let after_items2 = match locals.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            _ => panic!("my_list should still exist in locals"),
        };
        prop_assert_eq!(after_items2.len(), len, "List should remain unmodified after OOB index assign (too large)");
        for i in 0..len {
            prop_assert!(
                values_equal(&after_items2[i], &initial_items[i]),
                "Element at {} should be unchanged after failed index assign (too large)",
                i
            );
        }
    }

    #[test]
    fn prop_out_of_bounds_remove_errors(
        initial_items in arb_list(),
        negative_idx in -100i64..=-1i64,
    ) {
        let len = initial_items.len();
        let mut interp = minimal_interpreter();
        interp.state.insert("my_list".to_string(), Value::List(initial_items.clone()));

        // Test 1: negative index
        let remove_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "remove".to_string(),
                args: vec![int_lit(negative_idx)],
            },
            FluxType::Int,
        );

        let locals = HashMap::new();
        let result = interp.eval_expr(&remove_expr, &locals);
        prop_assert!(result.is_err(), "remove({}) should error for negative index", negative_idx);
        let err_msg = result.unwrap_err();
        prop_assert!(
            err_msg.contains("runtime error: index"),
            "Error should contain 'runtime error: index', got: {}",
            err_msg
        );

        // Verify list is unchanged in state
        let after_items = match interp.state.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            _ => panic!("my_list should still exist in state"),
        };
        prop_assert_eq!(after_items.len(), len, "List should remain unmodified after OOB remove (negative)");
        for i in 0..len {
            prop_assert!(
                values_equal(&after_items[i], &initial_items[i]),
                "Element at {} should be unchanged after failed remove (negative)",
                i
            );
        }

        // Test 2: too-large index (len or greater)
        let too_large_idx = len as i64; // i >= len is OOB for remove
        let mut interp2 = minimal_interpreter();
        interp2.state.insert("my_list".to_string(), Value::List(initial_items.clone()));

        let remove_expr2 = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "remove".to_string(),
                args: vec![int_lit(too_large_idx)],
            },
            FluxType::Int,
        );

        let result2 = interp2.eval_expr(&remove_expr2, &locals);
        prop_assert!(result2.is_err(), "remove({}) should error for index >= len {}", too_large_idx, len);
        let err_msg2 = result2.unwrap_err();
        prop_assert!(
            err_msg2.contains("runtime error: index"),
            "Error should contain 'runtime error: index', got: {}",
            err_msg2
        );

        // Verify list is unchanged in state
        let after_items2 = match interp2.state.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            _ => panic!("my_list should still exist in state"),
        };
        prop_assert_eq!(after_items2.len(), len, "List should remain unmodified after OOB remove (too large)");
        for i in 0..len {
            prop_assert!(
                values_equal(&after_items2[i], &initial_items[i]),
                "Element at {} should be unchanged after failed remove (too large)",
                i
            );
        }
    }

    #[test]
    fn prop_out_of_bounds_insert_errors(
        initial_items in arb_list(),
        new_element in arb_element(),
        negative_idx in -100i64..=-1i64,
    ) {
        let len = initial_items.len();
        let mut interp = minimal_interpreter();
        interp.state.insert("my_list".to_string(), Value::List(initial_items.clone()));

        let element_expr = match &new_element {
            Value::Int(i) => texpr(TypedExprKind::IntLiteral(*i), FluxType::Int),
            Value::Float(f) => texpr(TypedExprKind::FloatLiteral(*f), FluxType::Float),
            _ => unreachable!("arb_element only generates Int or Float"),
        };

        // Test 1: negative index
        let insert_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "insert".to_string(),
                args: vec![int_lit(negative_idx), element_expr.clone()],
            },
            FluxType::Void,
        );

        let locals = HashMap::new();
        let result = interp.eval_expr(&insert_expr, &locals);
        prop_assert!(result.is_err(), "insert({}, ..) should error for negative index", negative_idx);
        let err_msg = result.unwrap_err();
        prop_assert!(
            err_msg.contains("runtime error: index"),
            "Error should contain 'runtime error: index', got: {}",
            err_msg
        );

        // Verify list is unchanged in state
        let after_items = match interp.state.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            _ => panic!("my_list should still exist in state"),
        };
        prop_assert_eq!(after_items.len(), len, "List should remain unmodified after OOB insert (negative)");
        for i in 0..len {
            prop_assert!(
                values_equal(&after_items[i], &initial_items[i]),
                "Element at {} should be unchanged after failed insert (negative)",
                i
            );
        }

        // Test 2: too-large index (> len is OOB for insert — len+1 is the first invalid)
        let too_large_idx = len as i64 + 1;
        let mut interp2 = minimal_interpreter();
        interp2.state.insert("my_list".to_string(), Value::List(initial_items.clone()));

        let element_expr2 = match &new_element {
            Value::Int(i) => texpr(TypedExprKind::IntLiteral(*i), FluxType::Int),
            Value::Float(f) => texpr(TypedExprKind::FloatLiteral(*f), FluxType::Float),
            _ => unreachable!("arb_element only generates Int or Float"),
        };

        let insert_expr2 = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(ident_expr_typed(
                    "my_list",
                    FluxType::List(Box::new(FluxType::Int)),
                )),
                method: "insert".to_string(),
                args: vec![int_lit(too_large_idx), element_expr2],
            },
            FluxType::Void,
        );

        let result2 = interp2.eval_expr(&insert_expr2, &locals);
        prop_assert!(result2.is_err(), "insert({}, ..) should error for index > len {}", too_large_idx, len);
        let err_msg2 = result2.unwrap_err();
        prop_assert!(
            err_msg2.contains("runtime error: index"),
            "Error should contain 'runtime error: index', got: {}",
            err_msg2
        );

        // Verify list is unchanged in state
        let after_items2 = match interp2.state.get("my_list") {
            Some(Value::List(items)) => items.clone(),
            _ => panic!("my_list should still exist in state"),
        };
        prop_assert_eq!(after_items2.len(), len, "List should remain unmodified after OOB insert (too large)");
        for i in 0..len {
            prop_assert!(
                values_equal(&after_items2[i], &initial_items[i]),
                "Element at {} should be unchanged after failed insert (too large)",
                i
            );
        }
    }
}
