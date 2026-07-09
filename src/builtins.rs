//! Primitive procedures (arithmetic, comparisons, list operations, I/O)
//! and the global environment that wires them all up under their Lisp
//! names.

use std::io::{self, Write};

use crate::env::{Env, env_define, new_env};
use crate::value::{Value, is_truthy};

fn as_number(v: &Value) -> Result<f64, String> {
    match v {
        Value::Number(n) => Ok(*n),
        _ => Err(format!("Expected number, got {}", v)),
    }
}

fn builtin_add(args: &[Value]) -> Result<Value, String> {
    let mut sum = 0.0;
    for a in args {
        sum += as_number(a)?;
    }
    Ok(Value::Number(sum))
}

fn builtin_sub(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("- needs at least 1 argument".to_string());
    }
    if args.len() == 1 {
        return Ok(Value::Number(-as_number(&args[0])?));
    }
    let mut result = as_number(&args[0])?;
    for a in &args[1..] {
        result -= as_number(a)?;
    }
    Ok(Value::Number(result))
}

fn builtin_mul(args: &[Value]) -> Result<Value, String> {
    let mut prod = 1.0;
    for a in args {
        prod *= as_number(a)?;
    }
    Ok(Value::Number(prod))
}

fn builtin_div(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("/ needs at least 1 argument".to_string());
    }
    if args.len() == 1 {
        return Ok(Value::Number(1.0 / as_number(&args[0])?));
    }
    let mut result = as_number(&args[0])?;
    for a in &args[1..] {
        let d = as_number(a)?;
        if d == 0.0 {
            return Err("Division by zero".to_string());
        }
        result /= d;
    }
    Ok(Value::Number(result))
}

macro_rules! cmp_builtin {
    ($name:ident, $op:tt) => {
        fn $name(args: &[Value]) -> Result<Value, String> {
            for w in args.windows(2) {
                let a = as_number(&w[0])?;
                let b = as_number(&w[1])?;
                if !(a $op b) {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }
    };
}

cmp_builtin!(builtin_lt, <);
cmp_builtin!(builtin_gt, >);
cmp_builtin!(builtin_le, <=);
cmp_builtin!(builtin_ge, >=);

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Symbol(x), Value::Symbol(y)) => x == y,
        (Value::Nil, Value::Nil) => true,
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| values_equal(p, q))
        }
        _ => false,
    }
}

fn builtin_eq(args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(values_equal(&args[0], &args[1])))
}

fn builtin_cons(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("cons needs 2 arguments".to_string());
    }
    let mut list = match &args[1] {
        Value::List(l) => l.clone(),
        Value::Nil => Vec::new(),
        other => vec![other.clone()],
    };
    list.insert(0, args[0].clone());
    Ok(Value::List(list))
}

fn builtin_car(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::List(l) if !l.is_empty() => Ok(l[0].clone()),
        _ => Err("car needs a non-empty list".to_string()),
    }
}

fn builtin_cdr(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::List(l) if !l.is_empty() => Ok(Value::List(l[1..].to_vec())),
        _ => Err("cdr needs a non-empty list".to_string()),
    }
}

fn builtin_list(args: &[Value]) -> Result<Value, String> {
    Ok(Value::List(args.to_vec()))
}

fn builtin_is_null(args: &[Value]) -> Result<Value, String> {
    let empty = match &args[0] {
        Value::Nil => true,
        Value::List(l) => l.is_empty(),
        _ => false,
    };
    Ok(Value::Bool(empty))
}

fn builtin_is_list(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args[0], Value::List(_) | Value::Nil)))
}

fn builtin_length(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::List(l) => Ok(Value::Number(l.len() as f64)),
        Value::Nil => Ok(Value::Number(0.0)),
        _ => Err("length needs a list".to_string()),
    }
}

fn builtin_display(args: &[Value]) -> Result<Value, String> {
    for a in args {
        print!("{}", a);
    }
    io::stdout().flush().ok();
    Ok(Value::Nil)
}

fn builtin_newline(_args: &[Value]) -> Result<Value, String> {
    println!();
    Ok(Value::Nil)
}

fn builtin_not(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(!is_truthy(&args[0])))
}

/// Build a fresh top-level environment with every builtin procedure
/// registered under its Lisp name.
pub fn global_env() -> Env {
    let env = new_env(None);
    let builtins: &[(&'static str, fn(&[Value]) -> Result<Value, String>)] = &[
        ("+", builtin_add),
        ("-", builtin_sub),
        ("*", builtin_mul),
        ("/", builtin_div),
        ("<", builtin_lt),
        (">", builtin_gt),
        ("<=", builtin_le),
        (">=", builtin_ge),
        ("=", builtin_eq),
        ("eq?", builtin_eq),
        ("equal?", builtin_eq),
        ("cons", builtin_cons),
        ("car", builtin_car),
        ("cdr", builtin_cdr),
        ("list", builtin_list),
        ("null?", builtin_is_null),
        ("list?", builtin_is_list),
        ("length", builtin_length),
        ("display", builtin_display),
        ("newline", builtin_newline),
        ("not", builtin_not),
    ];
    for (name, func) in builtins {
        env_define(&env, name, Value::Builtin(name, *func));
    }
    env
}
