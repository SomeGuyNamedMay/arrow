use iced::advanced::text::{Paragraph, Renderer as TextRenderer, Text};
use iced::keyboard;
use iced::widget::canvas::{self, Canvas, Geometry};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse};
use ropey::Rope;
use std::path::Path;

use super::syntax::{SyntaxLanguageConfig, SyntaxState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dot {
    pub cursor: usize,
    pub selection: Option<SelectionRange>,
    pub is_focused: bool,
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

pub struct TextEditor {
    pub buffer: Rope,
    dots: Vec<Dot>,
    syntax: SyntaxState,
    pub mode: Mode,
    scroll_y: f32,
    scroll_target_y: f32,
    viewport_height: Option<f32>,
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
            syntax: SyntaxState::default(),
            dots: vec![Dot {
                cursor: 0,
                selection: None,
                is_focused: true,
            }],
            mode: Mode::Normal,
            scroll_y: 0.0,
            scroll_target_y: 0.0,
            viewport_height: None,
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
        self.refresh_syntax();
        self.dots = self
            .dots
            .iter()
            .map(|dot| Dot {
                cursor: dot.cursor.min(self.char_count().saturating_sub(1)),
                ..*dot
            })
            .collect();
        self.ensure_focused_dots_visible();
    }

    pub fn dots(&self) -> Vec<Dot> {
        self.dots.clone()
    }

    pub fn set_dots(&mut self, dots: &[Dot]) {
        if dots.is_empty() {
            return;
        }
        self.dots = dots
            .iter()
            .map(|dot| Dot {
                cursor: dot.selection.map_or(dot.cursor, |selection| {
                    if dot.cursor >= selection.end {
                        selection.end.saturating_sub(1)
                    } else if dot.cursor <= selection.start {
                        selection.start
                    } else {
                        dot.cursor
                    }
                }),
                ..*dot
            })
            .collect();
        self.ensure_focused_dots_visible();
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.buffer = Rope::from_str(&text.into());
        self.refresh_syntax();
        self.dots = vec![Dot {
            cursor: 0,
            selection: None,
            is_focused: true,
        }];
        self.scroll_y = 0.0;
        self.scroll_target_y = 0.0;
        self
    }

    pub fn set_file_path(&mut self, path: &Path) -> Result<(), String> {
        self.syntax.set_file_path(path, &self.buffer.to_string())
    }

    pub fn set_syntax_languages(
        &mut self,
        languages: Vec<SyntaxLanguageConfig>,
    ) -> Result<(), String> {
        self.syntax.set_languages(languages)
    }

    pub fn select_function_at_focused_dots(&mut self) {
        for dot in &mut self.dots {
            if !dot.is_focused {
                continue;
            }
            if let Some((start, end)) = self.syntax.enclosing_function(&self.buffer, dot.cursor) {
                dot.cursor = end.saturating_sub(1);
                dot.selection = Some(SelectionRange { start, end });
            }
        }
        self.ensure_focused_dots_visible();
    }

    fn refresh_syntax(&mut self) {
        if let Err(error) = self.syntax.reparse(&self.buffer.to_string()) {
            eprintln!("{error}");
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ScrollBy {
                delta,
                viewport_height,
            } => {
                self.viewport_height = Some(viewport_height);
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

    pub fn set_viewport_height(&mut self, viewport_height: f32) {
        self.viewport_height = Some(viewport_height);
        self.scroll_target_y = self.scroll_target_y.min(self.max_scroll(viewport_height));
        self.ensure_focused_dots_visible();
    }

    fn ensure_focused_dots_visible(&mut self) {
        let Some(viewport_height) = self.viewport_height else {
            return;
        };
        let focused_rows = self
            .dots
            .iter()
            .filter_map(|dot| dot.is_focused.then(|| self.position_at(dot.cursor).0))
            .collect::<Vec<_>>();
        let Some((&first, remaining_rows)) = focused_rows.split_first() else {
            return;
        };
        let first_row = remaining_rows.iter().fold(first, |min, row| min.min(*row));
        let last_row = remaining_rows.iter().fold(first, |max, row| max.max(*row));
        let visible_height = (viewport_height - TEXT_Y).max(LINE_HEIGHT);
        let focused_height = (last_row - first_row + 1) as f32 * LINE_HEIGHT;

        if focused_height <= visible_height {
            let first_top = first_row as f32 * LINE_HEIGHT;
            let last_bottom = (last_row + 1) as f32 * LINE_HEIGHT;
            if first_top < self.scroll_target_y {
                self.scroll_target_y = first_top;
            } else if last_bottom > self.scroll_target_y + visible_height {
                self.scroll_target_y = last_bottom - visible_height;
            }
        } else {
            let row = self
                .dots
                .iter()
                .rev()
                .find_map(|dot| dot.is_focused.then(|| self.position_at(dot.cursor).0))
                .unwrap_or(first_row);
            let top = row as f32 * LINE_HEIGHT;
            let bottom = top + LINE_HEIGHT;
            if top < self.scroll_target_y {
                self.scroll_target_y = top;
            } else if bottom > self.scroll_target_y + visible_height {
                self.scroll_target_y = bottom - visible_height;
            }
        }

        self.scroll_target_y = self
            .scroll_target_y
            .clamp(0.0, self.max_scroll(viewport_height));
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
        let dots = self.dots.clone();
        let positions = dots.iter().map(|dot| dot.cursor).collect::<Vec<_>>();
        let mut insertions = positions.clone();
        insertions.sort_unstable();
        insertions.dedup();

        for &position in insertions.iter().rev() {
            self.buffer.insert(position, &character.to_string());
        }

        let mut transformed = Vec::with_capacity(dots.len());
        for (dot, cursor) in dots.into_iter().zip(positions) {
            let following = insertions
                .iter()
                .filter(|&&position| position <= cursor)
                .count();
            let next = cursor.saturating_add(following).min(self.char_count());
            transformed.push(Dot {
                cursor: next,
                ..dot
            });
        }
        transformed.sort_unstable_by_key(|dot| dot.cursor);
        let mut merged = Vec::<Dot>::with_capacity(transformed.len());
        for dot in transformed {
            if let Some(previous) = merged.last_mut()
                && previous.cursor == dot.cursor
            {
                previous.is_focused |= dot.is_focused;
            } else {
                merged.push(dot);
            }
        }
        self.dots = merged;
        self.refresh_syntax();
        self.ensure_focused_dots_visible();
    }

    pub(crate) fn normal_mode(&mut self) {
        self.mode = Mode::Normal;
        for dot in &mut self.dots {
            dot.selection = None;
        }
    }

    fn position_at(&self, cursor: usize) -> (usize, usize) {
        let prefix: String = self.buffer.chars().take(cursor).collect();
        let lines: Vec<&str> = prefix.split('\n').collect();
        let row = lines.len().saturating_sub(1);
        let column = lines.last().map_or(0, |line| line.chars().count());
        (row, column)
    }

    fn selected_ranges(&self, mode: Mode) -> Vec<(SelectionRange, Color)> {
        self.dots
            .iter()
            .filter_map(|dot| {
                dot.selection
                    .map(|range| (range, dot_color(mode, dot.is_focused)))
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
        let selections = self.editor.selected_ranges(self.editor.mode);

        for (row, line) in lines
            .iter()
            .enumerate()
            .take(last_row.saturating_add(1))
            .skip(first_row)
        {
            let line_start = self.editor.buffer.line_to_char(row);
            let y = TEXT_Y + row as f32 * LINE_HEIGHT - scroll_offset;

            for &(selection, selection_color) in &selections {
                let selection_start = selection
                    .start
                    .max(line_start)
                    .min(line_start + line.chars().count());
                let selection_end = selection
                    .end
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
                        selection_color,
                    );
                }
            }

            let line_start_byte = self.editor.buffer.line_to_byte(row);
            let line_end_byte = line_start_byte + line.len();
            let mut boundaries = vec![0, line.len()];
            let line_spans = self
                .editor
                .syntax
                .line_highlights(row)
                .iter()
                .filter_map(|span| {
                    let start = span.start_byte.max(line_start_byte);
                    let end = span.end_byte.min(line_end_byte);
                    (start < end).then_some((start - line_start_byte, end - line_start_byte, *span))
                })
                .collect::<Vec<_>>();
            for (start, end, _) in &line_spans {
                boundaries.push(*start);
                boundaries.push(*end);
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut x = TEXT_X;
            for range in boundaries.windows(2) {
                let (start, end) = (range[0], range[1]);
                if start == end {
                    continue;
                }
                let segment = &line[start..end];
                let color = line_spans
                    .iter()
                    .filter(|(span_start, span_end, _)| *span_start <= start && *span_end >= end)
                    .max_by_key(|(_, _, span)| span.priority)
                    .map_or(Color::from_rgb(0.88, 0.89, 0.92), |(_, _, span)| span.color);
                let mut text = canvas::Text::default();
                text.content = segment.to_string();
                text.position = Point::new(x, y);
                text.color = color;
                text.size = iced::Pixels(16.0);
                text.font = iced::Font::MONOSPACE;
                frame.fill_text(text);
                x += character_width::<Renderer>(segment);
            }
        }

        for dot in &self.editor.dots {
            let (row, column) = self.editor.position_at(dot.cursor);
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
                dot_color(self.editor.mode, dot.is_focused),
            );
        }

        vec![frame.into_geometry()]
    }
}

fn dot_color(mode: Mode, is_focused: bool) -> Color {
    if is_focused {
        Color::from_rgba(0.20, 0.55, 0.95, 0.85)
    } else if mode == Mode::Insert {
        Color::from_rgba(0.70, 0.85, 0.98, 0.55)
    } else {
        Color::from_rgba(0.95, 0.72, 0.30, 0.85)
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
        assert_eq!(editor.dots()[0].cursor, 1);
    }

    #[test]
    fn focused_dot_color_is_blue_in_both_modes() {
        let blue = Color::from_rgba(0.20, 0.55, 0.95, 0.85);
        assert_eq!(dot_color(Mode::Normal, true), blue);
        assert_eq!(dot_color(Mode::Insert, true), blue);
        assert_ne!(dot_color(Mode::Normal, false), blue);
        assert_ne!(dot_color(Mode::Insert, false), blue);
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
    fn focused_dots_scroll_into_view_at_both_edges() {
        let text = (0..20).map(|_| "line").collect::<Vec<_>>().join("\n");
        let mut editor = TextEditor::default().with_text(text);
        editor.set_viewport_height(100.0);

        editor.set_dots(&[
            Dot {
                cursor: 20,
                selection: None,
                is_focused: true,
            },
            Dot {
                cursor: 60,
                selection: None,
                is_focused: false,
            },
        ]);
        assert_eq!(editor.scroll_target_y, 20.0);

        editor.set_dots(&[
            Dot {
                cursor: 0,
                selection: None,
                is_focused: true,
            },
            Dot {
                cursor: 60,
                selection: None,
                is_focused: false,
            },
        ]);
        assert_eq!(editor.scroll_target_y, 0.0);

        editor.set_dots(&[Dot {
            cursor: 60,
            selection: None,
            is_focused: false,
        }]);
        assert_eq!(editor.scroll_target_y, 0.0);
    }

    #[test]
    fn dot_position_uses_exact_monospace_advance() {
        let mut editor = TextEditor::default().with_text("Hello");
        editor.set_dots(&[Dot {
            cursor: 3,
            selection: None,
            is_focused: true,
        }]);
        let rendered_width = character_width::<Renderer>("Hel");

        assert!(rendered_width > 0.0);
        assert_eq!(editor.position_at(editor.dots()[0].cursor), (0, 3));
    }

    #[test]
    fn selection_range_round_trips_through_dots() {
        let mut editor = TextEditor::default().with_text("one\ntwo");
        editor.set_dots(&[Dot {
            cursor: 3,
            selection: Some(SelectionRange { start: 0, end: 3 }),
            is_focused: true,
        }]);

        let dots = editor.dots();
        assert_eq!(dots[0].cursor, 2);
        assert_eq!(dots[0].selection, Some(SelectionRange { start: 0, end: 3 }));

        editor.replace_ranges(&[(0, 3, String::new())]);
        assert_eq!(editor.buffer.to_string(), "\ntwo");
    }

    #[test]
    fn select_function_selects_only_at_focused_dots() {
        let source = "fn first() {}\nfn second() { let x = 1; }\n";
        let mut editor = TextEditor::default().with_text(source);
        let python = crate::components::python::PythonRuntime::new().unwrap();
        editor
            .set_syntax_languages(python.syntax_languages().unwrap())
            .unwrap();
        editor.set_file_path(Path::new("example.rs")).unwrap();
        editor.set_dots(&[
            Dot {
                cursor: source.find("let x").unwrap(),
                selection: None,
                is_focused: true,
            },
            Dot {
                cursor: source.find("first").unwrap(),
                selection: None,
                is_focused: false,
            },
        ]);

        editor.select_function_at_focused_dots();
        let dots = editor.dots();
        let selection = dots[0].selection.unwrap();
        assert!(
            source[editor.buffer.char_to_byte(selection.start)
                ..editor.buffer.char_to_byte(selection.end)]
                .starts_with("fn second()")
        );
        assert!(dots[0].cursor >= selection.start && dots[0].cursor < selection.end);
        assert_eq!(dots[1].selection, None);
    }

    #[test]
    fn selection_color_matches_its_dot_color() {
        let mut editor = TextEditor::default().with_text("one\ntwo");
        editor.set_dots(&[
            Dot {
                cursor: 3,
                selection: Some(SelectionRange { start: 0, end: 3 }),
                is_focused: true,
            },
            Dot {
                cursor: 7,
                selection: Some(SelectionRange { start: 4, end: 7 }),
                is_focused: false,
            },
        ]);

        let blue = Color::from_rgba(0.20, 0.55, 0.95, 0.85);
        let selections = editor.selected_ranges(Mode::Normal);
        assert_eq!(selections[0].1, blue);
        assert_eq!(selections[0].1, dot_color(Mode::Normal, true));
        assert_eq!(selections[1].1, dot_color(Mode::Normal, false));
    }

    #[test]
    fn comma_keeps_only_the_first_dot() {
        let mut editor = TextEditor::default().with_text("one\ntwo\nthree");
        editor.set_dots(&[
            Dot {
                cursor: 1,
                selection: None,
                is_focused: true,
            },
            Dot {
                cursor: 5,
                selection: None,
                is_focused: false,
            },
            Dot {
                cursor: 9,
                selection: None,
                is_focused: false,
            },
        ]);
        editor.set_dots(&editor.dots()[..1]);

        assert_eq!(editor.dots().len(), 1);
        assert_eq!(editor.dots()[0].cursor, 1);
    }

    #[test]
    fn inserts_text_at_every_dot() {
        let mut editor = TextEditor::default().with_text("one\ntwo");
        editor.set_dots(&[
            Dot {
                cursor: 0,
                selection: None,
                is_focused: true,
            },
            Dot {
                cursor: 4,
                selection: None,
                is_focused: false,
            },
        ]);
        editor.mode = Mode::Insert;

        editor.update(Message::Key {
            key: keyboard::Key::Character("x".into()),
        });

        assert_eq!(editor.buffer.to_string(), "xone\nxtwo");
        assert_eq!(
            editor
                .dots()
                .iter()
                .map(|dot| dot.cursor)
                .collect::<Vec<_>>(),
            vec![1, 6]
        );
    }
}
