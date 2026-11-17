# arrow

A small Rust compiler scaffold using [nom](https://docs.rs/nom) for parsing and [Cranelift](https://docs.rs/cranelift) for native JIT compilation.

Expressions support integers, addition, subtraction, and multiplication:

```sh
cargo run -- "1 + 2 * 3"
```

Run the parser tests with:

```sh
cargo test
```

## Language features

Syntax highlighting and text objects use an internal AST returned by a
language-specific Python parser callback; they do not use LSP or Tree-sitter.
The parser receives the complete source and returns nodes created with
`arrow.ast_node(kind, start, end, children=(), capture=None)`. Offsets are
zero-based Unicode character positions, with an exclusive end. The editor
validates node ranges and uses `capture` names for highlighting and `kind`
names for configured text objects.

For example:

```python
def parse_example(source):
    start = source.find("fn ")
    if start < 0:
        return arrow.ast_node("module", 0, len(source))
    end = len(source)
    return arrow.ast_node("module", 0, end, [
        arrow.ast_node("function", start, end, [
            arrow.ast_node("keyword", start, start + 2, capture="keyword")
        ])
    ])

arrow.register_language(
    "example",
    ("example",),
    parser=parse_example,
    highlight_colors={"keyword": (0.78, 0.55, 0.96, 2)},
    text_object_kinds=("function",),
)
```

Language registrations may also specify an optional LSP server with
`command`, `args`, and `root_markers`; it remains available for features such
as completion. The default Rust and Python registrations configure
`rust-analyzer` and `pyright-langserver` for LSP features.

The default Rust and Python registrations also bundle parsers
(`arrow.parse_rust` and `arrow.parse_python` in `python/defaults/syntax.py`).
They are error-tolerant lexers that capture comments, strings, numbers,
keywords, types, constants, macros, and function names, and that emit a
`function` node for every function so `select-function` works. Register a
language again with your own `parser` to replace them.