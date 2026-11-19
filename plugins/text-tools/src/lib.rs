//! Example Arrow plugin. It adds a few text commands to the command menu, binds
//! keys to them (and to a built-in command), and uses event hooks to keep stats.
//!
//! Build it with `cargo build --release --target wasm32-wasip2`. Arrow's build
//! script does this automatically and bundles the result.

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "plugin",
    path: "../../wit",
});

use arrow::plugin::editor as ed;
use arrow::plugin::registry as reg;
// `Event` is already in scope: the world `use`s it.
use arrow::plugin::types::{EventKind, Selection};

/// (name, description, implementation)
const COMMANDS: &[(&str, &str, fn())] = &[
    ("sort-lines", "Sort the selected lines", sort_lines),
    ("duplicate-line", "Duplicate the cursor's line below it", duplicate_line),
    ("count-words", "Count words in the selection, or the buffer", count_words),
    ("plugin-stats", "Show what text-tools' event hooks have seen", plugin_stats),
];

const BINDINGS: &[(&str, &str)] = &[
    ("<a-s>", "sort-lines"),
    ("<space>d", "duplicate-line"),
    ("<space>c", "count-words"),
    // Plugins can bind other plugins' commands too (this one is from core).
    ("<space>a", "select-all"),
];

#[derive(Default)]
struct Stats {
    buffer_changes: u32,
    selection_changes: u32,
    mode_changes: u32,
    commands: u32,
    last_command: Option<String>,
}

thread_local! {
    static STATS: RefCell<Stats> = RefCell::new(Stats::default());
}

struct TextTools;

impl Guest for TextTools {
    fn init() {
        for (name, description, _) in COMMANDS {
            reg::register_command(name, description, false);
        }
        for (keys, command) in BINDINGS {
            if let Err(error) = reg::bind_key("normal", keys, command) {
                ed::message(&format!("text-tools: {error}"));
            }
        }
        for kind in [
            EventKind::Startup,
            EventKind::BufferChanged,
            EventKind::SelectionChanged,
            EventKind::ModeChanged,
            EventKind::CommandExecuted,
        ] {
            reg::subscribe(kind);
        }
    }

    fn run_command(name: String, _arg: Option<char>) {
        match COMMANDS.iter().find(|(n, ..)| *n == name) {
            Some((.., run)) => run(),
            None => ed::message(&format!("text-tools has no command `{name}`")),
        }
    }

    fn on_event(event: Event) {
        STATS.with(|stats| {
            let mut stats = stats.borrow_mut();
            match event {
                Event::Startup => {
                    ed::message("text-tools loaded: try :sort-lines, <space>d or <space>c")
                }
                Event::BufferChanged => stats.buffer_changes += 1,
                Event::SelectionChanged(_) => stats.selection_changes += 1,
                Event::ModeChanged(_) => stats.mode_changes += 1,
                Event::CommandExecuted(name) => {
                    stats.commands += 1;
                    stats.last_command = Some(name);
                }
            }
        });
    }
}

export!(TextTools);

// ----- Helpers over the host API -----

fn selection_range() -> (u32, u32) {
    let s = ed::get_selection();
    (s.anchor.min(s.cursor), s.anchor.max(s.cursor))
}

fn line_start(index: u32) -> u32 {
    let mut start = index;
    while start > 0 && ed::char_at(start - 1) != Some('\n') {
        start -= 1;
    }
    start
}

/// Index of the line's newline.
fn line_end(index: u32) -> u32 {
    let mut end = index;
    while ed::char_at(end).is_some_and(|c| c != '\n') {
        end += 1;
    }
    end
}

fn select(anchor: u32, cursor: u32) {
    ed::set_selection(Selection { anchor, cursor });
}

/// Replace `start..end` with `text`. The host only stores dots, so shift the
/// ones after the change ourselves to keep them on the same text.
fn edit(start: u32, end: u32, text: &str) {
    let inserted = text.chars().count() as u32;
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
    ed::set_dots(&dots);
}

// ----- Commands -----

fn sort_lines() {
    let (start, end) = selection_range();
    let (start, end) = (line_start(start), line_end(end));
    let text = ed::slice(start, end + 1);
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    let sorted = lines.join("\n") + "\n";
    edit(start, end + 1, &sorted);
    select(start, start + sorted.chars().count() as u32 - 1);
}

fn duplicate_line() {
    let cursor = ed::get_selection().cursor;
    let (start, end) = (line_start(cursor), line_end(cursor));
    let line = ed::slice(start, end + 1);
    edit(end + 1, end + 1, &line);
    let new_start = end + 1;
    select(new_start, new_start + line.chars().count() as u32 - 1);
}

fn count_words() {
    let (start, end) = selection_range();
    let (text, scope) = if end > start {
        (ed::slice(start, end + 1), "selection")
    } else {
        (ed::slice(0, ed::len()), "buffer")
    };
    ed::message(&format!(
        "{scope}: {} words, {} lines, {} chars",
        text.split_whitespace().count(),
        text.lines().count(),
        text.chars().count(),
    ));
}

fn plugin_stats() {
    STATS.with(|stats| {
        let stats = stats.borrow();
        ed::message(&format!(
            "seen: {} commands (last: {}), {} buffer changes, {} selection changes, {} mode changes",
            stats.commands,
            stats.last_command.as_deref().unwrap_or("none"),
            stats.buffer_changes,
            stats.selection_changes,
            stats.mode_changes,
        ));
    });
}
