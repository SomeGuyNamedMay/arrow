//! Snapshots of a buffer, for undo.
//!
//! The core plugin's undo used to copy the whole buffer into the plugin as a
//! string several times per command, and compare those strings to see whether
//! anything changed. Now it asks the host to keep snapshots instead:
//!
//! - **Taking one is O(1).** A snapshot holds a clone of the buffer's rope,
//!   which shares every chunk with the live text. Only chunks edited
//!   afterwards stop being shared, so a history of many steps costs about as
//!   much memory as the edits themselves, not a full copy per step.
//! - **"Has anything changed?" is usually O(1).** Each snapshot records the
//!   buffer's revision (a counter bumped on every edit). If the revision is
//!   the same, nothing changed. Only if it differs are the ropes compared, to
//!   notice edits that cancelled out (type a character, delete it).
//! - **Restoring is O(1)**, by putting the snapshot's rope back.
//!
//! The plugin refers to snapshots by id and drops them when it is done with
//! them. Snapshots belong to the buffer they were taken from. Restoring one
//! into another buffer is refused, and closing a buffer drops its snapshots.

use ropey::Rope;

use super::{BufferId, Core};
use crate::components::text_editor::Selection;

pub type SnapshotId = u32;

/// A buffer's text and dots at one moment.
pub struct Snapshot {
    buffer: BufferId,
    text: Rope,
    dots: Vec<Selection>,
    /// The buffer's revision when the snapshot was taken.
    revision: u64,
}

impl Core {
    /// Save the current buffer's text and dots. Returns the snapshot's id.
    pub fn take_snapshot(&mut self) -> SnapshotId {
        self.last_snapshot_id = self.last_snapshot_id.wrapping_add(1);
        let editor = self.editor.editor();
        let snapshot = Snapshot {
            buffer: self.buffer_id,
            text: editor.rope().clone(),
            dots: editor.dots().to_vec(),
            revision: editor.revision(),
        };
        self.snapshots.insert(self.last_snapshot_id, snapshot);
        self.last_snapshot_id
    }

    /// Put the current buffer back as it was in the snapshot. Returns false if
    /// there is no such snapshot, or it belongs to another buffer.
    pub fn restore_snapshot(&mut self, id: SnapshotId) -> bool {
        let Some(snapshot) = self.snapshots.get(&id).filter(|s| s.buffer == self.buffer_id) else {
            return false;
        };
        let (text, dots) = (snapshot.text.clone(), snapshot.dots.clone());
        self.editor.editor_mut().restore(text, &dots);
        true
    }

    /// Whether the current buffer's text differs from the snapshot's. A missing
    /// snapshot, or one from another buffer, counts as different.
    pub fn changed_since(&self, id: SnapshotId) -> bool {
        let Some(snapshot) = self.snapshots.get(&id).filter(|s| s.buffer == self.buffer_id) else {
            return true;
        };
        let editor = self.editor.editor();
        // Same revision: not a single edit since. Otherwise compare, which
        // ropes do chunk by chunk after checking the lengths.
        editor.revision() != snapshot.revision && *editor.rope() != snapshot.text
    }

    pub fn drop_snapshot(&mut self, id: SnapshotId) {
        self.snapshots.remove(&id);
    }

    /// Forget the snapshots of a buffer that has been closed.
    pub(super) fn drop_snapshots_of(&mut self, buffer: BufferId) {
        self.snapshots.retain(|_, snapshot| snapshot.buffer != buffer);
    }
}

#[cfg(test)]
mod tests {
    use crate::components::text_editor::TextEditor;
    use crate::plugins::Core;

    #[test]
    fn snapshots_restore_and_notice_changes_cheaply() {
        let mut core = Core::new(TextEditor::new("one\n"), Vec::new());
        let id = core.take_snapshot();
        assert!(!core.changed_since(id));

        core.editor.editor_mut().replace(0, 0, "x");
        assert!(core.changed_since(id));
        // An edit that cancels out isn't a change.
        core.editor.editor_mut().replace(0, 1, "");
        assert!(!core.changed_since(id));

        core.editor.editor_mut().replace(0, 3, "two");
        assert!(core.restore_snapshot(id));
        assert_eq!(core.editor.editor().slice(0, 4), "one\n");

        // Snapshots stay with their buffer.
        core.open_buffer("other", "other\n");
        assert!(!core.restore_snapshot(id));
        assert!(core.changed_since(id));
        core.switch_buffer(0);
        core.drop_snapshot(id);
        assert!(!core.restore_snapshot(id));
    }
}
