#![allow(dead_code)]
//! Flux type system definitions and type compatibility utilities.

use std::fmt;

/// The set of types in the Flux type system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FluxType {
    Int,
    Float,
    String,
    Bool,
    Null,
    Void,
    Signal,
    List(Box<FluxType>),
    /// A one-dimensional vector of Float values (e.g., weights, returns).
    VecFloat,
    /// A two-dimensional matrix of Float values (e.g., covariance matrix).
    MatFloat,
    /// Function type: parameter specification and return type.
    /// Used internally for imported functions and built-in signals.
    Fn { params: FnParams, ret: Box<FluxType> },
    /// A named struct type (e.g., `Quote`, `Tick`). Two struct types are
    /// considered the same type iff their names match exactly.
    Struct(String),
    /// A fixed-size array type `[T; N]`. Two fixed-array types are the same
    /// type iff their element types match and their sizes match exactly.
    FixedArray(Box<FluxType>, usize),
    /// An enum type, identified by name (Phase 1).
    Enum(String),
    /// A generic type parameter (unresolved during definition, e.g. `T`).
    TypeParam(String),
    /// A generic type: name + resolved type arguments (Phase 4).
    /// e.g., HashMap[String, Float] → Generic("HashMap", [String, Float])
    Generic(String, Vec<FluxType>),
}

/// Function parameter specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnParams {
    /// Fixed parameter types.
    Fixed(Vec<FluxType>),
    /// Variable number of numeric arguments (for imported indicator functions).
    VariadicNumeric,
    /// Overloaded: multiple valid signatures (e.g., CLOSE with 1 or 2 args).
    Overloaded(Vec<Vec<FluxType>>),
}

impl FluxType {
    /// Returns true if this type is numeric (Int or Float).
    /// Null and TypeParam are included because they represent untyped/unresolved types (gradual typing).
    pub fn is_numeric(&self) -> bool {
        matches!(self, FluxType::Int | FluxType::Float | FluxType::Null | FluxType::TypeParam(_))
    }

    /// Returns true if `self` is assignable to `target` (with coercion).
    /// Int is assignable to Float. List(Null) is assignable to any List(T).
    /// TypeParam matches any concrete type (for generic contexts).
    /// Null is assignable to any type (gradual typing for untyped list elements).
    pub fn is_assignable_to(&self, target: &FluxType) -> bool {
        if self == target {
            return true;
        }
        match (self, target) {
            // TypeParam is assignable to any type (unresolved generic placeholder)
            (FluxType::TypeParam(_), _) | (_, FluxType::TypeParam(_)) => true,
            // Null is assignable to any type (gradual typing for untyped list element access)
            (FluxType::Null, _) | (_, FluxType::Null) => true,
            (FluxType::Int, FluxType::Float) => true,
            (FluxType::List(inner), FluxType::List(_)) if inner.as_ref() == &FluxType::Null => {
                true
            }
            // Any List(T) is assignable to List(Null) — untyped list accepts any element type
            (FluxType::List(_), FluxType::List(target_inner)) if target_inner.as_ref() == &FluxType::Null => {
                true
            }
            (FluxType::Struct(a), FluxType::Struct(b)) => a == b,
            (FluxType::FixedArray(a_elem, a_size), FluxType::FixedArray(b_elem, b_size)) => {
                a_size == b_size && a_elem.is_assignable_to(b_elem)
            }
            // Generic types are assignable if names match and all args are assignable
            (FluxType::Generic(a_name, a_args), FluxType::Generic(b_name, b_args)) => {
                a_name == b_name
                    && a_args.len() == b_args.len()
                    && a_args.iter().zip(b_args.iter()).all(|(a, b)| a.is_assignable_to(b))
            }
            _ => false,
        }
    }

    /// Compute the result type of a binary arithmetic operation.
    /// Returns None if the operand types are incompatible.
    pub fn arithmetic_result(left: &FluxType, right: &FluxType) -> Option<FluxType> {
        match (left, right) {
            (FluxType::Int, FluxType::Int) => Some(FluxType::Int),
            (FluxType::Float, FluxType::Float) => Some(FluxType::Float),
            (FluxType::Int, FluxType::Float) | (FluxType::Float, FluxType::Int) => {
                Some(FluxType::Float)
            }
            // Gradual typing: Null (from untyped list access) is compatible with arithmetic
            (FluxType::Null, FluxType::Float) | (FluxType::Float, FluxType::Null) => {
                Some(FluxType::Float)
            }
            (FluxType::Null, FluxType::Int) | (FluxType::Int, FluxType::Null) => {
                Some(FluxType::Int)
            }
            (FluxType::Null, FluxType::Null) => Some(FluxType::Null),
            _ => None,
        }
    }
}

impl fmt::Display for FluxType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FluxType::Int => write!(f, "Int"),
            FluxType::Float => write!(f, "Float"),
            FluxType::String => write!(f, "String"),
            FluxType::Bool => write!(f, "Bool"),
            FluxType::Null => write!(f, "Null"),
            FluxType::Void => write!(f, "Void"),
            FluxType::Signal => write!(f, "Signal"),
            FluxType::List(t) => write!(f, "List({})", t),
            FluxType::VecFloat => write!(f, "VecFloat"),
            FluxType::MatFloat => write!(f, "MatFloat"),
            FluxType::Fn { ret, .. } => write!(f, "Fn -> {}", ret),
            FluxType::Struct(name) => write!(f, "{}", name),
            FluxType::FixedArray(elem, size) => write!(f, "[{}; {}]", elem, size),
            FluxType::Enum(name) => write!(f, "{}", name),
            FluxType::TypeParam(name) => write!(f, "{}", name),
            FluxType::Generic(name, args) => {
                write!(f, "{}[", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, "]")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_numeric() {
        assert!(FluxType::Int.is_numeric());
        assert!(FluxType::Float.is_numeric());
        // Null and TypeParam are considered numeric for gradual typing support
        assert!(FluxType::Null.is_numeric());
        assert!(FluxType::TypeParam("T".to_string()).is_numeric());

        assert!(!FluxType::String.is_numeric());
        assert!(!FluxType::Bool.is_numeric());
        assert!(!FluxType::Void.is_numeric());
        assert!(!FluxType::Signal.is_numeric());
        assert!(!FluxType::List(Box::new(FluxType::Int)).is_numeric());
        assert!(!FluxType::Fn {
            params: FnParams::Fixed(vec![]),
            ret: Box::new(FluxType::Void),
        }
        .is_numeric());
    }

    #[test]
    fn test_is_assignable_to_same_type() {
        assert!(FluxType::Int.is_assignable_to(&FluxType::Int));
        assert!(FluxType::Float.is_assignable_to(&FluxType::Float));
        assert!(FluxType::String.is_assignable_to(&FluxType::String));
        assert!(FluxType::Bool.is_assignable_to(&FluxType::Bool));
        assert!(FluxType::Null.is_assignable_to(&FluxType::Null));
        assert!(FluxType::Void.is_assignable_to(&FluxType::Void));
        assert!(FluxType::Signal.is_assignable_to(&FluxType::Signal));
        assert!(
            FluxType::List(Box::new(FluxType::Int))
                .is_assignable_to(&FluxType::List(Box::new(FluxType::Int)))
        );
    }

    #[test]
    fn test_int_assignable_to_float() {
        assert!(FluxType::Int.is_assignable_to(&FluxType::Float));
    }

    #[test]
    fn test_float_not_assignable_to_int() {
        assert!(!FluxType::Float.is_assignable_to(&FluxType::Int));
    }

    #[test]
    fn test_list_null_assignable_to_any_list() {
        let list_null = FluxType::List(Box::new(FluxType::Null));
        assert!(list_null.is_assignable_to(&FluxType::List(Box::new(FluxType::Int))));
        assert!(list_null.is_assignable_to(&FluxType::List(Box::new(FluxType::Float))));
        assert!(list_null.is_assignable_to(&FluxType::List(Box::new(FluxType::String))));
        assert!(list_null.is_assignable_to(&FluxType::List(Box::new(FluxType::Bool))));
        assert!(list_null.is_assignable_to(&FluxType::List(Box::new(FluxType::List(
            Box::new(FluxType::Int)
        )))));
    }

    #[test]
    fn test_list_int_not_assignable_to_list_string() {
        assert!(
            !FluxType::List(Box::new(FluxType::Int))
                .is_assignable_to(&FluxType::List(Box::new(FluxType::String)))
        );
    }

    #[test]
    fn test_arithmetic_result_int_int() {
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::Int, &FluxType::Int),
            Some(FluxType::Int)
        );
    }

    #[test]
    fn test_arithmetic_result_float_float() {
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::Float, &FluxType::Float),
            Some(FluxType::Float)
        );
    }

    #[test]
    fn test_arithmetic_result_mixed() {
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::Int, &FluxType::Float),
            Some(FluxType::Float)
        );
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::Float, &FluxType::Int),
            Some(FluxType::Float)
        );
    }

    #[test]
    fn test_arithmetic_result_non_numeric() {
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::String, &FluxType::Int),
            None
        );
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::Bool, &FluxType::Float),
            None
        );
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::String, &FluxType::String),
            None
        );
        // Null is compatible with numeric types for gradual typing
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::Null, &FluxType::Int),
            Some(FluxType::Int)
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(FluxType::Int.to_string(), "Int");
        assert_eq!(FluxType::Float.to_string(), "Float");
        assert_eq!(FluxType::String.to_string(), "String");
        assert_eq!(FluxType::Bool.to_string(), "Bool");
        assert_eq!(FluxType::Null.to_string(), "Null");
        assert_eq!(FluxType::Void.to_string(), "Void");
        assert_eq!(FluxType::Signal.to_string(), "Signal");
        assert_eq!(
            FluxType::List(Box::new(FluxType::Int)).to_string(),
            "List(Int)"
        );
        assert_eq!(FluxType::VecFloat.to_string(), "VecFloat");
        assert_eq!(FluxType::MatFloat.to_string(), "MatFloat");
        assert_eq!(
            FluxType::Fn {
                params: FnParams::Fixed(vec![FluxType::Int]),
                ret: Box::new(FluxType::Float),
            }
            .to_string(),
            "Fn -> Float"
        );
    }

    #[test]
    fn test_vecfloat_assignable_to_self() {
        assert!(FluxType::VecFloat.is_assignable_to(&FluxType::VecFloat));
    }

    #[test]
    fn test_matfloat_assignable_to_self() {
        assert!(FluxType::MatFloat.is_assignable_to(&FluxType::MatFloat));
    }

    #[test]
    fn test_vecfloat_not_assignable_to_other_types() {
        assert!(!FluxType::VecFloat.is_assignable_to(&FluxType::MatFloat));
        assert!(!FluxType::VecFloat.is_assignable_to(&FluxType::Float));
        assert!(!FluxType::VecFloat.is_assignable_to(&FluxType::Int));
        assert!(!FluxType::VecFloat.is_assignable_to(&FluxType::List(Box::new(FluxType::Float))));
    }

    #[test]
    fn test_matfloat_not_assignable_to_other_types() {
        assert!(!FluxType::MatFloat.is_assignable_to(&FluxType::VecFloat));
        assert!(!FluxType::MatFloat.is_assignable_to(&FluxType::Float));
        assert!(!FluxType::MatFloat.is_assignable_to(&FluxType::Int));
        assert!(!FluxType::MatFloat.is_assignable_to(&FluxType::List(Box::new(FluxType::Float))));
    }

    #[test]
    fn test_vecfloat_not_numeric() {
        assert!(!FluxType::VecFloat.is_numeric());
    }

    #[test]
    fn test_matfloat_not_numeric() {
        assert!(!FluxType::MatFloat.is_numeric());
    }

    #[test]
    fn test_struct_same_name_assignable() {
        let a = FluxType::Struct("Quote".to_string());
        let b = FluxType::Struct("Quote".to_string());
        assert!(a.is_assignable_to(&b));
    }

    #[test]
    fn test_struct_different_name_not_assignable() {
        let a = FluxType::Struct("Quote".to_string());
        let b = FluxType::Struct("Tick".to_string());
        assert!(!a.is_assignable_to(&b));
    }

    #[test]
    fn test_struct_not_assignable_to_non_struct() {
        let a = FluxType::Struct("Quote".to_string());
        assert!(!a.is_assignable_to(&FluxType::Int));
        assert!(!FluxType::Int.is_assignable_to(&a));
    }

    #[test]
    fn test_fixed_array_matching_elem_and_size_assignable() {
        let a = FluxType::FixedArray(Box::new(FluxType::Float), 20);
        let b = FluxType::FixedArray(Box::new(FluxType::Float), 20);
        assert!(a.is_assignable_to(&b));
    }

    #[test]
    fn test_fixed_array_mismatched_size_not_assignable() {
        let a = FluxType::FixedArray(Box::new(FluxType::Float), 20);
        let b = FluxType::FixedArray(Box::new(FluxType::Float), 10);
        assert!(!a.is_assignable_to(&b));
    }

    #[test]
    fn test_fixed_array_mismatched_elem_type_not_assignable() {
        let a = FluxType::FixedArray(Box::new(FluxType::Float), 20);
        let b = FluxType::FixedArray(Box::new(FluxType::Int), 20);
        assert!(!a.is_assignable_to(&b));
    }

    #[test]
    fn test_fixed_array_elem_coercion_int_to_float() {
        // Element assignability follows the same coercion rules (Int -> Float)
        let a = FluxType::FixedArray(Box::new(FluxType::Int), 5);
        let b = FluxType::FixedArray(Box::new(FluxType::Float), 5);
        assert!(a.is_assignable_to(&b));
    }

    #[test]
    fn test_fixed_array_of_struct_matching() {
        let a = FluxType::FixedArray(Box::new(FluxType::Struct("Level".to_string())), 20);
        let b = FluxType::FixedArray(Box::new(FluxType::Struct("Level".to_string())), 20);
        assert!(a.is_assignable_to(&b));
    }

    #[test]
    fn test_fixed_array_of_struct_mismatched_name() {
        let a = FluxType::FixedArray(Box::new(FluxType::Struct("Level".to_string())), 20);
        let b = FluxType::FixedArray(Box::new(FluxType::Struct("Quote".to_string())), 20);
        assert!(!a.is_assignable_to(&b));
    }

    #[test]
    fn test_struct_display() {
        assert_eq!(FluxType::Struct("Quote".to_string()).to_string(), "Quote");
    }

    #[test]
    fn test_fixed_array_display() {
        assert_eq!(
            FluxType::FixedArray(Box::new(FluxType::Float), 20).to_string(),
            "[Float; 20]"
        );
    }
}
