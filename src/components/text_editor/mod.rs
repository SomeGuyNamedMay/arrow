//! Kakoune-style modal text editor component for iced.
//!
//! [`editor`] holds the buffer, selection, mode and undo history; this module
//! draws them. Editing commands and key bindings come from plugins.

mod editor;

use std::cell::Cell;

use std::f32::consts::FRAC_PI_2;

use iced::gradient::{self, Gradient};
use iced::widget::text::{LineHeight, Span, Wrapping};
use iced::widget::{Column, column, container, responsive, rich_text, row, span, text};
use iced::{Center, Color, Element, Fill, Font, Radians, Size};

pub use editor::{Editor, Selection};

use crate::syntax::Highlighter;

const FONT_SIZE: f32 = 15.0;
/// Fixed line height, so the number of visible lines is easy to work out.
const LINE_HEIGHT: f32 = 20.0;
const PADDING: f32 = 10.0;

const BACKGROUND: Color = Color::from_rgb8(30, 31, 40);
const TEXT: Color = Color::from_rgb8(220, 220, 225);
const GUTTER: Color = Color::from_rgb8(95, 98, 115);
/// Every dot is drawn the same way, cursor and selection alike.
const DOT: Color = Color::from_rgb8(255, 196, 92);
/// Colour of the insert-mode bars.
const INSERT_DOT: Color = Color::from_rgb8(120, 200, 255);
/// Width of an insert-mode bar, as a fraction of one character.
const BAR_WIDTH: f32 = 0.2;
const DOT_TEXT: Color = Color::from_rgb8(30, 31, 40);
const MODELINE: Color = Color::from_rgb8(44, 46, 58);
const MODELINE_TEXT: Color = Color::from_rgb8(170, 172, 185);
const NORMAL_BADGE: Color = Color::from_rgb8(120, 200, 140);
const INSERT_BADGE: Color = Color::from_rgb8(120, 200, 255);

/// A modal, selection-first text editor modelled on Kakoune. Cheap to clone.
#[derive(Clone)]
pub struct TextEditor {
    editor: Editor,
    /// First visible line. Adjusted while drawing to keep the cursor in view.
    top_line: Cell<usize>,
    /// Colours the text, if the buffer has a language. Kept in step with the
    /// text by the session's syntax tree.
    highlighter: Option<Highlighter>,
}

/// How a run of characters is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    /// Covered by a dot (normal mode).
    Dot,
    /// Just after a dot's insertion point (insert mode): a bar is drawn on its
    /// left edge, between it and the character before.
    Bar,
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
            highlighter: None,
        }
    }

    pub fn set_highlighter(&mut self, highlighter: Option<Highlighter>) {
        self.highlighter = highlighter;
    }

    #[cfg(test)]
    pub fn highlighter(&self) -> Option<&Highlighter> {
        self.highlighter.as_ref()
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
        let line_count = self.editor.line_count();
        let (cursor_line, _) = self.editor.cursor_position();

        let mut top = self.top_line.get().min(line_count - 1);
        if cursor_line < top {
            top = cursor_line;
        } else if cursor_line >= top + rows {
            top = cursor_line + 1 - rows;
        }
        self.top_line.set(top);

        let gutter_width = line_count.to_string().len();
        let bottom = line_count.min(top + rows);
        // Colours for the visible lines only, newlines included.
        let first = self.editor.line_start(top);
        let shown = first..(self.editor.line_end(bottom - 1) + 1).min(self.editor.len());
        let colors = match &self.highlighter {
            Some(highlighter) => highlighter.colors(self.editor.rope(), shown),
            None => Vec::new(),
        };
        let lines = (top..bottom).map(|n| {
            // The line's newline, or one past the end if the text doesn't end with one.
            let (start, end) = (self.editor.line_start(n), self.editor.line_end(n));
            let number = text(format!("{:>gutter_width$} ", n + 1))
                .font(Font::MONOSPACE)
                .size(FONT_SIZE)
                .line_height(line_height())
                .color(if n == cursor_line { TEXT } else { GUTTER });
            let color = |i: usize| colors.get(i - first).copied().flatten();
            let content = rich_text(self.line_spans(start, end, color))
                .font(Font::MONOSPACE)
                .size(FONT_SIZE)
                .line_height(line_height())
                .wrapping(Wrapping::None);
            row![number, content].into()
        });

        Column::with_children(lines).into()
    }

    /// Spans for one line (`start..=end`, where `end` is its newline), with every
    /// character a dot covers drawn the same way, and the rest in its syntax
    /// colour (`color`, by character index).
    fn line_spans(
        &self,
        start: usize,
        end: usize,
        color: impl Fn(usize) -> Option<Color>,
    ) -> Vec<Span<'static, (), Font>> {
        // The line itself, with its newline if it has one.
        let chars: Vec<char> = self
            .editor
            .rope()
            .slice(start..(end + 1).min(self.editor.len()))
            .chars()
            .collect();
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
            if inserting {
                // Text is typed before the cursor's character, so the bar goes
                // on that character's left edge.
                if dots.iter().any(|dot| dot.cursor == i) {
                    Style::Bar
                } else {
                    Style::Plain
                }
            } else if dots.iter().any(|dot| dot.contains(i)) {
                Style::Dot
            } else {
                Style::Plain
            }
        };

        let mut spans = Vec::new();
        let mut run = String::new();
        let mut run_style = (style_of(start), color(start));
        for i in start..=end {
            let style = (style_of(i), color(i));
            // A bar is drawn across its span's left edge, so each bar character
            // gets a span of its own.
            if (style != run_style || style.0 == Style::Bar) && !run.is_empty() {
                spans.push(styled_span(std::mem::take(&mut run), run_style));
            }
            run_style = style;
            // Show the newline (or the end of the text) as a space so it can carry
            // a dot.
            run.push(match chars.get(i - start) {
                Some('\n' | '\t') | None => ' ',
                Some(&c) => c,
            });
        }
        spans.push(styled_span(run, run_style));
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
                label(format!("{} {}:{}", self.editor.name(), line + 1, column + 1)),
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

/// A span in a style, with its syntax colour if it has one. Text under a dot is
/// always drawn in the dot's text colour, so it stays readable.
fn styled_span(content: String, (style, color): (Style, Option<Color>)) -> Span<'static, (), Font> {
    let span = match color {
        Some(color) => span(content).color(color),
        None => span(content),
    };
    match style {
        Style::Plain => span,
        Style::Dot => span.background(DOT).color(DOT_TEXT),
        Style::Bar => span.background(bar()),
    }
}

/// A thin bar on the left edge of a character, drawn as a background that is
/// solid for the first fifth of its width and clear for the rest. This draws a
/// bar between two characters without shifting any text.
fn bar() -> Gradient {
    // Stops at the same offset replace each other, so the hard edge uses two
    // stops a hundredth apart.
    let clear = Color {
        a: 0.0,
        ..INSERT_DOT
    };
    Gradient::Linear(
        // A quarter turn points the gradient from left to right.
        gradient::Linear::new(Radians(FRAC_PI_2))
            .add_stop(0.0, INSERT_DOT)
            .add_stop(BAR_WIDTH, INSERT_DOT)
            .add_stop(BAR_WIDTH + 0.01, clear)
            .add_stop(1.0, clear),
    )
}
