//! `git status` in a menu: `<space>g` (or `:git-status`) lists the files with
//! changes, and choosing one opens it.
//!
//! This is also the example of running a program from a plugin, through the
//! host's `process` interface (see `wit/arrow.wit`):
//!
//! 1. `process::spawn` starts the program and returns an id. The plugin has to
//!    be allowed to run it; the host allows this bundled plugin `git`, and
//!    other plugins are allowed programs in `plugins.toml`.
//! 2. The command returns straight away. Plugins only run when the host calls
//!    them, so the program's output comes later, as `process-output` events,
//!    sent only to the plugin that started it (no subscription needed).
//! 3. One `process-exited` event follows the last output. Here, that is when
//!    the collected output is turned into a menu.
//!
//! A longer conversation with a program, as a proof assistant needs, works the
//! same way: keep the id, `process::write` each request to its stdin, and
//! handle the replies as they arrive in `on-event`.

use std::cell::RefCell;
use std::collections::HashMap;

wit_bindgen::generate!({
    world: "plugin",
    path: "../../wit",
});

use arrow::plugin::editor as ed;
use arrow::plugin::process::{self, Command};
use arrow::plugin::registry as reg;
use arrow::plugin::types::{Menu, MenuItem, OutputStream};
use arrow::plugin::ui;

/// What a running `git status` has said so far.
#[derive(Default)]
struct Run {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

thread_local! {
    /// Runs in progress, by process id. Plugin state lasts as long as the
    /// plugin, so it's here when the program's output arrives.
    static RUNS: RefCell<HashMap<u32, Run>> = RefCell::new(HashMap::new());
}

struct Git;

impl Guest for Git {
    fn init() {
        reg::register_command(
            "git-status",
            "List files with changes (git status) and open the one chosen",
            false,
        );
        if let Err(error) = reg::bind_key("normal", "<space>g", "git-status") {
            ed::message(&format!("git: {error}"));
        }
    }

    fn run_command(name: String, _arg: Option<String>) {
        if name == "git-status" {
            start_status();
        }
    }

    fn on_event(event: Event) {
        match event {
            Event::ProcessOutput(output) => RUNS.with(|runs| {
                if let Some(run) = runs.borrow_mut().get_mut(&output.id) {
                    match output.stream {
                        OutputStream::Stdout => run.stdout.extend(output.data),
                        OutputStream::Stderr => run.stderr.extend(output.data),
                    }
                }
            }),
            Event::ProcessExited(exit) => {
                let Some(run) = RUNS.with(|runs| runs.borrow_mut().remove(&exit.id)) else {
                    return;
                };
                if exit.code == Some(0) {
                    show_changes(&String::from_utf8_lossy(&run.stdout));
                } else {
                    let error = String::from_utf8_lossy(&run.stderr);
                    let first = error.lines().next().unwrap_or("git status failed");
                    ed::message(&format!("git: {first}"));
                }
            }
            _ => {}
        }
    }
}

export!(Git);

fn start_status() {
    let command = Command {
        program: "git".to_string(),
        // `--short` paths are relative to the working directory, which is where
        // the file manager opens files from.
        args: ["-c", "color.status=false", "status", "--short", "--untracked-files=all"]
            .map(String::from)
            .to_vec(),
        cwd: None,
        env: Vec::new(),
    };
    match process::spawn(&command) {
        Ok(id) => {
            RUNS.with(|runs| runs.borrow_mut().insert(id, Run::default()));
            ed::message("git status…");
        }
        Err(error) => ed::message(&format!("git: {error}")),
    }
}

/// Open a menu of the changed files. Choosing one runs the files plugin's
/// `open-file` with its path.
fn show_changes(status: &str) {
    let items: Vec<MenuItem> = status.lines().filter_map(change).collect();
    if items.is_empty() {
        ed::message("git: nothing changed");
        return;
    }
    ed::message(&format!("git: {} changed", items.len()));
    ui::open_menu(&Menu {
        prompt: "git".to_string(),
        input: String::new(),
        items,
        on_select: "open-file".to_string(),
    });
}

/// One line of `git status --short`: two status letters, a space, the path
/// (or `old -> new` for a rename).
fn change(line: &str) -> Option<MenuItem> {
    let code = line.get(..2)?;
    let path = line.get(3..)?;
    let path = path.rsplit(" -> ").next()?.trim_matches('"');
    let detail = match code.trim() {
        "M" | "MM" | "AM" => "modified",
        "A" => "added",
        "D" => "deleted",
        "R" | "RM" => "renamed",
        "??" => "untracked",
        "UU" | "AA" | "DD" => "conflict",
        _ => "changed",
    };
    Some(MenuItem {
        label: path.to_string(),
        value: path.to_string(),
        annotation: code.trim().to_string(),
        detail: detail.to_string(),
    })
}
