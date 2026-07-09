//! Lexical environments: a chain of scopes used for variable lookup,
//! `define`, and `set!`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::value::Value;

/// An environment is a reference-counted, mutable scope with an optional
/// parent. Cloning an `Env` is cheap (it just bumps the `Rc` count) and all
/// clones refer to the same underlying scope, which is what lets closures
/// share and mutate outer variables via `set!`.
pub type Env = Rc<RefCell<EnvData>>;

pub struct EnvData {
    vars: HashMap<String, Value>,
    parent: Option<Env>,
}

pub fn new_env(parent: Option<Env>) -> Env {
    Rc::new(RefCell::new(EnvData {
        vars: HashMap::new(),
        parent,
    }))
}

/// Look up `name`, walking outward through parent scopes if it isn't
/// found locally.
pub fn env_get(env: &Env, name: &str) -> Result<Value, String> {
    if let Some(v) = env.borrow().vars.get(name) {
        return Ok(v.clone());
    }
    match &env.borrow().parent {
        Some(p) => env_get(p, name),
        None => Err(format!("Unbound symbol: {}", name)),
    }
}

/// Create or overwrite a binding in the *local* scope (used by `define`
/// and to bind function parameters).
pub fn env_define(env: &Env, name: &str, value: Value) {
    env.borrow_mut().vars.insert(name.to_string(), value);
}

/// Mutate an *existing* binding, walking outward until one is found
/// (used by `set!`). Unlike `env_define`, this fails if the name was
/// never defined anywhere in the chain.
pub fn env_set(env: &Env, name: &str, value: Value) -> Result<(), String> {
    if env.borrow().vars.contains_key(name) {
        env.borrow_mut().vars.insert(name.to_string(), value);
        return Ok(());
    }
    let parent = env.borrow().parent.clone();
    match parent {
        Some(p) => env_set(&p, name, value),
        None => Err(format!("Cannot set! undefined symbol: {}", name)),
    }
}
