//! Kakoune-style modal text editor component for iced.
//!
//! [`editor`] holds the buffer, selection, mode and undo history; this module
//! draws them. Editing commands and key bindings come from plugins.

mod editor;

use std::cell::Cell;

use iced::widget::text::{LineHeight, Span, Wrapping};
use iced::widget::{Column, column, container, responsive, rich_text, row, span, text};
use iced::{Center, Color, Element, Fill, Font, Size};

pub use editor::{Editor, Selection};

const FONT_SIZE: f32 = 15.0;
/// Fixed line height, so the number of visible lines is easy to work out.
const LINE_HEIGHT: f32 = 20.0;
const PADDING: f32 = 10.0;

const BACKGROUND: Color = Color::from_rgb8(30, 31, 40);
const TEXT: Color = Color::from_rgb8(220, 220, 225);
const GUTTER: Color = Color::from_rgb8(95, 98, 115);
/// Every dot is drawn the same way, cursor and selection alike.
const DOT: Color = Color::from_rgb8(255, 196, 92);
const INSERT_DOT: Color = Color::from_rgb8(120, 200, 255);
const DOT_TEXT: Color = Color::from_rgb8(30, 31, 40);
const MODELINE: Color = Color::from_rgb8(44, 46, 58);
const MODELINE_TEXT: Color = Color::from_rgb8(170, 172, 185);
const NORMAL_BADGE: Color = Color::from_rgb8(120, 200, 140);
const INSERT_BADGE: Color = Color::from_rgb8(120, 200, 255);

/// A modal, selection-first text editor modelled on Kakoune.
pub struct TextEditor {
    editor: Editor,
    /// First visible line. Adjusted while drawing to keep the cursor in view.
    top_line: Cell<usize>,
}

/// How a run of characters is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    /// Covered by a dot.
    Dot,
}

impl Default for TextEditor {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextEditor {
    pub fn new(text: &str) -> Self {
        Self {
            editor: Editor::new(text),
            top_line: Cell::new(0),
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    pub fn view<'a, Message: 'a>(&'a self) -> Element<'a, Message> {
        let buffer = container(responsive(move |size| self.view_lines(size)))
            .padding(PADDING)
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(BACKGROUND.into()),
                text_color: Some(TEXT),
                ..Default::default()
            });

        column![buffer, self.view_modeline()].into()
    }

    /// Draw only the lines that fit, scrolled so the cursor is visible.
    fn view_lines<'a, Message: 'a>(&'a self, size: Size) -> Element<'a, Message> {
        let rows = ((size.height / LINE_HEIGHT).floor() as usize).max(1);
        let starts = self.editor.line_starts();
        let (cursor_line, _) = self.editor.cursor_position();

        let mut top = self.top_line.get().min(starts.len() - 1);
        if cursor_line < top {
            top = cursor_line;
        } else if cursor_line >= top + rows {
            top = cursor_line + 1 - rows;
        }
        self.top_line.set(top);

        let chars = self.editor.chars();
        let gutter_width = starts.len().to_string().len();
        let lines = starts.iter().enumerate().skip(top).take(rows).map(|(n, &start)| {
            // The line's newline, or one past the end if the text doesn't end with one.
            let end = start + chars[start..].iter().position(|c| *c == '\n').unwrap_or(chars.len() - start);
            let number = text(format!("{:>gutter_width$} ", n + 1))
                .font(Font::MONOSPACE)
                .size(FONT_SIZE)
                .line_height(line_height())
                .color(if n == cursor_line { TEXT } else { GUTTER });
            let content = rich_text(self.line_spans(start, end))
                .font(Font::MONOSPACE)
                .size(FONT_SIZE)
                .line_height(line_height())
                .wrapping(Wrapping::None);
            row![number, content].into()
        });

        Column::with_children(lines).into()
    }

    /// Spans for one line (`start..=end`, where `end` is its newline), with every
    /// character a dot covers drawn the same way.
    fn line_spans(&self, start: usize, end: usize) -> Vec<Span<'static, (), Font>> {
        let chars = self.editor.chars();
        // Only the dots that touch this line matter.
        let dots: Vec<_> = self
            .editor
            .dots()
            .iter()
            .filter(|dot| dot.start() <= end && dot.end() >= start)
            .copied()
            .collect();
        // Typing modes draw each dot at its insertion point only.
        let inserting = self.editor.inserting();

        let style_of = |i: usize| {
            let covered = if inserting {
                dots.iter().any(|dot| dot.cursor == i)
            } else {
                dots.iter().any(|dot| dot.contains(i))
            };
            if covered { Style::Dot } else { Style::Plain }
        };

        let mut spans = Vec::new();
        let mut run = String::new();
        let mut run_style = style_of(start);
        for i in start..=end {
            let style = style_of(i);
            if style != run_style {
                spans.push(styled_span(std::mem::take(&mut run), run_style, inserting));
                run_style = style;
            }
            // Show the newline (or the end of the text) as a space so it can carry
            // a dot.
            run.push(match chars.get(i) {
                Some('\n' | '\t') | None => ' ',
                Some(&c) => c,
            });
        }
        spans.push(styled_span(run, run_style, inserting));
        spans
    }

    fn view_modeline<'a, Message: 'a>(&'a self) -> Element<'a, Message> {
        let (line, column) = self.editor.cursor_position();
        // Key hints and plugin messages both arrive as the editor's message.
        let status = self.editor.message().unwrap_or_default().to_string();
        let mode_name = self.editor.mode().to_uppercase();
        let badge = if self.editor.inserting() {
            INSERT_BADGE
        } else {
            NORMAL_BADGE
        };

        let label = |content: String| text(content).font(Font::MONOSPACE).size(13);

        container(
            row![
                label(status).width(Fill),
                label(format!("*scratch* {}:{}", line + 1, column + 1)),
                label(match self.editor.dots().len() {
                    1 => format!("{} sel", self.editor.selection().len()),
                    dots => format!("{dots} dots · {} sel", self.editor.selection().len()),
                }),
                container(label(mode_name).color(BACKGROUND))
                    .padding([0.0, 6.0])
                    .style(move |_| container::Style {
                        background: Some(badge.into()),
                        ..Default::default()
                    }),
            ]
            .spacing(12)
            .align_y(Center),
        )
        .padding([4.0, 8.0])
        .width(Fill)
        .style(|_| container::Style {
            background: Some(MODELINE.into()),
            text_color: Some(MODELINE_TEXT),
            ..Default::default()
        })
        .into()
    }
}

fn line_height() -> LineHeight {
    LineHeight::Absolute(LINE_HEIGHT.into())
}

fn styled_span(content: String, style: Style, inserting: bool) -> Span<'static, (), Font> {
    match style {
        Style::Plain => span(content),
        Style::Dot => {
            let color = if inserting { INSERT_DOT } else { DOT };
            span(content).background(color).color(DOT_TEXT)
        }
    }
}
