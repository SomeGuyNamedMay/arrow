//! The editor's storage: text, dots, current mode and a status message.
//!
//! Dots are the editor's cursors, and each is also a selection. This module only
//! stores them. It has no editing behaviour: it doesn't move dots when the text
//! changes, merge them, keep a final newline or record undo history. Plugins do
//! all of that (the built-in behaviour is in `plugins/core`).
//!
//! Its one rule is safety for drawing: dot positions are kept inside the text,
//! and there is always at least one dot.
//!
//! The text is a [`Rope`], so edits and line lookups take logarithmic time
//! however large the buffer is. Positions are character indices.

use ropey::Rope;
use tree_sitter::InputEdit;

use crate::syntax;

/// Mode the editor starts in. Plugins define what it does.
pub const INITIAL_MODE: &str = "normal";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    /// The end that moves.
    pub cursor: usize,
}

impl Selection {
    pub fn point(index: usize) -> Self {
        Self {
            anchor: index,
            cursor: index,
        }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.cursor)
    }

    /// Inclusive end.
    pub fn end(&self) -> usize {
        self.anchor.max(self.cursor)
    }

    pub fn len(&self) -> usize {
        self.end() - self.start() + 1
    }

    pub fn contains(&self, index: usize) -> bool {
        (self.start()..=self.end()).contains(&index)
    }
}

/// Cheap to clone: the text is a rope, which shares its chunks between clones.
#[derive(Clone)]
pub struct Editor {
    text: Rope,
    /// Never empty. The last one is the primary dot.
    dots: Vec<Selection>,
    mode: String,
    /// Whether the current mode types characters (it has a fallback command).
    inserting: bool,
    message: Option<String>,
    /// Shown in the status bar, e.g. a file path.
    name: String,
    /// Changes since the syntax tree last caught up, in the order they were made.
    edits: Vec<InputEdit>,
}

impl Editor {
    pub fn new(text: &str) -> Self {
        Self {
            text: Rope::from_str(text),
            dots: vec![Selection::point(0)],
            mode: INITIAL_MODE.to_string(),
            inserting: false,
            message: None,
            name: "*scratch*".to_string(),
            edits: Vec::new(),
        }
    }

    /// The changes made since the last call, for the syntax tree.
    pub fn take_edits(&mut self) -> Vec<InputEdit> {
        std::mem::take(&mut self.edits)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    // ----- Reading -----

    /// The primary (last) dot.
    pub fn selection(&self) -> Selection {
        *self.dots.last().expect("there is always a dot")
    }

    pub fn dots(&self) -> &[Selection] {
        &self.dots
    }

    pub fn dot(&self, index: usize) -> Option<Selection> {
        self.dots.get(index).copied()
    }

    pub fn mode(&self) -> &str {
        &self.mode
    }

    pub fn inserting(&self) -> bool {
        self.inserting
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The whole text. Cloning it is cheap.
    pub fn rope(&self) -> &Rope {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len_chars()
    }

    pub fn char_at(&self, index: usize) -> Option<char> {
        (index < self.len()).then(|| self.text.char(index))
    }

    /// Text in `start..end` (end exclusive), clamped to the text.
    pub fn slice(&self, start: usize, end: usize) -> String {
        let end = end.min(self.len());
        let start = start.min(end);
        self.text.slice(start..end).to_string()
    }

    /// Number of lines. A newline at the very end doesn't start another line.
    pub fn line_count(&self) -> usize {
        let lines = self.text.len_lines();
        if lines > 1 && self.char_at(self.len() - 1) == Some('\n') {
            lines - 1
        } else {
            lines
        }
    }

    /// Index of the first character of line `line` (0-based, below `line_count`).
    pub fn line_start(&self, line: usize) -> usize {
        self.text.line_to_char(line)
    }

    /// Index of the newline ending line `line`, or the text's length if the line
    /// doesn't end with one.
    pub fn line_end(&self, line: usize) -> usize {
        let text = self.text.line(line);
        let chars = text.len_chars();
        let newline = chars > 0 && text.char(chars - 1) == '\n';
        self.line_start(line) + chars - usize::from(newline)
    }

    /// 0-based line and column of the primary dot's cursor.
    pub fn cursor_position(&self) -> (usize, usize) {
        let cursor = self.selection().cursor.min(self.len());
        let line = self.text.char_to_line(cursor).min(self.line_count() - 1);
        (line, cursor - self.line_start(line))
    }

    // ----- Writing (used by plugins through the host) -----

    /// Pull a dot back inside the text.
    fn clamp(&self, selection: Selection) -> Selection {
        let last = self.len().saturating_sub(1);
        Selection {
            anchor: selection.anchor.min(last),
            cursor: selection.cursor.min(last),
        }
    }

    /// Replace `start..end` (end exclusive) with `text`. Dots are not moved, only
    /// pulled back inside the text if it got shorter.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) {
        let end = end.min(self.len());
        let start = start.min(end);
        if start == end && text.is_empty() {
            return;
        }
        self.edits.push(syntax::input_edit(&self.text, start, end, text));
        self.text.remove(start..end);
        self.text.insert(start, text);
        self.dots = self.dots.iter().map(|dot| self.clamp(*dot)).collect();
    }

    /// Replace every dot. An empty list is ignored, since there must be a dot.
    pub fn set_dots(&mut self, dots: &[Selection]) {
        if !dots.is_empty() {
            self.dots = dots.iter().map(|dot| self.clamp(*dot)).collect();
        }
    }

    pub fn set_dot(&mut self, index: usize, selection: Selection) {
        let selection = self.clamp(selection);
        if let Some(dot) = self.dots.get_mut(index) {
            *dot = selection;
        }
    }

    pub fn set_selection(&mut self, selection: Selection) {
        let primary = self.dots.len() - 1;
        self.set_dot(primary, selection);
    }

    pub fn set_mode(&mut self, mode: &str, inserting: bool) {
        self.mode = mode.to_string();
        self.inserting = inserting;
    }

    pub fn set_message(&mut self, message: String) {
        self.message = Some(message);
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(editor: &Editor) -> String {
        editor.slice(0, editor.len())
    }

    #[test]
    fn replace_stores_text_without_moving_dots() {
        let mut editor = Editor::new("abc def");
        editor.set_dots(&[Selection::point(1), Selection::point(5)]);
        editor.replace(0, 0, "XX");
        assert_eq!(text(&editor), "XXabc def");
        assert_eq!(editor.dots(), [Selection::point(1), Selection::point(5)]);
    }

    #[test]
    fn dots_stay_inside_the_text() {
        let mut editor = Editor::new("abc");
        editor.set_selection(Selection {
            anchor: 0,
            cursor: 99,
        });
        assert_eq!(editor.selection().cursor, 2);
        editor.replace(0, 3, "");
        assert_eq!(editor.selection(), Selection::point(0));
        editor.set_dots(&[]);
        assert_eq!(editor.dots().len(), 1, "there is always a dot");
    }

    fn lines(editor: &Editor) -> Vec<(usize, usize)> {
        (0..editor.line_count())
            .map(|line| (editor.line_start(line), editor.line_end(line)))
            .collect()
    }

    #[test]
    fn positions() {
        let mut editor = Editor::new("ab\n\ncd\n");
        assert_eq!(lines(&editor), [(0, 2), (3, 3), (4, 6)]);
        editor.set_selection(Selection::point(5));
        assert_eq!(editor.cursor_position(), (2, 1));
        // Text without a final newline, and empty text, still work.
        assert_eq!(lines(&Editor::new("ab\ncd")), [(0, 2), (3, 5)]);
        assert_eq!(lines(&Editor::new("")), [(0, 0)]);
        assert_eq!(Editor::new("").cursor_position(), (0, 0));
        // Only "\n" ends a line.
        assert_eq!(Editor::new("a\rb\r\nc").line_count(), 2);
    }

    #[test]
    fn replace_works_across_rope_chunks() {
        let long = "line of text\n".repeat(5000);
        let mut editor = Editor::new(&long);
        editor.replace(13 * 2500, 13 * 2501, "");
        editor.replace(5, 5, "é");
        assert_eq!(editor.len(), long.chars().count() - 13 + 1);
        assert_eq!(editor.line_count(), 4999);
        assert_eq!(editor.slice(0, 8), "line éof");
    }
}
