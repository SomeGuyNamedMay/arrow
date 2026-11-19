//! The editor's storage: text, dots, current mode and a status message.
//!
//! Dots are the editor's cursors, and each is also a selection. This module only
//! stores them. It has no editing behaviour: it doesn't move dots when the text
//! changes, merge them, keep a final newline or record undo history. Plugins do
//! all of that (the built-in behaviour is in `plugins/core`).
//!
//! Its one rule is safety for drawing: dot positions are kept inside the text,
//! and there is always at least one dot.

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

pub struct Editor {
    text: Vec<char>,
    /// Never empty. The last one is the primary dot.
    dots: Vec<Selection>,
    mode: String,
    /// Whether the current mode types characters (it has a fallback command).
    inserting: bool,
    message: Option<String>,
}

impl Editor {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.chars().collect(),
            dots: vec![Selection::point(0)],
            mode: INITIAL_MODE.to_string(),
            inserting: false,
            message: None,
        }
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

    pub fn chars(&self) -> &[char] {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn char_at(&self, index: usize) -> Option<char> {
        self.text.get(index).copied()
    }

    /// Text in `start..end` (end exclusive), clamped to the text.
    pub fn slice(&self, start: usize, end: usize) -> String {
        let end = end.min(self.text.len());
        let start = start.min(end);
        self.text[start..end].iter().collect()
    }

    /// Index of the first character of every line. A newline at the very end
    /// doesn't start another line.
    pub fn line_starts(&self) -> Vec<usize> {
        let body = &self.text[..self.text.len().saturating_sub(1)];
        std::iter::once(0)
            .chain(
                body.iter()
                    .enumerate()
                    .filter(|(_, c)| **c == '\n')
                    .map(|(i, _)| i + 1),
            )
            .collect()
    }

    /// 0-based line and column of the primary dot's cursor.
    pub fn cursor_position(&self) -> (usize, usize) {
        let cursor = self.selection().cursor.min(self.text.len());
        let before = &self.text[..cursor];
        let line = before.iter().filter(|c| **c == '\n').count();
        let line_start = before.iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
        (line, cursor - line_start)
    }

    // ----- Writing (used by plugins through the host) -----

    /// Pull a dot back inside the text.
    fn clamp(&self, selection: Selection) -> Selection {
        let last = self.text.len().saturating_sub(1);
        Selection {
            anchor: selection.anchor.min(last),
            cursor: selection.cursor.min(last),
        }
    }

    /// Replace `start..end` (end exclusive) with `text`. Dots are not moved, only
    /// pulled back inside the text if it got shorter.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) {
        let end = end.min(self.text.len());
        let start = start.min(end);
        self.text.splice(start..end, text.chars());
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

    #[test]
    fn positions() {
        let mut editor = Editor::new("ab\n\ncd\n");
        assert_eq!(editor.line_starts(), vec![0, 3, 4]);
        editor.set_selection(Selection::point(5));
        assert_eq!(editor.cursor_position(), (2, 1));
        // Text without a final newline, and empty text, still work.
        assert_eq!(Editor::new("ab\ncd").line_starts(), vec![0, 3]);
        assert_eq!(Editor::new("").line_starts(), vec![0]);
        assert_eq!(Editor::new("").cursor_position(), (0, 0));
    }
}
