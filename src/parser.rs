//! Recursive-descent parser: turns the token stream from `lexer` into
//! `Value` trees (s-expressions) that `eval` can walk.

use crate::lexer::tokenize;
use crate::value::Value;

/// Parse a single expression starting at `tokens[0]`, returning the parsed
/// value and how many tokens it consumed.
fn parse(tokens: &[String]) -> Result<(Value, usize), String> {
    if tokens.is_empty() {
        return Err("Unexpected EOF while parsing".to_string());
    }

    let tok = &tokens[0];
    match tok.as_str() {
        "(" => {
            let mut items = Vec::new();
            let mut i = 1;
            loop {
                if i >= tokens.len() {
                    return Err("Unexpected EOF, expected ')'".to_string());
                }
                if tokens[i] == ")" {
                    i += 1;
                    break;
                }
                let (val, consumed) = parse(&tokens[i..])?;
                items.push(val);
                i += consumed;
            }
            Ok((Value::List(items), i))
        }
        ")" => Err("Unexpected ')'".to_string()),
        "'" => {
            // 'expr desugars to (quote expr)
            let (val, consumed) = parse(&tokens[1..])?;
            let quoted = Value::List(vec![Value::Symbol("quote".to_string()), val]);
            Ok((quoted, consumed + 1))
        }
        _ => Ok((parse_atom(tok), 1)),
    }
}

fn parse_atom(tok: &str) -> Value {
    if tok.starts_with('"') && tok.ends_with('"') && tok.len() >= 2 {
        return Value::Str(tok[1..tok.len() - 1].to_string());
    }
    if tok == "#t" {
        return Value::Bool(true);
    }
    if tok == "#f" {
        return Value::Bool(false);
    }
    if let Ok(n) = tok.parse::<f64>() {
        return Value::Number(n);
    }
    Value::Symbol(tok.to_string())
}

/// Parse a whole source string, which may contain several top-level
/// expressions back to back.
pub fn parse_all(src: &str) -> Result<Vec<Value>, String> {
    let tokens = tokenize(src);
    let mut exprs = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let (val, consumed) = parse(&tokens[i..])?;
        exprs.push(val);
        i += consumed;
    }
    Ok(exprs)
}
