//! The core `Value` type shared by every other module: the tokenizer produces
//! text, the parser turns that text into `Value` trees, and the evaluator
//! walks those trees, producing more `Value`s.

use std::fmt;
use std::rc::Rc;

use crate::env::Env;

#[derive(Clone)]
pub enum Value {
    Number(f64),
    Symbol(String),
    Str(String),
    Bool(bool),
    List(Vec<Value>),
    Nil,
    Lambda(Rc<LambdaData>),
    Builtin(&'static str, fn(&[Value]) -> Result<Value, String>),
}

/// A user-defined function: its parameter names, its body (a sequence of
/// expressions to evaluate in order), and the environment it closed over
/// at the point it was created (this is what gives us lexical scoping).
pub struct LambdaData {
    pub params: Vec<String>,
    pub body: Vec<Value>,
    pub env: Env,
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Number(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            Value::Symbol(s) => write!(f, "{}", s),
            Value::Str(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", if *b { "#t" } else { "#f" }),
            Value::Nil => write!(f, "()"),
            Value::List(items) => {
                let strs: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                write!(f, "({})", strs.join(" "))
            }
            Value::Lambda(_) => write!(f, "#<lambda>"),
            Value::Builtin(name, _) => write!(f, "#<builtin:{}>", name),
        }
    }
}

/// Anything other than the boolean `#f` counts as true, Scheme-style.
pub fn is_truthy(v: &Value) -> bool {
    !matches!(v, Value::Bool(false))
}
