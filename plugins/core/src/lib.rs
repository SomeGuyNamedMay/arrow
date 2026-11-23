//! Arrow's core plugin: Kakoune-style modes, editing commands and key bindings.
//!
//! Editing works through dots: cursors that are also selections. Most commands run
//! once at every dot. `c` spawns a new dot on the line below the newest one and
//! `,` removes the oldest.
//!
//! The editor only stores text and dots, so all behaviour lives here: moving and
//! resizing dots, shifting every dot when the text changes ([`edit`]), merging
//! dots that overlap, keeping a final newline, and undo history (one per buffer;
//! the yank register is shared, so text can be copied between buffers). It uses the same
//! `editor` and `registry` interfaces (`wit/arrow.wit`) any other plugin uses, so a
//! plugin loaded later can replace any of these commands or bindings.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

wit_bindgen::generate!({
    world: "plugin",
    path: "../../wit",
    additional_derives: [PartialEq, Eq],
});

use arrow::plugin::editor as ed;
use arrow::plugin::registry as reg;
use arrow::plugin::types::{Menu, MenuItem, Selection};
use arrow::plugin::ui;

struct Core;

impl Guest for Core {
    fn init() {
        keep_final_newline();
        // Start tracking from here, so edits other plugins make before our first
        // command (like loading a file) can be undone.
        remember_state();
        reg::define_mode("normal", None);
        reg::define_mode("insert", Some("insert-char"));

        for command in COMMANDS {
            reg::register_command(command.name, command.description, command.takes_char);
        }
        for (mode, bindings) in [("normal", NORMAL_BINDINGS), ("insert", INSERT_BINDINGS)] {
            for (keys, command) in bindings {
                if let Err(error) = reg::bind_key(mode, keys, command) {
                    ed::message(&format!("core: {error}"));
                }
            }
        }
        let _ = reg::name_prefix("normal", "g", "goto");
        let _ = reg::name_prefix("normal", "G", "extend to");
    }

    fn run_command(name: String, arg: Option<String>) {
        follow_buffer();
        let arg = arg.as_deref();
        let Some(command) = COMMANDS.iter().find(|c| c.name == name) else {
            ed::message(&format!("core has no command `{name}`"));
            return;
        };
        let history = !matches!(command.name, "undo" | "redo");
        if history {
            begin_undo_group();
        }
        if command.yanks {
            yank_all();
        }
        match command.scope {
            Scope::Once => (command.run)(arg),
            Scope::EachDot => {
                // `edit` shifts every dot after each change, so indices stay valid.
                for index in 0..dot_count() {
                    DOT.set(index);
                    (command.run)(arg);
                }
            }
        }
        merge_dots();
        if history {
            end_undo_group();
        }
        remember_state();
    }

    fn on_event(_event: Event) {}
}

export!(Core);

// ----- Plugin state (lives as long as the plugin instance) -----

/// The editor's text and dots at one moment, for undo.
#[derive(Clone)]
struct Snapshot {
    text: String,
    dots: Vec<Selection>,
}

#[derive(Default)]
struct State {
    /// The yank register: one entry per dot that was yanked.
    register: Vec<String>,
    /// Commands run from the command menu, most recent first.
    recent_commands: Vec<String>,
    /// The buffer `history` belongs to.
    buffer: u32,
    history: History,
    /// The other buffers' histories, kept while they aren't current.
    parked: HashMap<u32, History>,
}

/// What belongs to one buffer: its undo history, and where its dots aim.
#[derive(Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// State at the start of the current undo group, if one is open. A group
    /// stays open for a whole insert session.
    group: Option<Snapshot>,
    /// State after our last command, to notice edits other plugins made since.
    known: Option<Snapshot>,
    /// Per dot: the column to aim for in vertical motion, and where the cursor was
    /// left. If the cursor has moved since, the goal no longer applies.
    goals: Vec<Option<(u32, u32)>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The dot the current per-dot command is working on.
    static DOT: Cell<u32> = const { Cell::new(0) };
}

/// Use the current buffer's history. If another buffer became current since our
/// last command, park the old buffer's history and pick up the new one's (a
/// buffer we haven't seen starts with none).
fn follow_buffer() {
    let current = ed::get_buffer_id();
    STATE.with(|s| {
        let state = &mut *s.borrow_mut();
        if state.buffer != current {
            let previous = std::mem::take(&mut state.history);
            state.parked.insert(state.buffer, previous);
            state.history = state.parked.remove(&current).unwrap_or_default();
            state.buffer = current;
        }
    });
}

/// Copy every dot's text into the register.
fn yank_all() {
    let texts: Vec<String> = ed::get_dots()
        .iter()
        .map(|dot| ed::slice(dot.anchor.min(dot.cursor), dot.anchor.max(dot.cursor) + 1))
        .collect();
    STATE.with(|s| s.borrow_mut().register = texts);
}

/// What to paste at the current dot: its own yanked text if there is one, or the
/// last yanked text (so one yank can be pasted at every dot).
fn register_for_dot() -> Option<String> {
    STATE.with(|s| {
        let register = &s.borrow().register;
        register
            .get(DOT.get() as usize)
            .or(register.last())
            .cloned()
    })
}

// ----- Commands -----

/// A command's implementation. The argument is the typed character for
/// `takes_char` commands, or the chosen value when a menu runs the command.
type Run = fn(Option<&str>);

/// The first character of an argument, for `takes_char` commands.
fn first_char(arg: Option<&str>) -> Option<char> {
    arg.and_then(|text| text.chars().next())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Run once at each dot (with `DOT` set to it).
    EachDot,
    /// Run once for the whole editor.
    Once,
}

struct Command {
    name: &'static str,
    description: &'static str,
    takes_char: bool,
    scope: Scope,
    /// Copy every dot's text into the register before running.
    yanks: bool,
    run: Run,
}

impl Command {
    const fn each(name: &'static str, description: &'static str, run: Run) -> Self {
        Self {
            name,
            description,
            takes_char: false,
            scope: Scope::EachDot,
            yanks: false,
            run,
        }
    }

    const fn once(name: &'static str, description: &'static str, run: Run) -> Self {
        Self {
            scope: Scope::Once,
            ..Self::each(name, description, run)
        }
    }

    const fn takes_char(mut self) -> Self {
        self.takes_char = true;
        self
    }

    const fn yanks(mut self) -> Self {
        self.yanks = true;
        self
    }
}

const COMMANDS: &[Command] = &[
    // The command menu, built on the host's generic menu
    Command::once("command-menu", "Pick a command to run", |_| open_command_menu()),
    Command::once("execute-command", "Run the command named by the argument", |arg| {
        if let Some(name) = arg {
            execute_command(name);
        }
    }),
    Command::once("describe-bindings", "List every key binding and what it does", |_| {
        open_bindings_menu()
    }),
    Command::once("show-message", "Show the argument in the status bar", |arg| {
        if let Some(text) = arg {
            ed::message(text);
        }
    }),
    // Movement
    Command::each("move-left", "Move each dot left", |_| move_horizontal(false, false)),
    Command::each("move-right", "Move each dot right", |_| move_horizontal(true, false)),
    Command::each("move-up", "Move each dot up", |_| move_vertical(false, false)),
    Command::each("move-down", "Move each dot down", |_| move_vertical(true, false)),
    Command::each("extend-left", "Extend each dot left", |_| move_horizontal(false, true)),
    Command::each("extend-right", "Extend each dot right", |_| move_horizontal(true, true)),
    Command::each("extend-up", "Extend each dot up", |_| move_vertical(false, true)),
    Command::each("extend-down", "Extend each dot down", |_| move_vertical(true, true)),
    // Goto
    Command::each("goto-line-start", "Go to the start of the line", |_| move_to(line_start(cursor()), false)),
    Command::each("goto-line-end", "Go to the last character of the line", |_| move_to(line_last_char(cursor()), false)),
    Command::each("goto-line-newline", "Go past the last character of the line", |_| move_to(line_end(cursor()), false)),
    Command::each("goto-first-non-blank", "Go to the first non-blank character", |_| move_to(first_non_blank(cursor()), false)),
    Command::each("goto-buffer-start", "Go to the start of the buffer", |_| move_to(0, false)),
    Command::each("goto-last-line", "Go to the start of the last line", |_| move_to(line_start(last()), false)),
    Command::each("goto-buffer-end", "Go to the end of the buffer", |_| move_to(last(), false)),
    Command::each("extend-to-line-start", "Extend to the start of the line", |_| move_to(line_start(cursor()), true)),
    Command::each("extend-to-line-end", "Extend to the last character of the line", |_| move_to(line_last_char(cursor()), true)),
    Command::each("extend-to-first-non-blank", "Extend to the first non-blank character", |_| move_to(first_non_blank(cursor()), true)),
    Command::each("extend-to-buffer-start", "Extend to the start of the buffer", |_| move_to(0, true)),
    Command::each("extend-to-last-line", "Extend to the start of the last line", |_| move_to(line_start(last()), true)),
    Command::each("extend-to-buffer-end", "Extend to the end of the buffer", |_| move_to(last(), true)),
    // Words
    Command::each("select-next-word", "Select the next word and trailing space", |_| select_word(Motion::Next, false, false)),
    Command::each("select-word-end", "Select to the end of the word", |_| select_word(Motion::End, false, false)),
    Command::each("select-previous-word", "Select back to the start of the word", |_| select_word(Motion::Previous, false, false)),
    Command::each("extend-next-word", "Extend to the next word", |_| select_word(Motion::Next, false, true)),
    Command::each("extend-word-end", "Extend to the end of the word", |_| select_word(Motion::End, false, true)),
    Command::each("extend-previous-word", "Extend back to the start of the word", |_| select_word(Motion::Previous, false, true)),
    Command::each("select-next-big-word", "Select the next WORD (whitespace separated)", |_| select_word(Motion::Next, true, false)),
    Command::each("select-big-word-end", "Select to the end of the WORD", |_| select_word(Motion::End, true, false)),
    Command::each("select-previous-big-word", "Select back to the start of the WORD", |_| select_word(Motion::Previous, true, false)),
    Command::each("extend-next-big-word", "Extend to the next WORD", |_| select_word(Motion::Next, true, true)),
    Command::each("extend-big-word-end", "Extend to the end of the WORD", |_| select_word(Motion::End, true, true)),
    Command::each("extend-previous-big-word", "Extend back to the start of the WORD", |_| select_word(Motion::Previous, true, true)),
    // Selections
    Command::each("select-line", "Select the whole line, or extend to the next line", |_| select_line()),
    Command::once("select-all", "Replace all dots with one covering the buffer", |_| {
        ed::set_dots(&[Selection {
            anchor: 0,
            cursor: last(),
        }]);
    }),
    Command::each("collapse-selection", "Shrink each dot to its cursor", |_| {
        let c = cursor();
        set_dot(c, c);
    }),
    Command::each("flip-selection", "Swap each dot's cursor and anchor", |_| {
        let dot = dot();
        set_dot(dot.cursor, dot.anchor);
    }),
    // Dots
    Command::once("spawn-dot", "Add a dot on the line below the newest one", |_| spawn_dot()),
    Command::once("remove-oldest-dot", "Remove the oldest dot", |_| {
        let dots = ed::get_dots();
        if dots.len() > 1 {
            ed::set_dots(&dots[1..]);
        } else {
            ed::message("only one dot");
        }
    }),
    // Insert mode
    Command::each("insert-before", "Insert before each dot", |_| start_insert(dot_start())),
    Command::each("insert-after", "Insert after each dot", |_| start_insert((dot_end() + 1).min(last()))),
    Command::each("insert-at-line-start", "Insert at the first non-blank of the line", |_| start_insert(first_non_blank(dot_start()))),
    Command::each("insert-at-line-end", "Insert at the end of the line", |_| start_insert(line_end(dot_end()))),
    Command::each("open-line-below", "Open a new line below and insert", |_| open_line(true)),
    Command::each("open-line-above", "Open a new line above and insert", |_| open_line(false)),
    Command::each("change-selection", "Delete each dot's text and insert", |_| {
        let start = delete_dot_text();
        start_insert(start);
    })
    .yanks(),
    Command::each("normal-mode", "Return to normal mode", |_| {
        let _ = ed::set_mode("normal");
        let c = cursor().min(last());
        set_dot(c, c);
    }),
    Command::each("insert-char", "Insert the typed character", |arg| {
        if let Some(c) = first_char(arg) {
            insert_at_cursor(&c.to_string());
        }
    })
    .takes_char(),
    Command::each("insert-newline", "Insert a newline, keeping indentation", |_| {
        let indent = leading_blanks(cursor());
        insert_at_cursor(&format!("\n{indent}"));
    }),
    Command::each("insert-tab", "Insert indentation", |_| insert_at_cursor(INDENT)),
    Command::each("delete-char-before", "Delete the character before each dot", |_| {
        let c = cursor();
        if c > 0 {
            edit(c - 1, c, "");
            set_dot(c - 1, c - 1);
        }
    }),
    Command::each("delete-char-after", "Delete the character after each dot", |_| {
        let c = cursor();
        if c < last() {
            edit(c, c + 1, "");
        }
    }),
    // Editing
    Command::each("delete-selection", "Yank and delete each dot's text", |_| {
        delete_dot_text();
    })
    .yanks(),
    Command::each("delete-selection-no-yank", "Delete each dot's text without yanking", |_| {
        delete_dot_text();
    }),
    Command::once("yank", "Copy each dot's text", |_| {
        let count: usize = STATE.with(|s| s.borrow().register.len());
        let chars: usize = STATE.with(|s| s.borrow().register.iter().map(|t| t.chars().count()).sum());
        ed::message(&if count == 1 {
            format!("yanked {chars} chars")
        } else {
            format!("yanked {count} selections ({chars} chars)")
        });
    })
    .yanks(),
    Command::each("paste-after", "Paste after each dot", |_| paste(true)),
    Command::each("paste-before", "Paste before each dot", |_| paste(false)),
    Command::each("replace-with-yank", "Replace each dot's text with the yanked text", |_| {
        let Some(text) = register_for_dot() else {
            ed::message("register is empty");
            return;
        };
        let start = dot_start();
        edit(start, dot_end() + 1, &text);
        set_dot(start, start + char_count(&text).max(1) - 1);
    }),
    Command::each("replace-chars", "Replace each selected character", |arg| {
        if let Some(c) = first_char(arg) {
            map_dot_text(|x| if x == '\n' { x } else { c });
        }
    })
    .takes_char(),
    Command::each("uppercase", "Uppercase each dot's text", |_| map_dot_text(upper)),
    Command::each("lowercase", "Lowercase each dot's text", |_| map_dot_text(lower)),
    Command::each("swap-case", "Swap the case of each dot's text", |_| {
        map_dot_text(|c| if c.is_uppercase() { lower(c) } else { upper(c) })
    }),
    Command::each("indent", "Indent the lines under each dot", |_| indent(true)),
    Command::each("dedent", "Dedent the lines under each dot", |_| indent(false)),
    Command::once("undo", "Undo the last change", |_| {
        if !step_history(true) {
            ed::message("nothing left to undo");
        }
    }),
    Command::once("redo", "Redo the last undone change", |_| {
        if !step_history(false) {
            ed::message("nothing left to redo");
        }
    }),
];

const NORMAL_BINDINGS: &[(&str, &str)] = &[
    // The command menu is part of the app; the key that opens it is ours.
    (":", "command-menu"),
    ("<a-x>", "command-menu"),
    ("?", "describe-bindings"),
    // Dots
    ("c", "spawn-dot"),
    (",", "remove-oldest-dot"),
    // Movement
    ("h", "move-left"),
    ("j", "move-down"),
    ("k", "move-up"),
    ("l", "move-right"),
    ("H", "extend-left"),
    ("J", "extend-down"),
    ("K", "extend-up"),
    ("L", "extend-right"),
    ("<left>", "move-left"),
    ("<down>", "move-down"),
    ("<up>", "move-up"),
    ("<right>", "move-right"),
    ("<s-left>", "extend-left"),
    ("<s-down>", "extend-down"),
    ("<s-up>", "extend-up"),
    ("<s-right>", "extend-right"),
    ("<home>", "goto-line-start"),
    ("<end>", "goto-line-end"),
    ("<s-home>", "extend-to-line-start"),
    ("<s-end>", "extend-to-line-end"),
    // Words
    ("w", "select-next-word"),
    ("e", "select-word-end"),
    ("b", "select-previous-word"),
    ("W", "extend-next-word"),
    ("E", "extend-word-end"),
    ("B", "extend-previous-word"),
    ("<a-w>", "select-next-big-word"),
    ("<a-e>", "select-big-word-end"),
    ("<a-b>", "select-previous-big-word"),
    ("<a-W>", "extend-next-big-word"),
    ("<a-E>", "extend-big-word-end"),
    ("<a-B>", "extend-previous-big-word"),
    // Selections
    ("x", "select-line"),
    ("%", "select-all"),
    (";", "collapse-selection"),
    ("<a-;>", "flip-selection"),
    // Goto
    ("gh", "goto-line-start"),
    ("gl", "goto-line-end"),
    ("gi", "goto-first-non-blank"),
    ("gg", "goto-buffer-start"),
    ("gk", "goto-buffer-start"),
    ("gj", "goto-last-line"),
    ("ge", "goto-buffer-end"),
    ("Gh", "extend-to-line-start"),
    ("Gl", "extend-to-line-end"),
    ("Gi", "extend-to-first-non-blank"),
    ("Gg", "extend-to-buffer-start"),
    ("Gk", "extend-to-buffer-start"),
    ("Gj", "extend-to-last-line"),
    ("Ge", "extend-to-buffer-end"),
    // Insert mode
    ("i", "insert-before"),
    ("a", "insert-after"),
    ("I", "insert-at-line-start"),
    ("A", "insert-at-line-end"),
    ("o", "open-line-below"),
    ("O", "open-line-above"),
    ("s", "change-selection"),
    // Editing
    ("d", "delete-selection"),
    ("<a-d>", "delete-selection-no-yank"),
    ("y", "yank"),
    ("p", "paste-after"),
    ("P", "paste-before"),
    ("R", "replace-with-yank"),
    ("r", "replace-chars"),
    ("~", "uppercase"),
    ("`", "lowercase"),
    ("<a-`>", "swap-case"),
    (">", "indent"),
    ("<lt>", "dedent"),
    ("u", "undo"),
    ("U", "redo"),
    // The host's own commands: plugins can bind them, but only the host writes
    // files or runs the terminal.
    ("<c-s>", "write"),
    ("<c-t>", "terminal"),
    ("<a-t>", "terminal-toggle"),
];

const INSERT_BINDINGS: &[(&str, &str)] = &[
    ("<esc>", "normal-mode"),
    ("<ret>", "insert-newline"),
    ("<tab>", "insert-tab"),
    ("<backspace>", "delete-char-before"),
    ("<del>", "delete-char-after"),
    ("<left>", "move-left"),
    ("<right>", "move-right"),
    ("<up>", "move-up"),
    ("<down>", "move-down"),
    ("<home>", "goto-line-start"),
    ("<end>", "goto-line-newline"),
    ("<c-s>", "write"),
    ("<c-t>", "terminal"),
    ("<a-t>", "terminal-toggle"),
];

const INDENT: &str = "    ";

// ----- The command menu -----

/// Open the host's generic menu listing every command: recently run ones first,
/// then shorter names, then alphabetically. Choosing one runs `execute-command`.
fn open_command_menu() {
    let recent = STATE.with(|s| s.borrow().recent_commands.clone());
    let mut commands: Vec<_> = reg::list_commands()
        .into_iter()
        .filter(|command| command.name != "execute-command")
        .collect();
    commands.sort_by_key(|command| {
        let rank = recent
            .iter()
            .position(|name| *name == command.name)
            .unwrap_or(usize::MAX);
        (rank, command.name.len(), command.name.clone())
    });
    let items = commands
        .into_iter()
        .map(|command| {
            // Name the plugin, except for this one, which provides the basics.
            let detail = if command.plugin == "core" {
                command.description
            } else {
                format!("{} ({})", command.description, command.plugin)
            };
            MenuItem {
                label: command.name.clone(),
                value: command.name,
                annotation: command.binding.unwrap_or_default(),
                detail,
            }
        })
        .collect();
    ui::open_menu(&Menu {
        prompt: "M-x".to_string(),
        input: String::new(),
        items,
        on_select: "execute-command".to_string(),
    });
}

/// Open the host's generic menu listing every key binding: normal mode first,
/// then other modes, each in the order the bindings were made. Rows read
/// "keys  command" so either can be searched; choosing one explains it.
fn open_bindings_menu() {
    let commands = reg::list_commands();
    let mut bindings = reg::list_bindings();
    // Stable sort: keeps the binding order within each mode.
    bindings.sort_by_key(|binding| binding.mode != "normal");
    let width = bindings
        .iter()
        .map(|binding| binding.keys.chars().count())
        .max()
        .unwrap_or(0);

    let items = bindings
        .into_iter()
        .map(|binding| {
            let command = commands.iter().find(|c| c.name == binding.command);
            let description = match command {
                Some(c) if c.plugin == "core" => c.description.clone(),
                Some(c) => format!("{} ({})", c.description, c.plugin),
                None => "unknown command".to_string(),
            };
            MenuItem {
                label: format!("{:<width$}  {}", binding.keys, binding.command),
                value: format!(
                    "{} in {} mode runs {}: {}",
                    binding.keys, binding.mode, binding.command, description
                ),
                annotation: binding.mode,
                detail: description,
            }
        })
        .collect();
    ui::open_menu(&Menu {
        prompt: "keys".to_string(),
        input: String::new(),
        items,
        on_select: "show-message".to_string(),
    });
}

/// Run the command chosen in the command menu, remembering it as recent.
fn execute_command(name: &str) {
    STATE.with(|s| {
        let recent = &mut s.borrow_mut().recent_commands;
        recent.retain(|entry| entry != name);
        recent.insert(0, name.to_string());
    });
    if !ed::run_command(name, None) {
        ed::message(&format!("no such command: {name}"));
    }
}

// ----- Editing, merging and undo: the behaviour the host leaves to us -----

/// Replace `start..end` (end exclusive) with `text`, then shift every dot so it
/// keeps pointing at the same text: dots after the change move by its size, and
/// dot ends inside the replaced range move to its start. Keeps the final newline.
fn edit(start: u32, end: u32, text: &str) {
    let end = end.min(ed::len());
    let start = start.min(end);
    let inserted = char_count(text);
    ed::replace(start, end, text);
    let shift = |pos: u32| {
        if pos >= end {
            pos - (end - start) + inserted
        } else if pos >= start {
            start
        } else {
            pos
        }
    };
    let dots: Vec<Selection> = ed::get_dots()
        .iter()
        .map(|dot| Selection {
            anchor: shift(dot.anchor),
            cursor: shift(dot.cursor),
        })
        .collect();
    keep_final_newline();
    ed::set_dots(&dots);
}

/// Like Kakoune, the buffer always ends with a newline.
fn keep_final_newline() {
    let len = ed::len();
    if len == 0 || ed::char_at(len - 1) != Some('\n') {
        ed::replace(len, len, "\n");
    }
}

/// Merge dots that overlap into the older one, keeping its direction.
fn merge_dots() {
    let mut merged: Vec<Selection> = Vec::new();
    for dot in ed::get_dots() {
        let (start, end) = (dot.anchor.min(dot.cursor), dot.anchor.max(dot.cursor));
        let overlapping = merged.iter_mut().find(|kept| {
            kept.anchor.min(kept.cursor) <= end && start <= kept.anchor.max(kept.cursor)
        });
        match overlapping {
            Some(kept) => {
                let forward = kept.anchor <= kept.cursor;
                let start = start.min(kept.anchor.min(kept.cursor));
                let end = end.max(kept.anchor.max(kept.cursor));
                *kept = if forward {
                    Selection { anchor: start, cursor: end }
                } else {
                    Selection { anchor: end, cursor: start }
                };
            }
            None => merged.push(dot),
        }
    }
    if merged != ed::get_dots() {
        ed::set_dots(&merged);
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        text: ed::slice(0, ed::len()),
        dots: ed::get_dots(),
    }
}

fn restore(snapshot: &Snapshot) {
    ed::replace(0, ed::len(), &snapshot.text);
    ed::set_dots(&snapshot.dots);
}

/// Open an undo group before a command, unless one is already open (as it is
/// throughout an insert session). Edits other plugins made since our last command
/// become their own undo step first.
fn begin_undo_group() {
    let now = snapshot();
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if state.history.group.is_none() {
            if let Some(known) = state.history.known.take()
                && known.text != now.text
            {
                state.history.undo.push(known);
                state.history.redo.clear();
            }
            state.history.group = Some(now);
        }
    });
}

/// Close the group after a command, except in insert mode, so a whole insert
/// session undoes as one step.
fn end_undo_group() {
    if ed::get_mode() != "insert" {
        close_undo_group();
    }
}

fn close_undo_group() {
    let text = ed::slice(0, ed::len());
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if let Some(group) = state.history.group.take()
            && group.text != text
        {
            state.history.undo.push(group);
            state.history.redo.clear();
        }
    });
}

fn remember_state() {
    let now = snapshot();
    STATE.with(|s| s.borrow_mut().history.known = Some(now));
}

/// Undo (or redo) one step. Returns false if there is nothing to step to.
fn step_history(undo: bool) -> bool {
    // Count edits made by other plugins, and an open group, as steps first.
    begin_undo_group();
    close_undo_group();
    let current = snapshot();
    let target = STATE.with(|s| {
        let state = &mut *s.borrow_mut();
        let (from, to) = if undo {
            (&mut state.history.undo, &mut state.history.redo)
        } else {
            (&mut state.history.redo, &mut state.history.undo)
        };
        let target = from.pop()?;
        to.push(current);
        Some(target)
    });
    match target {
        Some(target) => {
            restore(&target);
            true
        }
        None => false,
    }
}

// ----- Dot and buffer helpers over the host API -----

fn dot_count() -> u32 {
    ed::get_dots().len() as u32
}

/// The dot the current command is working on.
fn dot() -> Selection {
    ed::get_dot(DOT.get()).unwrap_or_else(ed::get_selection)
}

fn set_dot(anchor: u32, cursor: u32) {
    ed::set_dot(DOT.get(), Selection { anchor, cursor });
}

fn cursor() -> u32 {
    dot().cursor
}

fn dot_start() -> u32 {
    let dot = dot();
    dot.anchor.min(dot.cursor)
}

/// Inclusive end of the dot.
fn dot_end() -> u32 {
    let dot = dot();
    dot.anchor.max(dot.cursor)
}

fn last() -> u32 {
    ed::len() - 1
}

fn char_at(index: u32) -> char {
    ed::char_at(index).unwrap_or('\n')
}

fn char_count(text: &str) -> u32 {
    text.chars().count() as u32
}

fn line_start(index: u32) -> u32 {
    let mut start = index;
    while start > 0 && char_at(start - 1) != '\n' {
        start -= 1;
    }
    start
}

/// Index of the line's newline.
fn line_end(index: u32) -> u32 {
    let mut end = index;
    while end < last() && char_at(end) != '\n' {
        end += 1;
    }
    end
}

/// Last character before the newline, or the newline on an empty line.
fn line_last_char(index: u32) -> u32 {
    let start = line_start(index);
    let end = line_end(index);
    if end > start { end - 1 } else { end }
}

/// Start of the line `lines` below the one containing `index`, if it exists.
fn line_below(index: u32, lines: u32) -> Option<u32> {
    let mut start = line_start(index);
    for _ in 0..lines {
        let end = line_end(start);
        if end + 1 >= ed::len() {
            return None;
        }
        start = end + 1;
    }
    Some(start)
}

/// Number of line breaks between two positions.
fn lines_between(from: u32, to: u32) -> u32 {
    (from..to).filter(|&i| char_at(i) == '\n').count() as u32
}

fn first_non_blank(index: u32) -> u32 {
    let mut pos = line_start(index);
    loop {
        let c = char_at(pos);
        if c == '\n' || !c.is_whitespace() {
            return pos;
        }
        pos += 1;
    }
}

fn leading_blanks(index: u32) -> String {
    ed::slice(line_start(index), first_non_blank(index))
}

fn column(index: u32) -> u32 {
    index - line_start(index)
}

// ----- Dots -----

/// Copy the newest dot onto the lines just below it, keeping its columns (like
/// Kakoune's `C`). The new dot becomes the newest.
fn spawn_dot() {
    let newest = ed::get_selection();
    let (start, end) = (newest.anchor.min(newest.cursor), newest.anchor.max(newest.cursor));
    // Place the copy entirely below the original.
    let offset = lines_between(start, end) + 1;
    let place = |pos: u32| {
        line_below(pos, offset).map(|line| (line + column(pos)).min(line_end(line)))
    };
    match (place(newest.anchor), place(newest.cursor)) {
        (Some(anchor), Some(cursor)) => {
            let mut dots = ed::get_dots();
            dots.push(Selection { anchor, cursor });
            ed::set_dots(&dots);
        }
        _ => ed::message("no line below for a new dot"),
    }
}

// ----- Movement -----

fn move_to(pos: u32, extend: bool) {
    if extend {
        set_dot(dot().anchor, pos);
    } else {
        set_dot(pos, pos);
    }
}

fn move_horizontal(forward: bool, extend: bool) {
    let c = cursor();
    let pos = if forward {
        (c + 1).min(last())
    } else {
        c.saturating_sub(1)
    };
    move_to(pos, extend);
}

fn move_vertical(down: bool, extend: bool) {
    let index = DOT.get() as usize;
    let from = cursor();
    let goal = STATE.with(|s| s.borrow().history.goals.get(index).copied().flatten());
    let column = match goal {
        Some((column, at)) if at == from => column,
        _ => column(from),
    };
    let target_start = if down {
        line_below(from, 1)
    } else {
        let start = line_start(from);
        (start > 0).then(|| line_start(start - 1))
    };
    let pos = match target_start {
        Some(start) => (start + column).min(line_end(start)),
        None => from,
    };
    move_to(pos, extend);
    STATE.with(|s| {
        let goals = &mut s.borrow_mut().history.goals;
        if goals.len() <= index {
            goals.resize(index + 1, None);
        }
        goals[index] = Some((column, pos));
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Eol,
    Blank,
    Word,
    Punct,
}

#[derive(Clone, Copy)]
enum Motion {
    Next,
    End,
    Previous,
}

fn class(index: u32, big: bool) -> Class {
    let c = char_at(index);
    if c == '\n' {
        Class::Eol
    } else if c.is_whitespace() {
        Class::Blank
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// Start point for a forward word motion: step over a class boundary and newlines.
fn forward_origin(big: bool) -> Option<u32> {
    let len = ed::len();
    let mut start = cursor();
    if start + 1 >= len {
        return None;
    }
    if class(start, big) != class(start + 1, big) {
        start += 1;
    }
    while start + 1 < len && char_at(start) == '\n' {
        start += 1;
    }
    Some(start)
}

fn select_word(motion: Motion, big: bool, extend: bool) {
    let len = ed::len();
    let found = match motion {
        // The word and the whitespace after it.
        Motion::Next => forward_origin(big).map(|start| {
            let mut end = start;
            let c = class(start, big);
            if c != Class::Blank && c != Class::Eol {
                while end + 1 < len && class(end + 1, big) == c {
                    end += 1;
                }
            }
            while end + 1 < len && class(end + 1, big) == Class::Blank {
                end += 1;
            }
            (start, end)
        }),
        // Leading whitespace and the word up to its end.
        Motion::End => forward_origin(big).map(|start| {
            let mut end = start;
            while end + 1 < len && class(end, big) == Class::Blank {
                end += 1;
            }
            let c = class(end, big);
            if c != Class::Eol {
                while end + 1 < len && class(end + 1, big) == c {
                    end += 1;
                }
            }
            (start, end)
        }),
        // Backwards to the start of the previous word.
        Motion::Previous => {
            let mut start = cursor();
            (start > 0).then(|| {
                if class(start, big) != class(start - 1, big) {
                    start -= 1;
                }
                while start > 0 && char_at(start) == '\n' {
                    start -= 1;
                }
                let mut end = start;
                while end > 0 && class(end, big) == Class::Blank {
                    end -= 1;
                }
                let c = class(end, big);
                if c != Class::Eol {
                    while end > 0 && class(end - 1, big) == c {
                        end -= 1;
                    }
                }
                (start, end)
            })
        }
    };
    if let Some((anchor, cursor)) = found {
        if extend {
            move_to(cursor, true);
        } else {
            set_dot(anchor, cursor);
        }
    }
}

fn select_line() {
    let (dot_start, dot_end) = (dot_start(), dot_end());
    let start = line_start(dot_start);
    let mut end = line_end(dot_end);
    if dot_start == start && dot_end == end && end + 1 < ed::len() {
        end = line_end(end + 1);
    }
    set_dot(start, end);
}

// ----- Editing -----

fn start_insert(pos: u32) {
    let _ = ed::set_mode("insert");
    set_dot(pos, pos);
}

fn insert_at_cursor(text: &str) {
    let pos = cursor();
    edit(pos, pos, text);
    let after = pos + char_count(text);
    set_dot(after, after);
}

/// Remove the current dot's text and return where it started.
fn delete_dot_text() -> u32 {
    let start = dot_start();
    edit(start, dot_end() + 1, "");
    let start = start.min(last());
    set_dot(start, start);
    start
}

fn open_line(below: bool) {
    let indent = leading_blanks(cursor());
    let indent_len = char_count(&indent);
    let pos = if below {
        let end = line_end(dot_end());
        edit(end, end, &format!("\n{indent}"));
        end + 1 + indent_len
    } else {
        let start = line_start(dot_start());
        edit(start, start, &format!("{indent}\n"));
        start + indent_len
    };
    start_insert(pos);
}

fn paste(after: bool) {
    let Some(text) = register_for_dot() else {
        ed::message("register is empty");
        return;
    };
    let linewise = text.ends_with('\n');
    let pos = match (after, linewise) {
        (true, true) => line_end(dot_end()) + 1,
        (true, false) => (dot_end() + 1).min(last()),
        (false, true) => line_start(dot_start()),
        (false, false) => dot_start(),
    };
    edit(pos, pos, &text);
    set_dot(pos, pos + char_count(&text).max(1) - 1);
}

/// Replace every character of the current dot, keeping the dot.
fn map_dot_text(f: impl Fn(char) -> char) {
    let dot = dot();
    let (start, end) = (dot_start(), dot_end() + 1);
    let mapped: String = ed::slice(start, end).chars().map(f).collect();
    edit(start, end, &mapped);
    set_dot(dot.anchor, dot.cursor);
}

fn upper(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Indent or dedent every line touched by the current dot.
fn indent(indent: bool) {
    let first = line_start(dot_start());
    let dot_end = dot_end();
    let mut starts = Vec::new();
    let mut line = first;
    loop {
        starts.push(line);
        let end = line_end(line);
        if end >= dot_end || end + 1 >= ed::len() {
            break;
        }
        line = end + 1;
    }
    // Walk backwards so earlier line starts stay valid.
    for &start in starts.iter().rev() {
        if indent {
            if char_at(start) != '\n' {
                edit(start, start, INDENT);
            }
        } else {
            let spaces = (start..start + INDENT.len() as u32)
                .take_while(|&i| char_at(i) == ' ')
                .count() as u32;
            edit(start, start + spaces, "");
        }
    }
    let mut end = line_end(first);
    for _ in 1..starts.len() {
        end = line_end(end + 1);
    }
    set_dot(first, end);
}
