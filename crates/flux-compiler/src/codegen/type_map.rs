//! FluxType → Rust type string mapping for code generation.

use crate::error::{CompileError, Result};
use crate::typeck::types::FluxType;

/// Map a FluxType to its Rust type string representation.
///
/// # Arguments
/// * `ty` - The Flux type to map
/// * `span_start` - Byte offset for error reporting
///
/// # Returns
/// The Rust type string (e.g., "i64", "f64", "Vec<String>")
///
/// # Errors
/// Returns `CompileError::Codegen` for `FluxType::Fn` (cannot be emitted as a Rust type)
pub fn map_type(ty: &FluxType, span_start: usize) -> Result<String> {
    match ty {
        FluxType::Int => Ok("i64".to_string()),
        FluxType::Float => Ok("f64".to_string()),
        FluxType::String => Ok("String".to_string()),
        FluxType::Bool => Ok("bool".to_string()),
        FluxType::List(inner) => {
            let inner_rust = map_type(inner, span_start)?;
            Ok(format!("Vec<{}>", inner_rust))
        }
        FluxType::Signal => Ok("Signal".to_string()),
        FluxType::Null => Ok("()".to_string()),
        FluxType::Void => Ok("()".to_string()),
        FluxType::VecFloat => Ok("Vec<f64>".to_string()),
        FluxType::MatFloat => Ok("Vec<f64>".to_string()), // row-major flat storage
        FluxType::Fn { .. } => Err(CompileError::Codegen(format!(
            "at byte {}: function types cannot be emitted as Rust types",
            span_start
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typeck::types::FnParams;

    #[test]
    fn map_int() {
        assert_eq!(map_type(&FluxType::Int, 0).unwrap(), "i64");
    }

    #[test]
    fn map_float() {
        assert_eq!(map_type(&FluxType::Float, 0).unwrap(), "f64");
    }

    #[test]
    fn map_string() {
        assert_eq!(map_type(&FluxType::String, 0).unwrap(), "String");
    }

    #[test]
    fn map_bool() {
        assert_eq!(map_type(&FluxType::Bool, 0).unwrap(), "bool");
    }

    #[test]
    fn map_list_int() {
        let ty = FluxType::List(Box::new(FluxType::Int));
        assert_eq!(map_type(&ty, 0).unwrap(), "Vec<i64>");
    }

    #[test]
    fn map_nested_list() {
        let ty = FluxType::List(Box::new(FluxType::List(Box::new(FluxType::Float))));
        assert_eq!(map_type(&ty, 0).unwrap(), "Vec<Vec<f64>>");
    }

    #[test]
    fn map_signal() {
        assert_eq!(map_type(&FluxType::Signal, 0).unwrap(), "Signal");
    }

    #[test]
    fn map_null() {
        assert_eq!(map_type(&FluxType::Null, 0).unwrap(), "()");
    }

    #[test]
    fn map_void() {
        assert_eq!(map_type(&FluxType::Void, 0).unwrap(), "()");
    }

    #[test]
    fn map_fn_returns_error() {
        let ty = FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::Int]),
            ret: Box::new(FluxType::Float),
        };
        let result = map_type(&ty, 42);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("at byte 42"),
            "Error should contain byte offset, got: {}",
            msg
        );
        assert!(
            msg.contains("function types cannot be emitted as Rust types"),
            "Error should describe the issue, got: {}",
            msg
        );
    }

    #[test]
    fn map_fn_error_with_different_offset() {
        let ty = FluxType::Fn {
            params: FnParams::VariadicNumeric,
            ret: Box::new(FluxType::Void),
        };
        let result = map_type(&ty, 100);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("at byte 100"));
    }

    #[test]
    fn map_nested_list_int() {
        let ty = FluxType::List(Box::new(FluxType::List(Box::new(FluxType::Int))));
        assert_eq!(map_type(&ty, 0).unwrap(), "Vec<Vec<i64>>");
    }

    #[test]
    fn map_triple_nested_list_string() {
        let ty = FluxType::List(Box::new(FluxType::List(Box::new(FluxType::List(
            Box::new(FluxType::String),
        )))));
        assert_eq!(map_type(&ty, 0).unwrap(), "Vec<Vec<Vec<String>>>");
    }

    #[test]
    fn map_fn_returns_codegen_error_variant() {
        let ty = FluxType::Fn {
            params: FnParams::Fixed(vec![FluxType::Bool]),
            ret: Box::new(FluxType::Int),
        };
        let result = map_type(&ty, 7);
        match result {
            Err(CompileError::Codegen(msg)) => {
                assert!(
                    msg.contains("at byte 7:"),
                    "Expected byte offset in message, got: {}",
                    msg
                );
                assert!(
                    msg.contains("function types cannot be emitted as Rust types"),
                    "Expected function type error description, got: {}",
                    msg
                );
            }
            Err(other) => panic!("Expected CompileError::Codegen, got: {:?}", other),
            Ok(val) => panic!("Expected error, got Ok({})", val),
        }
    }
}
