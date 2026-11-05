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