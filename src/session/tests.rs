//! The session driven through the bundled plugins, key by key: core editing,
//! dots, the text-tools and files plugins, and syntax highlighting.

use super::*;
use crate::components::keys::{self, KeyPress};
use crate::components::menu;
use crate::components::text_editor::Selection;
use crate::syntax::{Languages, Syntax};

/// The tests play the window's part: they drive the session's futures to
/// completion, one key at a time.
type App = Session;

fn block_on<F: Future>(future: F) -> F::Output {
    thread_local! {
        static RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a tokio runtime for the tests");
    }
    RUNTIME.with(|runtime| runtime.block_on(future))
}

pub(crate) fn app(text: &str) -> App {
    let app = block_on(Session::start(text, false));
    assert!(
        message(&app).starts_with("text-tools loaded"),
        "plugins should load cleanly: {}",
        message(&app)
    );
    app
}

/// Type keys written in Kakoune notation, e.g. "wd", "itext<esc>".
fn type_keys(app: &mut App, keys: &str) {
    for chord in keys::parse_keys(keys).unwrap() {
        block_on(app.press(KeyPress {
            key: chord.key,
            alt: chord.alt,
            ctrl: chord.ctrl,
            shift: chord.shift,
        }));
    }
}

fn text(app: &App) -> String {
    let editor = app.core.editor.editor();
    editor.slice(0, editor.len())
}

fn selected(app: &App) -> String {
    let editor = app.core.editor.editor();
    let selection = editor.selection();
    editor.slice(selection.start(), selection.end() + 1)
}

fn message(app: &App) -> String {
    let message = app.core.editor.editor().message();
    message.unwrap_or_default().to_string()
}

fn position(app: &App) -> (usize, usize) {
    app.core.editor.editor().cursor_position()
}

/// The items in the open menu, in display order.
fn menu_items(app: &App) -> Vec<menu::MenuItem> {
    app.menu.matching_items().into_iter().cloned().collect()
}

fn menu_item(app: &App, label: &str) -> menu::MenuItem {
    menu_items(app)
        .into_iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("no menu item `{label}`"))
}

#[test]
fn files_opened_from_the_file_manager_are_highlighted() {
    let mut app = app("");
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/syntax");
    app.syntax = Some(Syntax::new(Languages::load(&fixtures).unwrap()).unwrap());
    type_keys(&mut app, "<space>f");
    for value in ["tests", "tests/fixtures", "tests/fixtures/syntax"] {
        highlight(&mut app, value);
        type_keys(&mut app, "l");
    }
    highlight(&mut app, "tests/fixtures/syntax/example.demo");
    type_keys(&mut app, "<ret>");
    assert_eq!(app.core.editor.editor().name(), "tests/fixtures/syntax/example.demo");
    assert!(text(&app).starts_with("let greeting"), "{}", text(&app));
    assert!(app.core.editor.highlighter().is_some(), "{}", message(&app));
}

#[test]
fn the_syntax_tree_follows_edits_made_through_plugins() {
    let mut app = app("let a = 1;\nlet b = 2;\n");
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/syntax");
    app.syntax = Some(Syntax::new(Languages::load(&fixtures).unwrap()).unwrap());
    app.core.editor.editor_mut().set_name("example.demo".into());
    // The first key picks the language; then edits at two dots, in insert
    // mode, in one batch the way the window sends them.
    type_keys(&mut app, "c");
    type_keys(&mut app, "i{ <esc>");
    assert_eq!(text(&app), "{ let a = 1;\n{ let b = 2;\n");
    let tree = app.syntax.as_ref().unwrap().tree().unwrap().root_node().to_sexp();
    assert!(tree.starts_with("(source_file (ERROR"), "unclosed blocks: {tree}");

    // Undo restores the whole text in one replace; the tree still matches.
    type_keys(&mut app, "u");
    assert_eq!(text(&app), "let a = 1;\nlet b = 2;\n");
    let tree = app.syntax.as_ref().unwrap().tree().unwrap().root_node().to_sexp();
    assert_eq!(
        tree,
        "(source_file (binding name: (identifier) (number)) (binding name: (identifier) (number)))"
    );

    type_keys(&mut app, "%dilet c = \"done\";<esc>");
    assert_eq!(text(&app), "let c = \"done\";\n");
    let tree = app.syntax.as_ref().unwrap().tree().unwrap().root_node().to_sexp();
    assert_eq!(tree, "(source_file (binding name: (identifier) (string)))");
    assert!(app.core.editor.highlighter().is_some());
}

// ----- Core plugin: editing behaviour -----

#[test]
fn core_plugin_provides_modes_commands_and_bindings() {
    let mut app = app("");
    assert_eq!(app.core.editor.editor().mode(), "normal");
    assert!(app.core.registry.fallback("insert").is_some());
    // The command menu comes from core too, through the generic menu.
    type_keys(&mut app, ":");
    assert!(app.menu.is_open());
    let items = menu_items(&app);
    assert!(items.len() > 60, "{} commands", items.len());
    assert_eq!(menu_item(&app, "command-menu").annotation, ":");
    assert_eq!(menu_item(&app, "quit").detail, "Close Arrow (app)");
    type_keys(&mut app, "<esc>");
    assert!(!app.menu.is_open());
}

#[test]
fn describe_bindings_lists_and_explains_every_binding() {
    let mut app = app("abc");
    type_keys(&mut app, "?");
    assert!(app.menu.is_open());
    let items = menu_items(&app);
    // Normal-mode bindings come first, then insert mode.
    assert_eq!(items[0].annotation, "normal");
    assert_eq!(items.last().unwrap().annotation, "insert");
    // Bindings from other plugins are listed and named.
    let duplicate = items
        .iter()
        .find(|item| item.label.ends_with("duplicate-line"))
        .unwrap();
    assert!(duplicate.label.starts_with("<space>d"), "{}", duplicate.label);
    assert!(duplicate.detail.ends_with("(text-tools)"), "{}", duplicate.detail);

    // Search by keys and command name (goto-line-start is also on <home>);
    // choosing explains the binding without running it.
    type_keys(&mut app, "gh<space>goto<ret>");
    assert!(!app.menu.is_open());
    assert_eq!(
        message(&app),
        "gh in normal mode runs goto-line-start: Go to the start of the line"
    );
    assert_eq!(text(&app), "abc\n");
}

#[test]
fn command_menu_lists_recent_commands_first() {
    let mut app = app("abc");
    type_keys(&mut app, ":select-line<ret>");
    assert_eq!(selected(&app), "abc\n");
    type_keys(&mut app, ":");
    assert_eq!(menu_items(&app)[0].label, "select-line");
    // A takes-char command chosen from the menu still waits for its character.
    type_keys(&mut app, "<c-backspace>replace-chars<ret>");
    assert_eq!(message(&app), "replace-chars: type a character");
    type_keys(&mut app, "z");
    assert_eq!(text(&app), "zzz\n");
}

#[test]
fn word_motion_selects_word_and_trailing_space() {
    let mut app = app("hello world");
    type_keys(&mut app, "w");
    assert_eq!(selected(&app), "hello ");
    type_keys(&mut app, "w");
    assert_eq!(selected(&app), "world");
    type_keys(&mut app, "<a-;>");
    assert_eq!(app.core.editor.editor().selection().cursor, 6);
}

#[test]
fn delete_word_and_undo() {
    let mut app = app("hello world");
    type_keys(&mut app, "wd");
    assert_eq!(text(&app), "world\n");
    type_keys(&mut app, "u");
    assert_eq!(text(&app), "hello world\n");
    type_keys(&mut app, "U");
    assert_eq!(text(&app), "world\n");
}

#[test]
fn insert_session_types_and_undoes_as_one_step() {
    let mut app = app("hello world");
    type_keys(&mut app, "eshi<esc>");
    assert_eq!(text(&app), "hi world\n");
    assert_eq!(app.core.editor.editor().mode(), "normal");
    type_keys(&mut app, "u");
    assert_eq!(text(&app), "hello world\n");
}

#[test]
fn insert_mode_newlines_keep_indent() {
    let mut app = app("    foo");
    type_keys(&mut app, "obar<ret>baz<esc>");
    assert_eq!(text(&app), "    foo\n    bar\n    baz\n");
}

#[test]
fn line_selection_yank_and_paste() {
    let mut app = app("one\ntwo\nthree");
    type_keys(&mut app, "x");
    assert_eq!(selected(&app), "one\n");
    type_keys(&mut app, "x");
    assert_eq!(selected(&app), "one\ntwo\n");
    type_keys(&mut app, ";xyp");
    assert_eq!(text(&app), "one\ntwo\ntwo\nthree\n");
}

#[test]
fn replace_whole_buffer_with_yanked_line() {
    let mut app = app("one\ntwo");
    type_keys(&mut app, "xy%R");
    assert_eq!(text(&app), "one\n");
}

#[test]
fn vertical_motion_remembers_column() {
    let mut app = app("abcdef\nab\nabcdef");
    type_keys(&mut app, "glj");
    assert_eq!(position(&app), (1, 2));
    type_keys(&mut app, "j");
    assert_eq!(position(&app), (2, 5));
    type_keys(&mut app, "hk");
    assert_eq!(position(&app), (1, 2));
}

#[test]
fn replace_chars_waits_for_a_character() {
    let mut app = app("abc");
    type_keys(&mut app, "Lr");
    assert_eq!(message(&app), "replace-chars: type a character");
    type_keys(&mut app, "x");
    assert_eq!(text(&app), "xxc\n");
    type_keys(&mut app, "%~");
    assert_eq!(text(&app), "XXC\n");
}

#[test]
fn indent_and_dedent() {
    let mut app = app("a\nb");
    type_keys(&mut app, "xx>");
    assert_eq!(text(&app), "    a\n    b\n");
    type_keys(&mut app, "<lt>");
    assert_eq!(text(&app), "a\nb\n");
}

#[test]
fn prefix_shows_hint_and_unknown_key_cancels() {
    let mut app = app("abc");
    type_keys(&mut app, "g");
    let hint = message(&app);
    assert!(hint.starts_with("goto: h goto-line-start"), "{hint}");
    type_keys(&mut app, "z");
    assert_eq!(message(&app), "");
    assert_eq!(text(&app), "abc\n");
}

// ----- Dots -----

fn dots(app: &App) -> Vec<Selection> {
    app.core.editor.editor().dots().to_vec()
}

#[test]
fn c_spawns_dots_below_and_edits_happen_at_every_dot() {
    let mut app = app("abc\ndef\nghi");
    type_keys(&mut app, "lcc");
    assert_eq!(
        dots(&app),
        [Selection::point(1), Selection::point(5), Selection::point(9)]
    );
    type_keys(&mut app, "iX<esc>");
    assert_eq!(text(&app), "aXbc\ndXef\ngXhi\n");
    // The whole multi-dot insert undoes as one step.
    type_keys(&mut app, "u");
    assert_eq!(text(&app), "abc\ndef\nghi\n");
}

#[test]
fn spawning_past_the_last_line_says_so() {
    let mut app = app("abc\ndef");
    type_keys(&mut app, "cc");
    assert_eq!(dots(&app).len(), 2);
    assert_eq!(message(&app), "no line below for a new dot");
}

#[test]
fn comma_removes_the_oldest_dot() {
    let mut app = app("abc\ndef\nghi");
    type_keys(&mut app, "cc,");
    assert_eq!(dots(&app), [Selection::point(4), Selection::point(8)]);
    type_keys(&mut app, "d");
    assert_eq!(text(&app), "abc\nef\nhi\n");
    type_keys(&mut app, ",,");
    assert_eq!(dots(&app).len(), 1);
    assert_eq!(message(&app), "only one dot");
}

#[test]
fn dots_grow_into_selections() {
    let mut app = app("one two\nthree four");
    type_keys(&mut app, "cw");
    assert_eq!(
        dots(&app),
        [
            Selection {
                anchor: 0,
                cursor: 3
            },
            Selection {
                anchor: 8,
                cursor: 13
            }
        ]
    );
    type_keys(&mut app, "~");
    assert_eq!(text(&app), "ONE two\nTHREE four\n");
    // `s` changes the text of every dot.
    type_keys(&mut app, "sX<esc>");
    assert_eq!(text(&app), "Xtwo\nXfour\n");
}

#[test]
fn each_dot_pastes_its_own_yank() {
    let mut app = app("ab\ncd");
    type_keys(&mut app, "cyp");
    assert_eq!(text(&app), "aab\nccd\n");
}

#[test]
fn dots_that_meet_merge() {
    let mut app = app("abc\ndef");
    type_keys(&mut app, "ck");
    assert_eq!(dots(&app), [Selection::point(0)]);
}

// ----- text-tools plugin: commands, bindings and hooks -----

#[test]
fn plugin_command_runs_from_the_menu_and_undoes_in_one_step() {
    let mut app = app("b\na\nc");
    type_keys(&mut app, "%:sort-lines<ret>");
    assert!(!app.menu.is_open());
    assert_eq!(text(&app), "a\nb\nc\n");
    type_keys(&mut app, "u");
    assert_eq!(text(&app), "b\na\nc\n");
}

#[test]
fn plugin_key_bindings() {
    let mut app = app("one\ntwo");
    type_keys(&mut app, "<space>d");
    assert_eq!(text(&app), "one\none\ntwo\n");
    // Bound by text-tools to a command from core.
    type_keys(&mut app, "<space>a");
    assert_eq!(selected(&app), text(&app));
    type_keys(&mut app, ":");
    let sort = menu_item(&app, "sort-lines");
    assert_eq!(sort.annotation, "<a-s>");
    assert!(sort.detail.ends_with("(text-tools)"), "{}", sort.detail);
}

#[test]
fn other_plugins_edits_keep_dots_in_place_and_undo_separately() {
    let mut app = app("ab\ncd\nef");
    // Dots on "c" and "e"; then text-tools duplicates the primary dot's line.
    type_keys(&mut app, "jc<space>d");
    assert_eq!(text(&app), "ab\ncd\nef\nef\n");
    // The older dot is still on "c".
    assert_eq!(dots(&app)[0], Selection::point(3));
    type_keys(&mut app, "x");
    type_keys(&mut app, "u");
    assert_eq!(text(&app), "ab\ncd\nef\n");
}

// ----- files plugin: the tree menu -----

/// Move the file manager's highlight down to the node with this value.
fn highlight(app: &mut App, value: &str) {
    for _ in 0..50 {
        if app.tree.highlighted_value() == Some(value) {
            return;
        }
        type_keys(app, "j");
    }
    panic!("`{value}` not found in the file manager");
}

#[test]
fn file_manager_browses_and_opens_files() {
    let mut app = app("scratch");
    type_keys(&mut app, "<space>f");
    assert!(app.tree.is_open());
    // Directories come first, sorted by name; hidden ones like .git are left out.
    assert_eq!(app.tree.highlighted_value(), Some("plugins"));

    highlight(&mut app, "src");
    type_keys(&mut app, "l");
    highlight(&mut app, "src/main.rs");
    // h goes back up and l returns to the same place.
    type_keys(&mut app, "hl");
    assert_eq!(app.tree.highlighted_value(), Some("src/main.rs"));

    type_keys(&mut app, "<ret>");
    assert!(!app.tree.is_open());
    assert!(text(&app).starts_with("//! Arrow"), "{}", &text(&app)[..40]);
    assert_eq!(app.core.editor.editor().name(), "src/main.rs");
    assert_eq!(message(&app), "opened src/main.rs");

    // Loading a file is an undoable change like any other.
    type_keys(&mut app, "u");
    assert_eq!(text(&app), "scratch\n");
}

#[test]
fn file_manager_closes_with_escape_and_only_it_can_read_files() {
    let mut app = app("");
    type_keys(&mut app, "<space>f<esc>");
    assert!(!app.tree.is_open());
    // Plugins without file access can't open files: the command belongs to
    // the files plugin, which runs it in its own sandbox.
    type_keys(&mut app, ":open-file<ret>");
    assert!(message(&app).contains("needs a path"), "{}", message(&app));
}

#[test]
fn event_hooks_see_commands_and_changes() {
    let mut app = app("one");
    type_keys(&mut app, "<space>d:plugin-stats<ret>");
    let stats = message(&app);
    // Opening the menu (command-menu) and choosing from it (execute-command,
    // which then runs plugin-stats) are commands too.
    assert!(
        stats.starts_with("seen: 3 commands (last: execute-command), 1 buffer changes"),
        "{stats}"
    );
}
