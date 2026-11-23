//! Saving a buffer to its file.
//!
//! Writing is the host's job, not a plugin's: plugins stay read-only, and a
//! buffer can only be written inside the directory Arrow was started in. A
//! buffer's name is its path, relative to that directory. Names in `*stars*`,
//! like `*scratch*`, aren't files.
//!
//! The text goes to a temporary file next to the target, which then replaces
//! it, so a failed or interrupted write never leaves half a file behind.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use crate::components::text_editor::Editor;

/// Write the buffer to the file it is named after, or to `path` if given (the
/// buffer then takes that name). Returns the path written.
pub fn write_buffer(editor: &mut Editor, path: Option<&str>) -> Result<String, String> {
    let name = path.unwrap_or(editor.name()).to_string();
    let target = writable_path(&name)?;
    write_atomically(&target, editor).map_err(|e| format!("can't write {name}: {e}"))?;
    editor.set_name(name.clone());
    editor.mark_saved();
    Ok(name)
}

/// The path for a buffer name, if it names a file inside the working directory.
fn writable_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || (name.starts_with('*') && name.ends_with('*')) {
        return Err(format!("{name} isn't a file; give a path to write it to"));
    }
    let path = Path::new(name);
    let inside = path
        .components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if !inside {
        return Err(format!(
            "{name} is outside the working directory; only files inside it can be written"
        ));
    }
    Ok(path.to_path_buf())
}

fn write_atomically(target: &Path, editor: &Editor) -> std::io::Result<()> {
    let file_name = target.file_name().unwrap_or_default().to_string_lossy();
    let temporary = target.with_file_name(format!(".{file_name}.arrow-write"));
    let result = (|| {
        let mut out = BufWriter::new(File::create(&temporary)?);
        for chunk in editor.rope().chunks() {
            out.write_all(chunk.as_bytes())?;
        }
        out.into_inner()?.sync_all()?;
        // Replaces the target, on Windows too.
        fs::rename(&temporary, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_inside_the_working_directory_can_be_written() {
        assert!(writable_path("src/main.rs").is_ok());
        assert!(writable_path("./notes.txt").is_ok());
        assert!(writable_path("*scratch*").is_err());
        assert!(writable_path("").is_err());
        assert!(writable_path("../outside.txt").is_err());
        assert!(writable_path("src/../../outside.txt").is_err());
        let absolute = if cfg!(windows) { "C:\\outside.txt" } else { "/tmp/outside.txt" };
        assert!(writable_path(absolute).is_err());
    }
}
