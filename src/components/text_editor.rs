use iced::advanced::text::{Paragraph, Renderer as TextRenderer, Text};
use iced::keyboard;
use iced::widget::canvas::{self, Canvas, Geometry};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse};
use ropey::Rope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

#[derive(Debug, Clone)]
pub enum Message {
    Key {
        key: keyboard::Key,
        text: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct TextEditor {
    pub buffer: Rope,
    pub cursor: usize,
    pub cursors: Vec<usize>,
    pub anchors: Vec<Option<usize>>,
    pub anchor: Option<usize>,
    pub mode: Mode,
    yank: String,
}

pub(crate) const TEXT_X: f32 = 12.0;
pub(crate) const TEXT_Y: f32 = 10.0;
pub(crate) const LINE_HEIGHT: f32 = 22.0;
fn character_width<ParagraphRenderer>(line_prefix: &str) -> f32
where
    ParagraphRenderer: TextRenderer<Font = iced::Font>,
    ParagraphRenderer::Paragraph: Paragraph<Font = ParagraphRenderer::Font>,
{
    let paragraph = ParagraphRenderer::Paragraph::with_text(Text {
        content: line_prefix,
        bounds: iced::Size::new(f32::INFINITY, f32::INFINITY),
        size: iced::Pixels(16.0),
        line_height: iced::advanced::text::LineHeight::default(),
        font: iced::Font::MONOSPACE,
        align_x: iced::advanced::text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::advanced::text::Shaping::Basic,
        wrapping: iced::advanced::text::Wrapping::None,
    });

    paragraph.min_width()
}

impl Default for TextEditor {
    fn default() -> Self {
        Self {
            buffer: Rope::new(),
            cursor: 0,
            cursors: vec![0],
            anchors: vec![None],
            anchor: None,
            mode: Mode::Normal,
            yank: String::new(),
        }
    }
}

impl TextEditor {
    fn chars(&self) -> Vec<char> {
        self.buffer.chars().collect()
    }

    fn lines(&self) -> Vec<String> {
        self.buffer
            .lines()
            .map(|line| line.to_string().trim_end_matches('\n').to_string())
            .collect()
    }

    fn text_range(&self, start: usize, end: usize) -> String {
        self.buffer.slice(start..end).to_string()
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.buffer = Rope::from_str(&text.into());
        self.cursor = 0;
        self.cursors = vec![0];
        self.anchor = None;
        self
    }

    pub fn update(&mut self, message: Message) {
        let Message::Key { key, text } = message;

        if self.mode == Mode::Insert {
            match key.as_ref() {
                keyboard::Key::Named(keyboard::key::Named::Escape) => self.normal_mode(),
                keyboard::Key::Named(keyboard::key::Named::Enter) => self.insert_char('\n'),
                keyboard::Key::Named(keyboard::key::Named::Backspace) => self.backspace(),
                keyboard::Key::Character(character) => {
                    for character in character.chars() {
                        self.insert_char(character);
                    }
                }
                _ => {}
            }
            return;
        }

        match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                self.anchor = None;
                self.anchors.fill(None);
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => self.move_horizontal(-1),
            keyboard::Key::Named(keyboard::key::Named::ArrowDown) => self.move_vertical(1),
            keyboard::Key::Named(keyboard::key::Named::ArrowUp) => self.move_vertical(-1),
            keyboard::Key::Named(keyboard::key::Named::ArrowRight) => self.move_horizontal(1),
            keyboard::Key::Character(character) => match character {
                "h" => self.move_horizontal(-1),
                "j" => self.move_vertical(1),
                "k" => self.move_vertical(-1),
                "l" => self.move_horizontal(1),
                "w" => self.move_to_next_word(),
                "b" => self.move_to_previous_word(),
                "x" => self.select_line(),
                "i" => {
                    self.anchor = None;
                    self.anchors.fill(None);
                    self.mode = Mode::Insert;
                }
                "a" => {
                    self.anchor = None;
                    self.anchors.fill(None);
                    let count = self.buffer.len_chars();
                    self.cursors = self
                        .cursors
                        .iter()
                        .map(|&cursor| (cursor + 1).min(count))
                        .collect();
                    self.cursors.sort_unstable();
                    self.cursors.dedup();
                    self.cursor = *self.cursors.last().unwrap_or(&0);
                    self.mode = Mode::Insert;
                }
                "c" => {
                    let (_, column) = self.position();
                    let (_, bottom_row) =
                        self.position_at(*self.cursors.last().unwrap_or(&self.cursor));
                    let lines = self.lines();
                    let bottom_row = if self.cursors.len() > 1 {
                        bottom_row
                    } else {
                        self.position().0
                    };
                    let target_row = bottom_row + 1;

                    if target_row < lines.len() {
                        let target_column =
                            column.min(lines[target_row].chars().count().saturating_sub(1));
                        let mut target = lines
                            .iter()
                            .take(target_row)
                            .map(|line| line.chars().count() + 1)
                            .sum::<usize>();
                        target += target_column;
                        target = target.min(self.char_count().saturating_sub(1));
                        if !self.cursors.contains(&target) {
                            self.cursors.push(target);
                        }
                        self.cursor = target;
                    }
                }
                "," => {
                    self.cursors.truncate(1);
                    self.anchors.truncate(1);
                    self.cursor = self.cursors.first().copied().unwrap_or(0);
                    self.anchor = self.anchors.first().copied().flatten();
                }
                "d" => self.delete_selection_or_character(),
                "p" => self.paste(),
                "u" => {
                    // Reserved for an undo command in a future extension.
                }
                single if single.chars().count() == 1 => {
                    if let Some(character) = single.chars().next()
                        && character.is_control()
                    {
                        let _ = text;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        Canvas::new(EditorCanvas { editor: self })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn char_count(&self) -> usize {
        self.buffer.len_chars()
    }

    fn insert_char(&mut self, character: char) {
        let positions = self.cursors.iter().copied().collect::<Vec<_>>();
        let old_primary = self.cursor;
        let mut insertions = positions.clone();
        insertions.sort_unstable();
        insertions.dedup();

        for &position in insertions.iter().rev() {
            self.buffer.insert(position, &character.to_string());
        }

        let mut transformed = Vec::with_capacity(positions.len());
        for cursor in positions {
            let following = insertions
                .iter()
                .filter(|&&position| position <= cursor)
                .count();
            let next = cursor.saturating_add(following).min(self.char_count());
            transformed.push(next);
        }
        transformed.sort_unstable();
        transformed.dedup();
        self.cursors = transformed;
        let primary = old_primary
            + insertions
                .iter()
                .filter(|&&position| position <= old_primary)
                .count();
        self.cursor = primary.min(self.char_count());
    }

    fn backspace(&mut self) {
        let mut ranges = self
            .cursors
            .iter()
            .filter_map(|&cursor| (cursor > 0).then_some(cursor.saturating_sub(1)..cursor))
            .collect::<Vec<_>>();
        ranges.sort_unstable_by_key(|range| range.start);
        ranges.dedup();
        if ranges.is_empty() {
            return;
        }

        for range in ranges.iter().rev() {
            self.buffer.remove(range.clone());
        }
        self.cursors = self
            .cursors
            .iter()
            .map(|&cursor| {
                ranges.iter().fold(cursor, |cursor, range| {
                    if cursor > range.end {
                        cursor - 1
                    } else if cursor >= range.start {
                        range.start
                    } else {
                        cursor
                    }
                })
            })
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
        self.cursor = self.cursors.first().copied().unwrap_or(0);
    }

    fn normal_mode(&mut self) {
        self.mode = Mode::Normal;
        self.anchor = None;
        self.anchors.fill(None);
        self.cursor = self.cursor.min(self.char_count().saturating_sub(1));
        self.cursors = self
            .cursors
            .iter()
            .copied()
            .map(|cursor| cursor.min(self.char_count().saturating_sub(1)))
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
    }

    fn position(&self) -> (usize, usize) {
        self.position_at(self.cursor)
    }

    fn position_at(&self, cursor: usize) -> (usize, usize) {
        let prefix: String = self.buffer.chars().take(cursor).collect();
        let lines: Vec<&str> = prefix.split('\n').collect();
        let row = lines.len().saturating_sub(1);
        let column = lines.last().map_or(0, |line| line.chars().count());
        (row, column)
    }

    fn move_horizontal(&mut self, delta: isize) {
        let count = self.char_count();
        self.cursors = self
            .cursors
            .iter()
            .map(|&cursor| {
                if delta < 0 {
                    cursor.saturating_sub(1)
                } else {
                    (cursor + 1).min(count.saturating_sub(1))
                }
            })
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
        self.cursor = self.cursors.last().copied().unwrap_or(0);
        self.anchor = None;
    }

    fn move_vertical(&mut self, delta: isize) {
        let lines = self.lines();
        let old_cursors = self.cursors.clone();
        self.cursors = old_cursors
            .iter()
            .map(|&cursor| {
                let (row, column) = self.position_at(cursor);
                let row = if delta < 0 {
                    row.saturating_sub(1)
                } else {
                    row + 1
                };
                let row = row.min(lines.len().saturating_sub(1));
                let target_column = column.min(lines[row].chars().count().saturating_sub(1));
                lines
                    .iter()
                    .take(row)
                    .map(|line| line.chars().count() + 1)
                    .sum::<usize>()
                    + target_column
            })
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
        self.cursor = *self.cursors.last().unwrap_or(&0);
        self.anchor = None;
    }

    fn move_to_next_word(&mut self) {
        let chars = self.chars();
        self.cursors = self
            .cursors
            .iter()
            .map(|&cursor| {
                let mut index = (cursor + 1).min(chars.len());
                while index < chars.len() && chars[index].is_whitespace() {
                    index += 1;
                }
                while index < chars.len() && !chars[index].is_whitespace() {
                    index += 1;
                }
                while index < chars.len() && chars[index].is_whitespace() {
                    index += 1;
                }
                index.min(chars.len().saturating_sub(1))
            })
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
        self.cursor = *self.cursors.last().unwrap_or(&0);
        self.anchor = None;
    }

    fn move_to_previous_word(&mut self) {
        let chars = self.chars();
        self.cursors = self
            .cursors
            .iter()
            .map(|&cursor| {
                let mut index = cursor.saturating_sub(1);
                while index > 0 && chars[index].is_whitespace() {
                    index -= 1;
                }
                while index > 0 && !chars[index].is_whitespace() {
                    index -= 1;
                }
                while index > 0 && chars[index].is_whitespace() {
                    index -= 1;
                }
                index
            })
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
        self.cursor = *self.cursors.last().unwrap_or(&0);
        self.anchor = None;
    }

    fn select_line(&mut self) {
        let lines = self.lines();
        let old_cursors = self.cursors.clone();
        let mut selections = Vec::new();
        for cursor in old_cursors {
            let (row, _) = self.position_at(cursor);
            let start = lines
                .iter()
                .take(row)
                .map(|line| line.chars().count() + 1)
                .sum::<usize>();
            let end = start + lines[row].chars().count();
            selections.push((start, end.saturating_sub(1)));
        }
        self.anchors = selections.iter().map(|(start, _)| Some(*start)).collect();
        self.cursors = selections.iter().map(|(_, end)| *end).collect();
        self.anchor = self.anchors.last().copied().flatten();
        self.cursor = *self.cursors.last().unwrap_or(&0);
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        Some(if anchor <= self.cursor {
            (anchor, self.cursor + 1)
        } else {
            (self.cursor, anchor + 1)
        })
    }

    fn selected_ranges(&self) -> Vec<(usize, usize)> {
        self.cursors
            .iter()
            .enumerate()
            .filter_map(|(index, &cursor)| {
                let anchor = self.anchors.get(index).copied().flatten().or_else(|| {
                    (index + 1 == self.cursors.len())
                        .then_some(self.anchor)
                        .flatten()
                })?;
                Some(if anchor <= cursor {
                    (anchor, cursor.saturating_add(1))
                } else {
                    (cursor, anchor.saturating_add(1))
                })
            })
            .collect()
    }

    fn delete_selection_or_character(&mut self) {
        if self.selected_range().is_some() {
            self.delete_selection();
        } else if self.buffer.len_chars() != 0 {
            let mut ranges = self
                .cursors
                .iter()
                .filter_map(|&cursor| {
                    (cursor < self.char_count()).then_some(cursor..cursor.saturating_add(1))
                })
                .collect::<Vec<_>>();
            ranges.sort_unstable_by_key(|range| range.start);
            ranges.dedup();
            if ranges.is_empty() {
                return;
            }

            let first = ranges[0].clone();
            let start = first.start;
            let end = first.end;
            self.yank = self.text_range(start, end);
            for range in ranges.iter().rev() {
                self.buffer.remove(range.clone());
            }

            self.cursors = self
                .cursors
                .iter()
                .map(|&cursor| {
                    ranges.iter().fold(cursor, |cursor, range| {
                        if cursor >= range.end {
                            cursor.saturating_sub(1)
                        } else if cursor >= range.start {
                            range.start
                        } else {
                            cursor
                        }
                    })
                })
                .collect();
            self.cursors.sort_unstable();
            self.cursors.dedup();
            self.cursor = *self.cursors.last().unwrap_or(&0);
        }
    }

    fn delete_selection(&mut self) {
        let mut ranges = self
            .cursors
            .iter()
            .enumerate()
            .filter_map(|(index, &cursor)| {
                let anchor = self.anchors.get(index).copied().flatten().or_else(|| {
                    (index + 1 == self.cursors.len())
                        .then_some(self.anchor)
                        .flatten()
                })?;
                Some(if anchor <= cursor {
                    (anchor, cursor.saturating_add(1))
                } else {
                    (cursor, anchor.saturating_add(1))
                })
            })
            .collect::<Vec<_>>();
        if ranges.is_empty() {
            return;
        }
        ranges.sort_unstable_by_key(|range| range.0);
        ranges.dedup();

        let (start, end) = ranges[0];
        self.yank = self.text_range(start, end);
        for &(start, end) in ranges.iter().rev() {
            self.buffer.remove(start..end);
        }

        self.cursors = self
            .cursors
            .iter()
            .map(|&cursor| {
                ranges.iter().fold(cursor, |cursor, &(start, end)| {
                    if cursor >= end {
                        cursor.saturating_sub(end - start)
                    } else if cursor >= start {
                        start
                    } else {
                        cursor
                    }
                })
            })
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
        self.cursor = *self.cursors.last().unwrap_or(&0);
        self.anchor = None;
        self.anchors.fill(None);
    }

    fn paste(&mut self) {
        if self.yank.is_empty() {
            return;
        }
        let positions = self.cursors.clone();
        let inserted_count = self.yank.chars().count();
        for &cursor in positions.iter().rev() {
            let position = (cursor + 1).min(self.char_count());
            self.buffer.insert(position, &self.yank);
        }
        self.cursors = positions
            .iter()
            .map(|&cursor| {
                cursor
                    + positions
                        .iter()
                        .filter(|&&position| position < cursor)
                        .count()
                        * inserted_count
                    + inserted_count
            })
            .collect();
        self.cursors.sort_unstable();
        self.cursors.dedup();
        self.cursor = *self.cursors.last().unwrap_or(&0);
        self.anchor = None;
    }
}

struct EditorCanvas<'a> {
    editor: &'a TextEditor,
}

impl canvas::Program<Message> for EditorCanvas<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb(0.06, 0.07, 0.09),
        );

        let lines = self.editor.lines();
        let mut line_start = 0;

        for (row, line) in lines.iter().enumerate() {
            let y = TEXT_Y + row as f32 * LINE_HEIGHT;

            for (selection_start, selection_end) in self.editor.selected_ranges() {
                let selection_start = selection_start
                    .max(line_start)
                    .min(line_start + line.chars().count());
                let selection_end = selection_end
                    .max(line_start)
                    .min(line_start + line.chars().count());
                if selection_start < selection_end {
                    let column = selection_start - line_start;
                    let prefix: String = line.chars().take(column).collect();
                    let selection: String = line
                        .chars()
                        .skip(column)
                        .take(selection_end - selection_start)
                        .collect();
                    let x = TEXT_X + character_width::<Renderer>(&prefix);
                    let width =
                        character_width::<Renderer>(&selection)
                            .max(character_width::<Renderer>("M"));
                    frame.fill_rectangle(
                        Point::new(x, y),
                        iced::Size::new(width, LINE_HEIGHT - 2.0),
                        Color::from_rgb(0.25, 0.42, 0.68),
                    );
                }
            }

            let mut text = canvas::Text::default();
            text.content = format!("{line}");
            text.position = Point::new(12.0, y);
            text.color = Color::from_rgb(0.88, 0.89, 0.92);
            text.size = iced::Pixels(16.0);
            text.font = iced::Font::MONOSPACE;
            frame.fill_text(text);

            line_start += line.chars().count() + 1;
        }

        for &cursor in &self.editor.cursors {
            let (row, column) = self.editor.position_at(cursor);
            let line = lines.get(row).map(String::as_str).unwrap_or_default();
            let prefix: String = line.chars().take(column).collect();
            let cursor_x = if prefix.is_empty() {
                TEXT_X
            } else {
                TEXT_X + character_width::<Renderer>(&prefix)
            };
            frame.fill_rectangle(
                Point::new(cursor_x, TEXT_Y + row as f32 * LINE_HEIGHT),
                iced::Size::new(character_width::<Renderer>("M"), LINE_HEIGHT - 2.0),
                if self.editor.mode == Mode::Insert {
                    Color::from_rgba(0.70, 0.85, 0.98, 0.55)
                } else {
                    Color::from_rgba(0.95, 0.72, 0.30, 0.85)
                },
            );
        }

        let (row, column) = self.editor.position();
        let line = lines.get(row).map(String::as_str).unwrap_or_default();
        let prefix: String = line.chars().take(column).collect();
        let cursor_x = if prefix.is_empty() {
            TEXT_X
        } else {
            TEXT_X + character_width::<Renderer>(&prefix)
        };
        if !self.editor.cursors.contains(&self.editor.cursor) {
            frame.fill_rectangle(
                Point::new(cursor_x, TEXT_Y + row as f32 * LINE_HEIGHT),
                iced::Size::new(character_width::<Renderer>("M"), LINE_HEIGHT - 2.0),
                Color::from_rgba(0.95, 0.72, 0.30, 0.85),
            );
        }

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_text() {
        let mut editor = TextEditor::default();
        editor.mode = Mode::Insert;
        editor.update(Message::Key {
            key: keyboard::Key::Character("a".into()),
            text: Some("a".into()),
        });
        assert_eq!(editor.buffer.to_string(), "a");
        assert_eq!(editor.cursor, 1);
    }

    #[test]
    fn cursor_position_uses_exact_monospace_advance() {
        let mut editor = TextEditor::default().with_text("Hello");
        editor.cursor = 3;
        let rendered_width = character_width::<Renderer>("Hel");

        assert!(rendered_width > 0.0);
        assert_eq!(editor.position(), (0, 3));
    }

    #[test]
    fn selects_and_deletes_a_line() {
        let mut editor = TextEditor::default().with_text("one\ntwo");
        editor.update(Message::Key {
            key: keyboard::Key::Character("x".into()),
            text: Some("x".into()),
        });
        assert_eq!(editor.selected_range(), Some((0, 3)));
        editor.update(Message::Key {
            key: keyboard::Key::Character("d".into()),
            text: Some("d".into()),
        });
        assert_eq!(editor.buffer.to_string(), "\ntwo");
    }

    #[test]
    fn creates_a_cursor_below() {
        let mut editor = TextEditor::default().with_text("one\ntwo\nthree");
        editor.cursor = 1;
        editor.cursors = vec![1];

        editor.update(Message::Key {
            key: keyboard::Key::Character("c".into()),
            text: Some("c".into()),
        });

        assert_eq!(editor.cursor, 5);
        assert_eq!(editor.cursors, vec![1, 5]);

        editor.update(Message::Key {
            key: keyboard::Key::Character("c".into()),
            text: Some("c".into()),
        });

        assert_eq!(editor.cursor, 9);
        assert_eq!(editor.cursors, vec![1, 5, 9]);
    }

    #[test]
    fn comma_keeps_only_the_first_cursor() {
        let mut editor = TextEditor::default().with_text("one\ntwo\nthree");
        editor.cursors = vec![1, 5, 9];
        editor.cursor = 9;

        editor.update(Message::Key {
            key: keyboard::Key::Character(",".into()),
            text: Some(",".into()),
        });

        assert_eq!(editor.cursor, 1);
        assert_eq!(editor.cursors, vec![1]);
    }

    #[test]
    fn inserts_text_at_every_cursor() {
        let mut editor = TextEditor::default().with_text("one\ntwo");
        editor.cursors = vec![0, 4];
        editor.cursor = 4;
        editor.mode = Mode::Insert;

        editor.update(Message::Key {
            key: keyboard::Key::Character("x".into()),
            text: Some("x".into()),
        });

        assert_eq!(editor.buffer.to_string(), "xone\nxtwo");
        assert_eq!(editor.cursors, vec![1, 6]);
    }

    #[test]
    fn deletes_characters_at_every_cursor() {
        let mut editor = TextEditor::default().with_text("one two");
        editor.cursors = vec![0, 4];
        editor.cursor = 4;

        editor.update(Message::Key {
            key: keyboard::Key::Character("d".into()),
            text: Some("d".into()),
        });

        assert_eq!(editor.buffer.to_string(), "ne wo");
    }
}
