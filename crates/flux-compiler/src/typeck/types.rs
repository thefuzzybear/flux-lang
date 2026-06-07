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
    /// Function type: parameter specification and return type.
    /// Used internally for imported functions and built-in signals.
    Fn { params: FnParams, ret: Box<FluxType> },
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
    pub fn is_numeric(&self) -> bool {
        matches!(self, FluxType::Int | FluxType::Float)
    }

    /// Returns true if `self` is assignable to `target` (with coercion).
    /// Int is assignable to Float. List(Null) is assignable to any List(T).
    pub fn is_assignable_to(&self, target: &FluxType) -> bool {
        if self == target {
            return true;
        }
        match (self, target) {
            (FluxType::Int, FluxType::Float) => true,
            (FluxType::List(inner), FluxType::List(_)) if inner.as_ref() == &FluxType::Null => {
                true
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
            FluxType::Fn { ret, .. } => write!(f, "Fn -> {}", ret),
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

        assert!(!FluxType::String.is_numeric());
        assert!(!FluxType::Bool.is_numeric());
        assert!(!FluxType::Null.is_numeric());
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
        assert_eq!(
            FluxType::arithmetic_result(&FluxType::Null, &FluxType::Int),
            None
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
        assert_eq!(
            FluxType::Fn {
                params: FnParams::Fixed(vec![FluxType::Int]),
                ret: Box::new(FluxType::Float),
            }
            .to_string(),
            "Fn -> Float"
        );
    }
}
