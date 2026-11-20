//! Grammar loading, incremental parsing and highlighting, against the tiny demo
//! grammar in `tests/fixtures`.

use std::path::{Path, PathBuf};

use iced::Color;

use super::config::{DEFAULT_THEME, parse_color};
use super::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/syntax")
}

fn syntax() -> Syntax {
    Syntax::new(Languages::load(&fixtures()).unwrap()).unwrap()
}

fn demo_buffer(text: &str) -> TextEditor {
    let mut editor = TextEditor::new(text);
    editor.editor_mut().set_name("example.demo".into());
    editor
}

fn sexp(syntax: &Syntax) -> String {
    syntax.tree().unwrap().root_node().to_sexp()
}

fn hex(color: Option<Color>) -> String {
    color.map_or("-".into(), |c| {
        let [r, g, b, _] = c.into_rgba8();
        format!("#{r:02x}{g:02x}{b:02x}")
    })
}

#[test]
fn the_buffer_name_picks_the_language() {
    let mut syntax = syntax();
    let mut editor = demo_buffer("let x = 1;\n");
    syntax.update(&mut editor).unwrap();
    assert_eq!(
        sexp(&syntax),
        "(source_file (binding name: (identifier) (number)))"
    );
    assert!(editor.highlighter().is_some());

    // A name with no configured language turns highlighting off.
    editor.editor_mut().set_name("notes.txt".into());
    syntax.update(&mut editor).unwrap();
    assert!(syntax.tree().is_none());
    assert!(editor.highlighter().is_none());
}

#[test]
fn incremental_parses_match_a_fresh_parse() {
    let mut syntax = syntax();
    let mut editor = demo_buffer("let a = 1;\n{ let b = \"two\"; }\n");
    syntax.update(&mut editor).unwrap();
    // Edits on several lines, multi-line inserts and non-ASCII text, all
    // applied in one batch the way a multi-dot command does.
    let text = editor.editor_mut();
    text.replace(4, 5, "first");
    text.replace(0, 0, "# héllo\nlet z = 9;\n");
    let end = text.len();
    text.replace(end, end, "{\n  { let c = d; }\n}\n");
    syntax.update(&mut editor).unwrap();

    let mut fresh = self::syntax();
    let mut copy = demo_buffer(&editor.editor().slice(0, editor.editor().len()));
    fresh.update(&mut copy).unwrap();
    assert_eq!(sexp(&syntax), sexp(&fresh));
    assert!(!sexp(&syntax).contains("ERROR"), "{}", sexp(&syntax));
}

#[test]
fn highlights_use_the_theme_innermost_and_first_pattern_first() {
    let mut syntax = syntax();
    let mut editor = demo_buffer("let x = y; # é\n");
    syntax.update(&mut editor).unwrap();
    let rope = editor.editor().rope();
    let colors = editor.highlighter().unwrap().colors(rope, 0..rope.len_chars());
    let at = |i: usize| hex(colors[i]);
    assert_eq!(at(0), "#ff0000", "`let` is a keyword (theme)");
    assert_eq!(at(3), "-", "spaces aren't captured");
    // `x` is captured as @variable first, so later patterns don't recolour it.
    assert_eq!(at(4), "#0000ff");
    assert_eq!(at(8), "#00ff00", "`y` falls through to @variable.other");
    let comment = hex(DEFAULT_THEME.iter().find(|(n, _)| *n == "comment").map(|(_, c)| *c));
    assert_eq!(at(11), comment, "comments use the default theme");
    assert_eq!(at(13), comment, "and colour every character, not every byte");

    // Only the requested characters are coloured.
    let visible = editor.highlighter().unwrap().colors(rope, 4..9);
    assert_eq!(visible.len(), 5);
    assert_eq!(hex(visible[0]), "#0000ff");
}

#[test]
fn missing_grammars_and_bad_config_are_reported() {
    let mut syntax = syntax();
    let mut editor = TextEditor::new("anything\n");
    editor.editor_mut().set_name("file.missing".into());
    let error = syntax.update(&mut editor).unwrap_err();
    assert!(error.contains("missing.wasm"), "{error}");
    // Reported once, not on every key.
    assert!(syntax.update(&mut editor).is_ok());

    assert!(parse_color("#12345").is_none());
    assert_eq!(hex(parse_color("#0a0B0c")), "#0a0b0c");
}
