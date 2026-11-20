// A tiny grammar for Arrow's syntax tests: `let` bindings, blocks, numbers,
// strings and comments. Build `demo.wasm` with:
//   tree-sitter generate && tree-sitter build --wasm -o ../syntax/grammars/demo.wasm
module.exports = grammar({
  name: 'demo',
  extras: $ => [/\s/, $.comment],
  word: $ => $.identifier,
  rules: {
    source_file: $ => repeat($._item),
    _item: $ => choice($.binding, $.block),
    binding: $ => seq('let', field('name', $.identifier), '=', $._value, ';'),
    block: $ => seq('{', repeat($._item), '}'),
    _value: $ => choice($.identifier, $.number, $.string),
    identifier: _ => /[a-z_]+/,
    number: _ => /\d+/,
    string: _ => /"[^"]*"/,
    comment: _ => /#[^\n]*/,
  },
});
