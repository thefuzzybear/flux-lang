//! Tier 1 — Core Math Builtins (stateless pure functions).
//!
//! Provides `abs`, `sqrt`, `exp`, `log`, `floor`, `ceil`, `round`, `sign`,
//! `pow`, `min`, and `max` operating on `Value::Int` and `Value::Float`.

use crate::interpreter::Value;

/// Attempt to evaluate a Tier 1 math builtin by name.
///
/// Returns:
/// - `Ok(Some(value))` if `name` is a recognized math builtin and evaluation succeeds
/// - `Ok(None)` if `name` is not a math builtin (caller should try next dispatch tier)
/// - `Err(String)` on validation errors (wrong arg count, non-numeric argument)
pub fn eval_math_builtin(name: &str, args: &[Value]) -> Result<Option<Value>, String> {
    match name {
        "abs" => eval_abs(args),
        "sqrt" => eval_sqrt(args),
        "exp" => eval_exp(args),
        "log" => eval_log(args),
        "floor" => eval_floor(args),
        "ceil" => eval_ceil(args),
        "round" => eval_round(args),
        "sign" => eval_sign(args),
        "pow" => eval_pow(args),
        "min" => eval_min(args),
        "max" => eval_max(args),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Single-argument functions
// ---------------------------------------------------------------------------

/// abs: Int→Int, Float→Float
fn eval_abs(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("abs", args, 1)?;
    match &args[0] {
        Value::Int(i) => Ok(Some(Value::Int(i.wrapping_abs()))),
        Value::Float(f) => Ok(Some(Value::Float(f.abs()))),
        _ => Err("abs requires a numeric argument".to_string()),
    }
}

/// sqrt: returns NaN for negative inputs (not an error)
fn eval_sqrt(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("sqrt", args, 1)?;
    let x = to_f64("sqrt", &args[0])?;
    Ok(Some(Value::Float(x.sqrt())))
}

/// exp: e^x
fn eval_exp(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("exp", args, 1)?;
    let x = to_f64("exp", &args[0])?;
    Ok(Some(Value::Float(x.exp())))
}

/// log: natural logarithm; returns NaN for non-positive inputs
fn eval_log(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("log", args, 1)?;
    let x = to_f64("log", &args[0])?;
    Ok(Some(Value::Float(x.ln())))
}

/// floor: largest integer ≤ x, returned as Float
fn eval_floor(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("floor", args, 1)?;
    let x = to_f64("floor", &args[0])?;
    Ok(Some(Value::Float(x.floor())))
}

/// ceil: smallest integer ≥ x, returned as Float
fn eval_ceil(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("ceil", args, 1)?;
    let x = to_f64("ceil", &args[0])?;
    Ok(Some(Value::Float(x.ceil())))
}

/// round: nearest integer, half-away-from-zero
fn eval_round(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("round", args, 1)?;
    let x = to_f64("round", &args[0])?;
    // Rust's f64::round() uses half-away-from-zero semantics
    Ok(Some(Value::Float(x.round())))
}

/// sign: 1.0 if positive, -1.0 if negative, 0.0 if zero
fn eval_sign(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("sign", args, 1)?;
    let x = to_f64("sign", &args[0])?;
    let result = if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    };
    Ok(Some(Value::Float(result)))
}

// ---------------------------------------------------------------------------
// Multi-argument functions
// ---------------------------------------------------------------------------

/// pow(base, exp): base raised to the power of exp
fn eval_pow(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("pow", args, 2)?;
    let base = to_f64("pow", &args[0])?;
    let exp = to_f64("pow", &args[1])?;
    Ok(Some(Value::Float(base.powf(exp))))
}

/// min(a, b): returns the smaller of two values.
/// If one Int and one Float, promotes Int to Float and returns Float.
fn eval_min(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("min", args, 2)?;
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Some(Value::Int(*a.min(b)))),
        (Value::Int(a), Value::Float(b)) => Ok(Some(Value::Float((*a as f64).min(*b)))),
        (Value::Float(a), Value::Int(b)) => Ok(Some(Value::Float(a.min(*b as f64)))),
        (Value::Float(a), Value::Float(b)) => Ok(Some(Value::Float(a.min(*b)))),
        _ => Err("min requires a numeric argument".to_string()),
    }
}

/// max(a, b): returns the larger of two values.
/// If one Int and one Float, promotes Int to Float and returns Float.
fn eval_max(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("max", args, 2)?;
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Some(Value::Int(*a.max(b)))),
        (Value::Int(a), Value::Float(b)) => Ok(Some(Value::Float((*a as f64).max(*b)))),
        (Value::Float(a), Value::Int(b)) => Ok(Some(Value::Float(a.max(*b as f64)))),
        (Value::Float(a), Value::Float(b)) => Ok(Some(Value::Float(a.max(*b)))),
        _ => Err("max requires a numeric argument".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate argument count. Returns an error with the function name if wrong.
fn check_arity(name: &str, args: &[Value], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        Err(format!("{} requires {} argument(s)", name, expected))
    } else {
        Ok(())
    }
}

/// Coerce a Value to f64, returning an error if non-numeric.
fn to_f64(name: &str, val: &Value) -> Result<f64, String> {
    match val {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        _ => Err(format!("{} requires a numeric argument", name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- abs ---

    #[test]
    fn test_abs_positive_int() {
        let result = eval_math_builtin("abs", &[Value::Int(5)]).unwrap().unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_abs_negative_int() {
        let result = eval_math_builtin("abs", &[Value::Int(-7)]).unwrap().unwrap();
        assert!(matches!(result, Value::Int(7)));
    }

    #[test]
    fn test_abs_positive_float() {
        let result = eval_math_builtin("abs", &[Value::Float(3.14)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.14).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_abs_negative_float() {
        let result = eval_math_builtin("abs", &[Value::Float(-2.5)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 2.5).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_abs_preserves_int_type() {
        let result = eval_math_builtin("abs", &[Value::Int(-42)]).unwrap().unwrap();
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_abs_preserves_float_type() {
        let result = eval_math_builtin("abs", &[Value::Float(-1.5)]).unwrap().unwrap();
        assert!(matches!(result, Value::Float(_)));
    }

    // --- sqrt ---

    #[test]
    fn test_sqrt_positive() {
        let result = eval_math_builtin("sqrt", &[Value::Float(9.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_sqrt_negative_returns_nan() {
        let result = eval_math_builtin("sqrt", &[Value::Float(-1.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!(f.is_nan()),
            _ => panic!("expected Float(NaN)"),
        }
    }

    #[test]
    fn test_sqrt_zero() {
        let result = eval_math_builtin("sqrt", &[Value::Float(0.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 0.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_sqrt_int_promoted_to_float() {
        let result = eval_math_builtin("sqrt", &[Value::Int(16)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 4.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    // --- exp and log ---

    #[test]
    fn test_exp_zero() {
        let result = eval_math_builtin("exp", &[Value::Float(0.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 1.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_log_one() {
        let result = eval_math_builtin("log", &[Value::Float(1.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!(f.abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_log_zero_returns_nan() {
        let result = eval_math_builtin("log", &[Value::Float(0.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!(f.is_infinite() || f.is_nan()),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_log_negative_returns_nan() {
        let result = eval_math_builtin("log", &[Value::Float(-5.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!(f.is_nan()),
            _ => panic!("expected Float(NaN)"),
        }
    }

    #[test]
    fn test_exp_log_roundtrip() {
        let x = 2.5;
        let exp_result = eval_math_builtin("exp", &[Value::Float(x)]).unwrap().unwrap();
        if let Value::Float(e_x) = exp_result {
            let log_result = eval_math_builtin("log", &[Value::Float(e_x)]).unwrap().unwrap();
            if let Value::Float(result) = log_result {
                assert!((result - x).abs() < 1e-10);
            }
        }
    }

    // --- floor, ceil, round ---

    #[test]
    fn test_floor() {
        let result = eval_math_builtin("floor", &[Value::Float(2.7)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 2.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_floor_negative() {
        let result = eval_math_builtin("floor", &[Value::Float(-2.3)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - (-3.0)).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_ceil() {
        let result = eval_math_builtin("ceil", &[Value::Float(2.1)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_ceil_negative() {
        let result = eval_math_builtin("ceil", &[Value::Float(-2.7)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - (-2.0)).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_round_half_away_from_zero() {
        // 2.5 rounds to 3.0 (half away from zero)
        let result = eval_math_builtin("round", &[Value::Float(2.5)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }

        // -2.5 rounds to -3.0 (half away from zero)
        let result = eval_math_builtin("round", &[Value::Float(-2.5)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - (-3.0)).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    // --- sign ---

    #[test]
    fn test_sign_positive() {
        let result = eval_math_builtin("sign", &[Value::Float(42.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 1.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_sign_negative() {
        let result = eval_math_builtin("sign", &[Value::Float(-7.5)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - (-1.0)).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_sign_zero() {
        let result = eval_math_builtin("sign", &[Value::Float(0.0)]).unwrap().unwrap();
        match result {
            Value::Float(f) => assert!((f - 0.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    // --- pow ---

    #[test]
    fn test_pow_basic() {
        let result = eval_math_builtin("pow", &[Value::Float(2.0), Value::Float(3.0)])
            .unwrap()
            .unwrap();
        match result {
            Value::Float(f) => assert!((f - 8.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_pow_fractional_exponent() {
        let result = eval_math_builtin("pow", &[Value::Float(4.0), Value::Float(0.5)])
            .unwrap()
            .unwrap();
        match result {
            Value::Float(f) => assert!((f - 2.0).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    // --- min / max ---

    #[test]
    fn test_min_two_ints() {
        let result = eval_math_builtin("min", &[Value::Int(3), Value::Int(7)])
            .unwrap()
            .unwrap();
        assert!(matches!(result, Value::Int(3)));
    }

    #[test]
    fn test_min_two_floats() {
        let result = eval_math_builtin("min", &[Value::Float(3.5), Value::Float(2.1)])
            .unwrap()
            .unwrap();
        match result {
            Value::Float(f) => assert!((f - 2.1).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_min_int_float_promotion() {
        let result = eval_math_builtin("min", &[Value::Int(5), Value::Float(3.2)])
            .unwrap()
            .unwrap();
        match result {
            Value::Float(f) => assert!((f - 3.2).abs() < 1e-10),
            _ => panic!("expected Float from Int/Float promotion"),
        }
    }

    #[test]
    fn test_max_two_ints() {
        let result = eval_math_builtin("max", &[Value::Int(3), Value::Int(7)])
            .unwrap()
            .unwrap();
        assert!(matches!(result, Value::Int(7)));
    }

    #[test]
    fn test_max_int_float_promotion() {
        let result = eval_math_builtin("max", &[Value::Int(2), Value::Float(5.5)])
            .unwrap()
            .unwrap();
        match result {
            Value::Float(f) => assert!((f - 5.5).abs() < 1e-10),
            _ => panic!("expected Float from Int/Float promotion"),
        }
    }

    // --- Argument count validation ---

    #[test]
    fn test_wrong_arg_count_abs() {
        let err = eval_math_builtin("abs", &[]).unwrap_err();
        assert_eq!(err, "abs requires 1 argument(s)");
    }

    #[test]
    fn test_wrong_arg_count_abs_too_many() {
        let err = eval_math_builtin("abs", &[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert_eq!(err, "abs requires 1 argument(s)");
    }

    #[test]
    fn test_wrong_arg_count_pow() {
        let err = eval_math_builtin("pow", &[Value::Float(2.0)]).unwrap_err();
        assert_eq!(err, "pow requires 2 argument(s)");
    }

    #[test]
    fn test_wrong_arg_count_min() {
        let err = eval_math_builtin("min", &[Value::Int(1)]).unwrap_err();
        assert_eq!(err, "min requires 2 argument(s)");
    }

    // --- Non-numeric argument errors ---

    #[test]
    fn test_non_numeric_abs() {
        let err = eval_math_builtin("abs", &[Value::Str("hello".to_string())]).unwrap_err();
        assert_eq!(err, "abs requires a numeric argument");
    }

    #[test]
    fn test_non_numeric_sqrt() {
        let err = eval_math_builtin("sqrt", &[Value::Bool(true)]).unwrap_err();
        assert_eq!(err, "sqrt requires a numeric argument");
    }

    #[test]
    fn test_non_numeric_min() {
        let err =
            eval_math_builtin("min", &[Value::Bool(true), Value::Float(1.0)]).unwrap_err();
        assert_eq!(err, "min requires a numeric argument");
    }

    // --- Unknown function returns None ---

    #[test]
    fn test_unknown_function_returns_none() {
        let result = eval_math_builtin("unknown_func", &[Value::Int(1)]).unwrap();
        assert!(result.is_none());
    }
}
