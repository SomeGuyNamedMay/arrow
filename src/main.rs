//! A basic Lisp (Scheme-like) interpreter in Rust, split into modules:
//!
//!   - `value`    the core `Value` enum shared by every stage
//!   - `env`      lexical environments (variable scopes)
//!   - `lexer`    source text -> tokens
//!   - `parser`   tokens -> `Value` trees (s-expressions)
//!   - `eval`     special forms + function application
//!   - `builtins` primitive procedures and the global environment
//!
//! Build:   cargo build --release
//! Run:     cargo run                 (starts a REPL)
//!          cargo run -- somefile.lisp (runs a file, then starts a REPL)

mod builtins;
mod env;
mod eval;
mod lexer;
mod parser;
mod value;

use std::fs;
use std::io::{self, Write};

use env::Env;
use value::Value;

fn run_source(src: &str, env: &Env) {
    match parser::parse_all(src) {
        Ok(exprs) => {
            for e in exprs {
                if let Err(err) = eval::eval(&e, env) {
                    eprintln!("Error: {}", err);
                }
            }
        }
        Err(e) => eprintln!("Parse error: {}", e),
    }
}

fn repl(env: &Env) {
    println!("Basic Lisp interpreter (Rust). Type (exit) or Ctrl-D to quit.");
    let stdin = io::stdin();
    loop {
        print!("lisp> ");
        io::stdout().flush().ok();
        let mut line = String::new();
        let bytes = stdin.read_line(&mut line).unwrap_or(0);
        if bytes == 0 {
            println!();
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "(exit)" {
            break;
        }
        match parser::parse_all(trimmed) {
            Ok(exprs) => {
                for e in exprs {
                    match eval::eval(&e, env) {
                        Ok(v) => {
                            if !matches!(v, Value::Nil) {
                                println!("{}", v);
                            }
                        }
                        Err(err) => println!("Error: {}", err),
                    }
                }
            }
            Err(err) => println!("Parse error: {}", err),
        }
    }
}

fn main() {
    let env = builtins::global_env();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match fs::read_to_string(&args[1]) {
            Ok(src) => run_source(&src, &env),
            Err(e) => {
                eprintln!("Could not read file {}: {}", args[1], e);
                return;
            }
        }
    }

    repl(&env);
}
