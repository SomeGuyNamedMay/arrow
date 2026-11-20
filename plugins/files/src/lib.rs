//! A file manager inspired by ranger, built on the host's tree menu.
//!
//! `<space>f` (or `:file-manager`) opens the directory Arrow was started in. The
//! tree is loaded lazily: the host asks for a directory's contents the first time
//! it is highlighted or entered. Choosing a file loads it into the buffer.
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
use arrow::plugin::types::{Selection, TreeMenu, TreeNode};
use arrow::plugin::ui;

struct Files;

impl Guest for Files {
    fn init() {
        reg::register_command("file-manager", "Browse files, ranger style", false);
        reg::register_command(
            "list-directory",
            "List a directory in the file manager (used by it)",
            false,
        );
        reg::register_command("open-file", "Load the file named by the argument", false);
        if let Err(error) = reg::bind_key("normal", "<space>f", "file-manager") {
            ed::message(&format!("files: {error}"));
        }
    }

    fn run_command(name: String, arg: Option<String>) {
        match (name.as_str(), arg) {
            ("file-manager", _) => ui::open_tree_menu(&TreeMenu {
                title: "files".to_string(),
                nodes: list(None),
                on_select: "open-file".to_string(),
                on_expand: Some("list-directory".to_string()),
            }),
            ("list-directory", Some(dir)) => ui::add_tree_nodes(Some(&dir), &list(Some(&dir))),
            ("open-file", Some(path)) => open(&path),
            _ => ed::message(&format!("files: `{name}` needs a path")),
        }
    }

    fn on_event(_event: Event) {}
}

export!(Files);

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

/// Load a file into the buffer, replacing its text.
fn open(path: &str) {
    match fs::read_to_string(path) {
        Ok(mut text) => {
            // The editor's convention (kept by the core plugin) is that the
            // buffer ends with a newline.
            if !text.ends_with('\n') {
                text.push('\n');
            }
            ed::replace(0, ed::len(), &text);
            ed::set_dots(&[Selection {
                anchor: 0,
                cursor: 0,
            }]);
            ed::set_buffer_name(path);
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
