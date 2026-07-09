//! The evaluator: special forms (`if`, `define`, `lambda`, ...) and
//! function application for both builtins and user-defined lambdas.

use std::rc::Rc;

use crate::env::{env_define, env_get, env_set, new_env, Env};
use crate::value::{is_truthy, LambdaData, Value};

pub fn eval(expr: &Value, env: &Env) -> Result<Value, String> {
    match expr {
        Value::Number(_) | Value::Str(_) | Value::Bool(_) | Value::Nil => Ok(expr.clone()),
        Value::Symbol(s) => env_get(env, s),
        Value::Lambda(_) | Value::Builtin(_, _) => Ok(expr.clone()),
        Value::List(items) => {
            if items.is_empty() {
                return Ok(Value::Nil);
            }
            if let Value::Symbol(op) = &items[0] {
                match op.as_str() {
                    "quote" => return Ok(items[1].clone()),
                    "if" => {
                        let cond = eval(&items[1], env)?;
                        return if is_truthy(&cond) {
                            eval(&items[2], env)
                        } else if items.len() > 3 {
                            eval(&items[3], env)
                        } else {
                            Ok(Value::Nil)
                        };
                    }
                    "define" => return eval_define(items, env),
                    "set!" => {
                        let name = match &items[1] {
                            Value::Symbol(s) => s.clone(),
                            _ => return Err("set! needs a symbol".to_string()),
                        };
                        let val = eval(&items[2], env)?;
                        env_set(env, &name, val)?;
                        return Ok(Value::Nil);
                    }
                    "lambda" => return eval_lambda(items, env),
                    "let" => return eval_let(items, env),
                    "begin" => {
                        let mut result = Value::Nil;
                        for e in &items[1..] {
                            result = eval(e, env)?;
                        }
                        return Ok(result);
                    }
                    "cond" => return eval_cond(items, env),
                    "and" => {
                        let mut result = Value::Bool(true);
                        for e in &items[1..] {
                            result = eval(e, env)?;
                            if !is_truthy(&result) {
                                return Ok(Value::Bool(false));
                            }
                        }
                        return Ok(result);
                    }
                    "or" => {
                        for e in &items[1..] {
                            let result = eval(e, env)?;
                            if is_truthy(&result) {
                                return Ok(result);
                            }
                        }
                        return Ok(Value::Bool(false));
                    }
                    _ => {}
                }
            }
            // Not a special form: evaluate the operator and arguments,
            // then apply.
            let f = eval(&items[0], env)?;
            let args: Vec<Value> = items[1..]
                .iter()
                .map(|e| eval(e, env))
                .collect::<Result<_, _>>()?;
            apply(&f, &args)
        }
    }
}

/// `(define name expr)` or `(define (name args...) body...)`
fn eval_define(items: &[Value], env: &Env) -> Result<Value, String> {
    match &items[1] {
        Value::Symbol(name) => {
            let val = eval(&items[2], env)?;
            env_define(env, name, val);
            Ok(Value::Symbol(name.clone()))
        }
        Value::List(sig) => {
            let name = match &sig[0] {
                Value::Symbol(s) => s.clone(),
                _ => return Err("Bad define signature".to_string()),
            };
            let params: Vec<String> = sig[1..]
                .iter()
                .map(|v| match v {
                    Value::Symbol(s) => Ok(s.clone()),
                    _ => Err("Bad parameter name".to_string()),
                })
                .collect::<Result<_, _>>()?;
            let body = items[2..].to_vec();
            let lambda = Value::Lambda(Rc::new(LambdaData {
                params,
                body,
                env: env.clone(),
            }));
            env_define(env, &name, lambda);
            Ok(Value::Symbol(name))
        }
        _ => Err("Bad define form".to_string()),
    }
}

/// `(lambda (params...) body...)`
fn eval_lambda(items: &[Value], env: &Env) -> Result<Value, String> {
    let params: Vec<String> = match &items[1] {
        Value::List(ps) => ps
            .iter()
            .map(|v| match v {
                Value::Symbol(s) => Ok(s.clone()),
                _ => Err("Bad parameter name".to_string()),
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("lambda needs a parameter list".to_string()),
    };
    let body = items[2..].to_vec();
    Ok(Value::Lambda(Rc::new(LambdaData {
        params,
        body,
        env: env.clone(),
    })))
}

/// `(let ((a 1) (b 2)) body...)`
fn eval_let(items: &[Value], env: &Env) -> Result<Value, String> {
    let bindings = match &items[1] {
        Value::List(b) => b,
        _ => return Err("let needs a binding list".to_string()),
    };
    let new_scope = new_env(Some(env.clone()));
    for b in bindings {
        if let Value::List(pair) = b {
            let name = match &pair[0] {
                Value::Symbol(s) => s.clone(),
                _ => return Err("Bad let binding".to_string()),
            };
            // Bindings are evaluated in the *outer* env (not let*-style),
            // matching plain `let` semantics.
            let val = eval(&pair[1], env)?;
            env_define(&new_scope, &name, val);
        }
    }
    let mut result = Value::Nil;
    for b in &items[2..] {
        result = eval(b, &new_scope)?;
    }
    Ok(result)
}

/// `(cond (test expr...) ... (else expr...))`
fn eval_cond(items: &[Value], env: &Env) -> Result<Value, String> {
    for clause in &items[1..] {
        if let Value::List(c) = clause {
            let test = &c[0];
            let is_else = matches!(test, Value::Symbol(s) if s == "else");
            if is_else || is_truthy(&eval(test, env)?) {
                let mut result = Value::Nil;
                for e in &c[1..] {
                    result = eval(e, env)?;
                }
                return Ok(result);
            }
        }
    }
    Ok(Value::Nil)
}

/// Apply a function (builtin or user-defined lambda) to already-evaluated
/// arguments.
pub fn apply(f: &Value, args: &[Value]) -> Result<Value, String> {
    match f {
        Value::Builtin(_, func) => func(args),
        Value::Lambda(data) => {
            if data.params.len() != args.len() {
                return Err(format!(
                    "Expected {} arguments, got {}",
                    data.params.len(),
                    args.len()
                ));
            }
            let call_env = new_env(Some(data.env.clone()));
            for (p, a) in data.params.iter().zip(args.iter()) {
                env_define(&call_env, p, a.clone());
            }
            let mut result = Value::Nil;
            for e in &data.body {
                result = eval(e, &call_env)?;
            }
            Ok(result)
        }
        _ => Err(format!("Not a function: {}", f)),
    }
}
