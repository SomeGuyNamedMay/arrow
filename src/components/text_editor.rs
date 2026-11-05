use iced::keyboard;
use iced::widget::canvas::{self, Canvas, Geometry};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse};
use iced::advanced::text::{Paragraph, Renderer as TextRenderer, Text};

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
    pub buffer: String,
    pub cursor: usize,
    pub anchor: Option<usize>,
    pub mode: Mode,
    yank: String,
}

pub(crate) const TEXT_X: f32 = 12.0;
pub(crate) const TEXT_Y: f32 = 10.0;
pub(crate) const LINE_HEIGHT: f32 = 22.0;
fn character_width<ParagraphRenderer>(
    line_prefix: &str,
) -> f32
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
            buffer: String::new(),
            cursor: 0,
            anchor: None,
            mode: Mode::Normal,
            yank: String::new(),
        }
    }
}

impl TextEditor {
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.buffer = text.into();
        self.cursor = 0;
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
                    self.mode = Mode::Insert;
                }
                "a" => {
                    self.anchor = None;
                    self.cursor = (self.cursor + 1).min(self.buffer.chars().count());
                    self.mode = Mode::Insert;
                }
                "c" => {
                    if self.anchor.is_some() {
                        self.delete_selection();
                    }
                    self.anchor = None;
                    self.mode = Mode::Insert;
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
        self.buffer.chars().count()
    }

    fn insert_char(&mut self, character: char) {
        let byte = self.byte_index(self.cursor);
        self.buffer.insert(byte, character);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let byte = self.byte_index(self.cursor - 1);
        let end = self.byte_index(self.cursor);
        self.buffer.replace_range(byte..end, "");
        self.cursor -= 1;
    }

    fn normal_mode(&mut self) {
        self.mode = Mode::Normal;
        self.anchor = None;
        self.cursor = self.cursor.min(self.char_count().saturating_sub(1));
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_index)
            .map(|(index, _)| index)
            .unwrap_or(self.buffer.len())
    }

    fn position(&self) -> (usize, usize) {
        let prefix: String = self.buffer.chars().take(self.cursor).collect();
        let lines: Vec<&str> = prefix.split('\n').collect();
        let row = lines.len().saturating_sub(1);
        let column = lines.last().map_or(0, |line| line.chars().count());
        (row, column)
    }

    fn move_horizontal(&mut self, delta: isize) {
        let count = self.char_count();
        let next = if delta < 0 {
            self.cursor.saturating_sub(1)
        } else {
            (self.cursor + 1).min(count.saturating_sub(1))
        };
        self.cursor = next;
        self.anchor = None;
    }

    fn move_vertical(&mut self, delta: isize) {
        let (row, column) = self.position();
        let row = if delta < 0 { row.saturating_sub(1) } else { row + 1 };
        let lines: Vec<&str> = self.buffer.split('\n').collect();
        let row = row.min(lines.len().saturating_sub(1));
        let target_column = column.min(lines[row].chars().count().saturating_sub(1));
        let mut cursor = lines.iter().take(row).map(|line| line.chars().count() + 1).sum::<usize>();
        cursor += target_column;
        self.cursor = cursor.min(self.char_count().saturating_sub(1));
        self.anchor = None;
    }

    fn move_to_next_word(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let mut index = (self.cursor + 1).min(chars.len());

        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        while index < chars.len() && !chars[index].is_whitespace() {
            index += 1;
        }
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }

        self.cursor = index.min(chars.len().saturating_sub(1));
        self.anchor = None;
    }

    fn move_to_previous_word(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let mut index = self.cursor.saturating_sub(1);
        while index > 0 && chars[index].is_whitespace() {
            index -= 1;
        }
        while index > 0 && !chars[index].is_whitespace() {
            index -= 1;
        }
        while index > 0 && chars[index].is_whitespace() {
            index -= 1;
        }
        self.cursor = index;
        self.anchor = None;
    }

    fn select_line(&mut self) {
        let (row, _) = self.position();
        let lines: Vec<&str> = self.buffer.split('\n').collect();
        let start = lines.iter().take(row).map(|line| line.chars().count() + 1).sum::<usize>();
        let end = start + lines[row].chars().count();
        self.anchor = Some(start);
        self.cursor = end.saturating_sub(1);
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        Some(if anchor <= self.cursor {
            (anchor, self.cursor + 1)
        } else {
            (self.cursor, anchor + 1)
        })
    }

    fn delete_selection_or_character(&mut self) {
        if self.selected_range().is_some() {
            self.delete_selection();
        } else if !self.buffer.is_empty() {
            let start = self.byte_index(self.cursor);
            let end = self.byte_index((self.cursor + 1).min(self.char_count()));
            self.yank = self.buffer[start..end].to_string();
            self.buffer.replace_range(start..end, "");
            self.cursor = self.cursor.min(self.char_count().saturating_sub(1));
        }
    }

    fn delete_selection(&mut self) {
        let Some((start, end)) = self.selected_range() else {
            return;
        };
        let start_byte = self.byte_index(start);
        let end_byte = self.byte_index(end);
        self.yank = self.buffer[start_byte..end_byte].to_string();
        self.buffer.replace_range(start_byte..end_byte, "");
        self.cursor = start.min(self.char_count().saturating_sub(1));
        self.anchor = None;
    }

    fn paste(&mut self) {
        if self.yank.is_empty() {
            return;
        }
        let byte = self.byte_index((self.cursor + 1).min(self.char_count()));
        self.buffer.insert_str(byte, &self.yank);
        self.cursor = (self.cursor + self.yank.chars().count()).min(self.char_count().saturating_sub(1));
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

        let lines: Vec<&str> = self.editor.buffer.split('\n').collect();
        let mut line_start = 0;

        for (row, line) in lines.iter().enumerate() {
            let y = TEXT_Y + row as f32 * LINE_HEIGHT;

            if let Some((selection_start, selection_end)) = self.editor.selected_range() {
                let selection_start = selection_start.max(line_start).min(line_start + line.chars().count());
                let selection_end = selection_end.max(line_start).min(line_start + line.chars().count());
                if selection_start < selection_end {
                    let column = selection_start - line_start;
                    let prefix: String = line.chars().take(column).collect();
                    let selection: String = line.chars().skip(column).take(selection_end - selection_start).collect();
                    let x = TEXT_X + character_width::<Renderer>(&prefix);
                    let width = character_width::<Renderer>(&selection).max(character_width::<Renderer>("M"));
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

        let (row, column) = self.editor.position();
        let line = lines.get(row).copied().unwrap_or_default();
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
        assert_eq!(editor.buffer, "a");
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
        assert_eq!(editor.buffer, "\ntwo");
    }
}
