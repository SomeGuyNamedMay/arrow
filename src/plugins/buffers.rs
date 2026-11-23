//! Several open buffers, one of them current.
//!
//! The current buffer lives in [`Core::editor`], so everything that edits or
//! draws "the editor" works on it unchanged; the others wait in
//! [`Core::hidden`] until a switch swaps one in. The mode belongs to the editor
//! rather than a buffer, so it carries over when the buffer changes.

use super::Core;
use crate::components::text_editor::TextEditor;

/// Identifies a buffer for as long as it is open. The first buffer is 0.
pub type BufferId = u32;

/// A buffer that isn't current: its text, dots, scroll position and
/// highlighting, kept as they were left.
pub struct Buffer {
    pub id: BufferId,
    pub editor: TextEditor,
}

/// A buffer as plugins and menus see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferInfo {
    pub id: BufferId,
    pub name: String,
    pub current: bool,
    pub len: usize,
    /// Changed since it was opened or last written.
    pub modified: bool,
}

impl Core {
    /// Open a buffer and make it current, or switch to the open buffer with
    /// this name (keeping its text). Returns its id.
    pub fn open_buffer(&mut self, name: &str, text: &str) -> BufferId {
        if let Some(id) = self.buffer_named(name) {
            self.switch_buffer(id);
            return id;
        }
        self.last_buffer_id += 1;
        let id = self.last_buffer_id;
        let mut editor = TextEditor::new(text);
        editor.editor_mut().set_name(name.to_string());
        self.hidden.push(Buffer { id, editor });
        self.switch_buffer(id);
        id
    }

    /// Make the buffer with this id current. Returns false if there is none.
    pub fn switch_buffer(&mut self, id: BufferId) -> bool {
        if id == self.buffer_id {
            return true;
        }
        let Some(position) = self.hidden.iter().position(|b| b.id == id) else {
            return false;
        };
        let mut next = self.hidden.remove(position).editor;
        let (mode, inserting) = {
            let current = self.editor.editor();
            (current.mode().to_string(), current.inserting())
        };
        next.editor_mut().set_mode(&mode, inserting);
        // A message belongs to the moment it was shown, not to the buffer.
        self.editor.editor_mut().clear_message();
        let previous = std::mem::replace(&mut self.editor, next);
        self.hidden.push(Buffer {
            id: self.buffer_id,
            editor: previous,
        });
        self.buffer_id = id;
        true
    }

    /// Close a buffer, discarding its text. If it is the current one, the most
    /// recently opened other buffer becomes current. The last buffer stays.
    pub fn close_buffer(&mut self, id: BufferId) -> Result<(), String> {
        if self.hidden.is_empty() {
            return Err("can't close the only buffer".to_string());
        }
        if id == self.buffer_id {
            let next = self.hidden.iter().map(|b| b.id).max().expect("not empty");
            self.switch_buffer(next);
        }
        let position = self
            .hidden
            .iter()
            .position(|b| b.id == id)
            .ok_or_else(|| format!("no buffer {id}"))?;
        self.hidden.remove(position);
        Ok(())
    }

    /// Every open buffer, in the order they were opened.
    pub fn buffers(&self) -> Vec<BufferInfo> {
        let current = std::iter::once((self.buffer_id, &self.editor));
        let hidden = self.hidden.iter().map(|b| (b.id, &b.editor));
        let mut buffers: Vec<BufferInfo> = current
            .chain(hidden)
            .map(|(id, editor)| BufferInfo {
                id,
                name: editor.editor().name().to_string(),
                current: id == self.buffer_id,
                len: editor.editor().len(),
                modified: editor.editor().is_modified(),
            })
            .collect();
        buffers.sort_by_key(|b| b.id);
        buffers
    }

    fn buffer_named(&self, name: &str) -> Option<BufferId> {
        if self.editor.editor().name() == name {
            return Some(self.buffer_id);
        }
        self.hidden
            .iter()
            .find(|b| b.editor.editor().name() == name)
            .map(|b| b.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(core: &Core) -> Vec<(BufferId, String, bool)> {
        core.buffers()
            .into_iter()
            .map(|b| (b.id, b.name, b.current))
            .collect()
    }

    #[test]
    fn buffers_open_switch_and_keep_their_text() {
        let mut core = Core {
            editor: TextEditor::new("scratch\n"),
            ..Default::default()
        };
        core.editor.editor_mut().set_mode("insert", true);
        assert_eq!(core.open_buffer("a.txt", "aaa\n"), 1);
        assert_eq!(core.editor.editor().slice(0, 3), "aaa");
        assert_eq!(core.editor.editor().mode(), "insert", "the mode carries over");
        core.editor.editor_mut().replace(0, 0, "edited ");

        assert_eq!(core.open_buffer("b.txt", "bbb\n"), 2);
        assert_eq!(
            names(&core),
            [
                (0, "*scratch*".into(), false),
                (1, "a.txt".into(), false),
                (2, "b.txt".into(), true),
            ]
        );

        // Opening a name that is already open switches to it; its edits stay.
        assert_eq!(core.open_buffer("a.txt", "reloaded\n"), 1);
        assert_eq!(core.editor.editor().slice(0, 10), "edited aaa");
        assert!(core.switch_buffer(0));
        assert_eq!(core.editor.editor().slice(0, 7), "scratch");
        assert!(!core.switch_buffer(9));
        assert_eq!(core.buffer_id, 0);
    }

    #[test]
    fn closing_buffers_picks_the_newest_and_never_reuses_ids() {
        let mut core = Core::default();
        core.open_buffer("a", "a\n");
        core.open_buffer("b", "b\n");
        assert!(core.switch_buffer(1));
        assert_eq!(core.close_buffer(1), Ok(()));
        assert_eq!(core.buffer_id, 2, "the newest other buffer becomes current");
        assert_eq!(core.close_buffer(0), Ok(()));
        assert_eq!(names(&core), [(2, "b".into(), true)]);
        assert!(core.close_buffer(2).is_err(), "the last buffer stays");
        assert!(core.close_buffer(7).is_err());
        // A new buffer gets a new id, even after the highest was closed.
        assert_eq!(core.open_buffer("c", "c\n"), 3);
    }
}
