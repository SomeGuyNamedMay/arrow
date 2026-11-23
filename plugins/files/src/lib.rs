//! A file manager inspired by ranger, built on the host's tree menu, and a list
//! of the open buffers to switch between.
//!
//! `<space>f` (or `:file-manager`) opens the directory Arrow was started in. The
//! tree is loaded lazily: the host asks for a directory's contents the first time
//! it is highlighted or entered. Its first entry, "open buffers", lists the
//! buffers that are open. Choosing a file opens it in a buffer of its own, or
//! switches to its buffer if it is open already, unsaved edits and all.
//!
//! `<space>b` (or `:buffers`) shows just the open buffers, and `<space>k` (or
//! `:close-buffer`) closes the current one, unless it has unsaved changes:
//! `:write` it first (the host's command, `<c-s>`), or `:discard-buffer`.
//!
//! The host gives this plugin, and only this one, a read-only view of the working
//! directory as ".", so ordinary `std::fs` calls work here.

use std::fs;

wit_bindgen::generate!({
    world: "plugin",
    path: "../../wit",
});

use arrow::plugin::editor as ed;
use arrow::plugin::registry as reg;
use arrow::plugin::types::{BufferInfo, TreeMenu, TreeNode};
use arrow::plugin::ui;

/// Value of the file manager's "open buffers" branch. File paths are relative,
/// so no file's value starts with "/".
const BUFFERS: &str = "/buffers";

struct Files;

impl Guest for Files {
    fn init() {
        reg::register_command("file-manager", "Browse files, ranger style", false);
        reg::register_command("buffers", "Switch to an open buffer", false);
        reg::register_command(
            "list-directory",
            "List a directory in the file manager (used by it)",
            false,
        );
        reg::register_command(
            "file-manager-choose",
            "Open a file or switch to a buffer chosen in the file manager (used by it)",
            false,
        );
        reg::register_command("open-file", "Open the file named by the argument", false);
        reg::register_command(
            "switch-buffer",
            "Switch to the buffer whose id is the argument",
            false,
        );
        reg::register_command(
            "close-buffer",
            "Close the current buffer, unless it has unsaved changes",
            false,
        );
        reg::register_command(
            "discard-buffer",
            "Close the current buffer, throwing away unsaved changes",
            false,
        );
        let bindings = [
            ("<space>f", "file-manager"),
            ("<space>b", "buffers"),
            ("<space>k", "close-buffer"),
        ];
        for (keys, command) in bindings {
            if let Err(error) = reg::bind_key("normal", keys, command) {
                ed::message(&format!("files: {error}"));
            }
        }
    }

    fn run_command(name: String, arg: Option<String>) {
        match (name.as_str(), arg) {
            ("file-manager", _) => {
                let mut nodes = buffer_nodes(Some(BUFFERS));
                nodes.insert(0, buffers_branch());
                nodes.extend(list(None));
                ui::open_tree_menu(&TreeMenu {
                    title: "files".to_string(),
                    nodes,
                    on_select: "file-manager-choose".to_string(),
                    on_expand: Some("list-directory".to_string()),
                });
            }
            ("buffers", _) => ui::open_tree_menu(&TreeMenu {
                title: "buffers".to_string(),
                nodes: buffer_nodes(None),
                on_select: "switch-buffer".to_string(),
                on_expand: None,
            }),
            ("list-directory", Some(dir)) => ui::add_tree_nodes(Some(&dir), &list(Some(&dir))),
            ("file-manager-choose", Some(value)) => match value.strip_prefix("/buffers/") {
                Some(id) => switch(id),
                None => open(&value),
            },
            ("open-file", Some(path)) => open(&path),
            ("switch-buffer", Some(id)) => switch(&id),
            ("close-buffer", _) => close(false),
            ("discard-buffer", _) => close(true),
            _ => ed::message(&format!("files: `{name}` needs an argument")),
        }
    }

    fn on_event(_event: Event) {}
}

export!(Files);

// ----- Buffers -----

/// The file manager's first entry, holding the open buffers.
fn buffers_branch() -> TreeNode {
    let count = ed::list_buffers().len();
    TreeNode {
        label: "open buffers/".to_string(),
        value: BUFFERS.to_string(),
        annotation: count.to_string(),
        detail: format!("{count} open"),
        parent: None,
        branch: true,
    }
}

/// A node for every open buffer, under `parent`. The values are buffer ids,
/// prefixed with the parent's value when there is one.
fn buffer_nodes(parent: Option<&str>) -> Vec<TreeNode> {
    ed::list_buffers()
        .into_iter()
        .map(|buffer| buffer_node(buffer, parent))
        .collect()
}

fn buffer_node(buffer: BufferInfo, parent: Option<&str>) -> TreeNode {
    TreeNode {
        label: buffer.name,
        value: match parent {
            Some(parent) => format!("{parent}/{}", buffer.id),
            None => buffer.id.to_string(),
        },
        annotation: match (buffer.modified, buffer.current) {
            (true, true) => "modified, current",
            (true, false) => "modified",
            (false, true) => "current",
            (false, false) => "",
        }
        .to_string(),
        detail: format!("buffer {}\n{} characters", buffer.id, buffer.len),
        parent: parent.map(String::from),
        branch: false,
    }
}

/// Make the buffer with this id (as text) current.
fn switch(id: &str) {
    let switched = id.parse().is_ok_and(ed::switch_buffer);
    if switched {
        ed::message(&format!("switched to {}", ed::get_buffer_name()));
    } else {
        ed::message(&format!("files: no buffer {id}"));
    }
}

/// Close the current buffer, showing the newest one left. Unless `discard` is
/// set, a buffer with unsaved changes stays open.
fn close(discard: bool) {
    let name = ed::get_buffer_name();
    let id = ed::get_buffer_id();
    let modified = ed::list_buffers().iter().any(|b| b.id == id && b.modified);
    if modified && !discard {
        ed::message(&format!(
            "{name} has unsaved changes: :write it, or :discard-buffer to close it anyway"
        ));
        return;
    }
    match ed::close_buffer(id) {
        Ok(()) => ed::message(&format!("closed {name}; now in {}", ed::get_buffer_name())),
        Err(error) => ed::message(&format!("files: {error}")),
    }
}

// ----- Files -----

/// The entries of a directory (`None` for the working directory) as tree nodes:
/// directories first, then files, each sorted by name ignoring case. Hidden
/// entries (starting with ".") are left out, as ranger does by default.
fn list(dir: Option<&str>) -> Vec<TreeNode> {
    let entries = match fs::read_dir(dir.unwrap_or(".")) {
        Ok(entries) => entries,
        Err(error) => {
            ed::message(&format!("can't list {}: {error}", dir.unwrap_or(".")));
            return Vec::new();
        }
    };
    let mut nodes: Vec<TreeNode> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            let path = match dir {
                Some(dir) => format!("{dir}/{name}"),
                None => name.clone(),
            };
            let is_dir = metadata.is_dir();
            Some(TreeNode {
                label: if is_dir { format!("{name}/") } else { name.clone() },
                value: path,
                annotation: if is_dir {
                    String::new()
                } else {
                    human_size(metadata.len())
                },
                detail: if is_dir {
                    "directory".to_string()
                } else {
                    format!("{}\n{} bytes", kind(&name), metadata.len())
                },
                parent: dir.map(String::from),
                branch: is_dir,
            })
        })
        .collect();
    nodes.sort_by_key(|node| (!node.branch, node.label.to_lowercase()));
    nodes
}

/// Open a file in a buffer of its own, or switch to its buffer if it is open.
fn open(path: &str) {
    if ed::list_buffers().iter().any(|buffer| buffer.name == path) {
        ed::open_buffer(path, "");
        ed::message(&format!("switched to {path}"));
        return;
    }
    match fs::read_to_string(path) {
        Ok(mut text) => {
            // The editor's convention (kept by the core plugin) is that the
            // buffer ends with a newline.
            if !text.ends_with('\n') {
                text.push('\n');
            }
            ed::open_buffer(path, &text);
            ed::message(&format!("opened {path}"));
        }
        Err(error) => ed::message(&format!("can't open {path}: {error}")),
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "K", "M", "G"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// A short description of a file from its extension, for the preview column.
fn kind(name: &str) -> String {
    let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    match extension {
        "rs" => "Rust source".to_string(),
        "toml" => "TOML file".to_string(),
        "md" => "Markdown".to_string(),
        "wit" => "WIT interface".to_string(),
        "lock" => "lock file".to_string(),
        "" => "file".to_string(),
        other => format!(".{other} file"),
    }
}
