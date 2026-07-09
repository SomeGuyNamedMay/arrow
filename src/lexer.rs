//! Turns raw Lisp source text into a flat stream of tokens: parens, the
//! quote shorthand `'`, string literals, and atoms (numbers/symbols).
//! The parser consumes this stream to build `Value` trees.

pub fn tokenize(src: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = src.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            ';' => {
                // Line comment: skip to end of line.
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '(' | ')' => {
                tokens.push(c.to_string());
                chars.next();
            }
            '\'' => {
                tokens.push("'".to_string());
                chars.next();
            }
            '"' => {
                let mut s = String::from("\"");
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                s.push('"');
                tokens.push(s);
            }
            _ => {
                let mut atom = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' {
                        break;
                    }
                    atom.push(c);
                    chars.next();
                }
                tokens.push(atom);
            }
        }
    }
    return tokens;
}
