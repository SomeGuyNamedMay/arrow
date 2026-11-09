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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dot {
    pub cursor: usize,
    pub selection: Option<SelectionRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub enum Message {
    Key { key: keyboard::Key },
    ScrollBy { delta: f32, viewport_height: f32 },
    ScrollFrame,
}

#[derive(Debug, Clone)]
pub struct TextEditor {
    pub buffer: Rope,
    pub cursor: usize,
    pub cursors: Vec<usize>,
    pub anchors: Vec<Option<usize>>,
    pub anchor: Option<usize>,
    pub mode: Mode,
    scroll_y: f32,
    scroll_target_y: f32,
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
            scroll_y: 0.0,
            scroll_target_y: 0.0,
        }
    }
}

impl TextEditor {
    fn lines(&self) -> Vec<String> {
        self.buffer
            .lines()
            .map(|line| line.to_string().trim_end_matches('\n').to_string())
            .collect()
    }

    pub fn replace_ranges(&mut self, ranges: &[(usize, usize, String)]) {
        let mut ranges = ranges.to_vec();
        ranges.sort_by(|left, right| right.0.cmp(&left.0));
        for (start, end, replacement) in ranges {
            let start = start.min(self.char_count());
            let end = end.min(self.char_count()).max(start);
            self.buffer.remove(start..end);
            self.buffer.insert(start, &replacement);
        }
        self.cursor = self.cursor.min(self.char_count().saturating_sub(1));
        self.cursors = self
            .cursors
            .iter()
            .map(|cursor| (*cursor).min(self.char_count().saturating_sub(1)))
            .collect();
    }

    pub fn dots(&self) -> Vec<Dot> {
        let count = self.cursors.len().max(1);
        self.cursors
            .iter()
            .enumerate()
            .map(|(index, cursor)| Dot {
                cursor: *cursor,
                selection: self
                    .anchors
                    .get(index)
                    .copied()
                    .flatten()
                    .or_else(|| (index == 0).then_some(self.anchor).flatten())
                    .map(|anchor| SelectionRange {
                        start: anchor.min(*cursor),
                        end: anchor.max(*cursor).saturating_add(1),
                    }),
            })
            .take(count)
            .collect()
    }

    pub fn set_dots(&mut self, dots: &[Dot]) {
        if dots.is_empty() {
            return;
        }
        self.cursors = dots
            .iter()
            .map(|dot| {
                dot.selection.map_or(dot.cursor, |selection| {
                    if dot.cursor >= selection.end {
                        selection.end.saturating_sub(1)
                    } else if dot.cursor <= selection.start {
                        selection.start
                    } else {
                        dot.cursor
                    }
                })
            })
            .collect();
        self.anchors = dots
            .iter()
            .map(|dot| {
                dot.selection.map(|selection| {
                    if dot.cursor >= selection.end {
                        selection.start
                    } else if dot.cursor <= selection.start {
                        selection.end.saturating_sub(1)
                    } else {
                        selection.start
                    }
                })
            })
            .collect();
        self.anchor = self.anchors.first().copied().flatten();
        self.cursor = self.cursors.last().copied().unwrap_or(0);
        self.sync_anchors();
    }

    fn sync_anchors(&mut self) {
        self.anchors.resize(self.cursors.len(), None);
        if let Some(anchor) = self.anchor {
            if let Some(first) = self.anchors.first_mut() {
                *first = Some(anchor);
            }
        }
        self.cursor = *self.cursors.last().unwrap_or(&0);
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.buffer = Rope::from_str(&text.into());
        self.cursor = 0;
        self.cursors = vec![0];
        self.anchor = None;
        self.scroll_y = 0.0;
        self.scroll_target_y = 0.0;
        self
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ScrollBy {
                delta,
                viewport_height,
            } => {
                let max_scroll = self.max_scroll(viewport_height);
                self.scroll_target_y = (self.scroll_target_y - delta).clamp(0.0, max_scroll);
                return;
            }
            Message::ScrollFrame => {
                let remaining = self.scroll_target_y - self.scroll_y;
                if remaining.abs() < 0.5 {
                    self.scroll_y = self.scroll_target_y;
                } else {
                    self.scroll_y += remaining * 0.22;
                }
                return;
            }
            Message::Key { key } => self.update_key(key),
        }
    }

    fn update_key(&mut self, key: keyboard::Key) {
        if self.mode == Mode::Insert {
            match key.as_ref() {
                keyboard::Key::Character(character) => {
                    for character in character.chars() {
                        self.insert_char(character);
                    }
                }
                _ => {}
            }
            return;
        }
    }

    pub fn is_scrolling(&self) -> bool {
        (self.scroll_target_y - self.scroll_y).abs() >= 0.5
    }

    fn max_scroll(&self, viewport_height: f32) -> f32 {
        let content_height = self.buffer.len_lines() as f32 * LINE_HEIGHT + TEXT_Y;
        (content_height - viewport_height).max(0.0)
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

    pub(crate) fn normal_mode(&mut self) {
        self.mode = Mode::Normal;
        self.anchor = None;
        self.anchors.fill(None);
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
}

struct EditorCanvas<'a> {
    editor: &'a TextEditor,
}

impl canvas::Program<Message> for EditorCanvas<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !cursor.is_over(bounds) {
            return None;
        }

        let canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) = event else {
            return None;
        };
        let delta = match delta {
            mouse::ScrollDelta::Lines { y, .. } => y * LINE_HEIGHT * 3.0,
            mouse::ScrollDelta::Pixels { y, .. } => *y,
        };

        Some(
            canvas::Action::publish(Message::ScrollBy {
                delta,
                viewport_height: bounds.height,
            })
            .and_capture(),
        )
    }

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
        let first_row = (self.editor.scroll_y / LINE_HEIGHT).floor() as usize;
        let last_row = ((self.editor.scroll_y + bounds.height) / LINE_HEIGHT).ceil() as usize;
        let scroll_offset = self.editor.scroll_y;
        let selections = self.editor.selected_ranges();

        for (row, line) in lines
            .iter()
            .enumerate()
            .take(last_row.saturating_add(1))
            .skip(first_row)
        {
            let line_start = self.editor.buffer.line_to_char(row);
            let y = TEXT_Y + row as f32 * LINE_HEIGHT - scroll_offset;

            for &(selection_start, selection_end) in &selections {
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
        }

        for &cursor in &self.editor.cursors {
            let (row, column) = self.editor.position_at(cursor);
            if row < first_row || row > last_row {
                continue;
            }
            let line = lines.get(row).map(String::as_str).unwrap_or_default();
            let prefix: String = line.chars().take(column).collect();
            let cursor_x = if prefix.is_empty() {
                TEXT_X
            } else {
                TEXT_X + character_width::<Renderer>(&prefix)
            };
            frame.fill_rectangle(
                Point::new(cursor_x, TEXT_Y + row as f32 * LINE_HEIGHT - scroll_offset),
                iced::Size::new(character_width::<Renderer>("M"), LINE_HEIGHT - 2.0),
                if self.editor.mode == Mode::Insert {
                    Color::from_rgba(0.70, 0.85, 0.98, 0.55)
                } else {
                    Color::from_rgba(0.95, 0.72, 0.30, 0.85)
                },
            );
        }

        let (row, column) = self.editor.position();
        if row < first_row || row > last_row {
            return vec![frame.into_geometry()];
        }
        let line = lines.get(row).map(String::as_str).unwrap_or_default();
        let prefix: String = line.chars().take(column).collect();
        let cursor_x = if prefix.is_empty() {
            TEXT_X
        } else {
            TEXT_X + character_width::<Renderer>(&prefix)
        };
        if !self.editor.cursors.contains(&self.editor.cursor) {
            frame.fill_rectangle(
                Point::new(cursor_x, TEXT_Y + row as f32 * LINE_HEIGHT - scroll_offset),
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
        });
        assert_eq!(editor.buffer.to_string(), "a");
        assert_eq!(editor.cursor, 1);
    }

    #[test]
    fn scroll_eases_toward_a_clamped_target() {
        let text = (0..100).map(|_| "line").collect::<Vec<_>>().join("\n");
        let mut editor = TextEditor::default().with_text(text);

        editor.update(Message::ScrollBy {
            delta: -100.0,
            viewport_height: 100.0,
        });
        assert_eq!(editor.scroll_y, 0.0);
        assert_eq!(editor.scroll_target_y, 100.0);
        assert!(editor.is_scrolling());

        editor.update(Message::ScrollFrame);
        assert!(editor.scroll_y > 0.0);
        assert!(editor.scroll_y < editor.scroll_target_y);

        for _ in 0..100 {
            editor.update(Message::ScrollFrame);
        }
        assert_eq!(editor.scroll_y, editor.scroll_target_y);
        assert!(!editor.is_scrolling());

        editor.update(Message::ScrollBy {
            delta: -10_000.0,
            viewport_height: 100.0,
        });
        assert_eq!(editor.scroll_target_y, editor.max_scroll(100.0));

        editor.update(Message::ScrollBy {
            delta: 10_000.0,
            viewport_height: 100.0,
        });
        assert_eq!(editor.scroll_target_y, 0.0);
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
    fn selection_range_round_trips_through_dots() {
        let mut editor = TextEditor::default().with_text("one\ntwo");
        editor.set_dots(&[Dot {
            cursor: 3,
            selection: Some(SelectionRange { start: 0, end: 3 }),
        }]);

        let dots = editor.dots();
        assert_eq!(dots[0].cursor, 2);
        assert_eq!(dots[0].selection, Some(SelectionRange { start: 0, end: 3 }));

        editor.replace_ranges(&[(0, 3, String::new())]);
        assert_eq!(editor.buffer.to_string(), "\ntwo");
    }

    #[test]
    fn comma_keeps_only_the_first_cursor() {
        let mut editor = TextEditor::default().with_text("one\ntwo\nthree");
        editor.cursors = vec![1, 5, 9];
        editor.cursor = 9;

        editor.cursors.truncate(1);
        editor.anchors.truncate(1);
        editor.cursor = editor.cursors.first().copied().unwrap_or(0);

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
        });

        assert_eq!(editor.buffer.to_string(), "xone\nxtwo");
        assert_eq!(editor.cursors, vec![1, 6]);
    }
}
