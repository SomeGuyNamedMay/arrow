//! Drawing the terminal: a header line, then one row of styled text per screen
//! row, with runs of cells in the same style joined into one span.

use std::cell::Cell;

use iced::font::Weight;
use iced::widget::text::{LineHeight, Span, Wrapping};
use iced::widget::{Column, column, container, responsive, rich_text, span, text};
use iced::{Color, Element, Fill, Font, Length};

use super::Terminal;

const FONT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 18.0;
const PADDING: f32 = 6.0;

const BACKGROUND: Color = Color::from_rgb8(22, 23, 30);
const FOREGROUND: Color = Color::from_rgb8(210, 212, 220);
const HEADER: Color = Color::from_rgb8(44, 46, 58);
const HEADER_TEXT: Color = Color::from_rgb8(150, 152, 168);

/// The 16 basic colours, in a palette that sits with the editor's.
const ANSI: [Color; 16] = [
    Color::from_rgb8(30, 31, 40),
    Color::from_rgb8(224, 108, 117),
    Color::from_rgb8(152, 195, 121),
    Color::from_rgb8(229, 192, 123),
    Color::from_rgb8(97, 175, 239),
    Color::from_rgb8(198, 120, 221),
    Color::from_rgb8(86, 182, 194),
    Color::from_rgb8(200, 202, 210),
    Color::from_rgb8(95, 98, 115),
    Color::from_rgb8(240, 128, 136),
    Color::from_rgb8(172, 215, 141),
    Color::from_rgb8(245, 210, 143),
    Color::from_rgb8(117, 195, 255),
    Color::from_rgb8(218, 140, 241),
    Color::from_rgb8(106, 202, 214),
    Color::from_rgb8(240, 240, 245),
];

/// Width of one monospace cell, used to work out how many columns fit. An
/// estimate: monospace fonts are about 0.6 em wide.
pub fn char_width() -> f32 {
    FONT_SIZE * 0.6
}

/// The panel: `rows` lines of the screen, as wide as the window. The columns
/// that fit are written to `measured`, so the window can resize the terminal
/// to match.
pub fn view<'a, Message: 'a>(
    terminal: &'a Terminal,
    focused: bool,
    rows: u16,
    measured: &'a Cell<(u16, u16)>,
) -> Element<'a, Message> {
    let screen = terminal.screen();
    let status = if screen.scrollback() > 0 {
        format!("terminal · scrolled back {} lines · Shift-PageDown to return", screen.scrollback())
    } else if focused {
        "terminal · Ctrl-t: back to the editor · Alt-t: hide · Shift-PageUp: scroll back".to_string()
    } else {
        "terminal · Ctrl-t to type here".to_string()
    };
    let header = container(text(status).font(Font::MONOSPACE).size(12).color(HEADER_TEXT))
        .padding([2.0, 8.0])
        .width(Fill)
        .style(|_| container::Style {
            background: Some(HEADER.into()),
            ..Default::default()
        });

    let body = responsive(move |size| {
        let cols = ((size.width / char_width()).floor() as u16).max(1);
        measured.set((rows, cols));
        let screen = terminal.screen();
        let (_, screen_cols) = screen.size();
        let cursor = (focused && !screen.hide_cursor() && screen.scrollback() == 0)
            .then(|| screen.cursor_position());
        let lines = (0..screen.size().0.get()).map(|row| {
            let cursor_col = cursor.filter(|(r, _)| *r == row).map(|(_, c)| c);
            rich_text(row_spans(screen, row, screen_cols.get(), cursor_col))
                .font(Font::MONOSPACE)
                .size(FONT_SIZE)
                .line_height(LineHeight::Absolute(LINE_HEIGHT.into()))
                .wrapping(Wrapping::None)
                .into()
        });
        Column::with_children(lines).into()
    });

    let height = f32::from(rows) * LINE_HEIGHT + 2.0 * PADDING;
    let body = container(body)
        .padding(PADDING)
        .width(Fill)
        .height(Length::Fixed(height))
        .clip(true)
        .style(|_| container::Style {
            background: Some(BACKGROUND.into()),
            text_color: Some(FOREGROUND),
            ..Default::default()
        });
    column![header, body].into()
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Color,
    bg: Option<Color>,
    bold: bool,
}

fn row_spans(screen: &vt100::Screen, row: u16, cols: u16, cursor: Option<u16>) -> Vec<Span<'static, (), Font>> {
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_style = None;
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            break;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let style = cell_style(cell, cursor == Some(col));
        if run_style != Some(style) && !run.is_empty() {
            spans.push(styled(std::mem::take(&mut run), run_style.unwrap_or(style)));
        }
        run_style = Some(style);
        match cell.contents() {
            "" => run.push(' '),
            contents => run.push_str(contents),
        }
    }
    if let Some(style) = run_style
        && !run.is_empty()
    {
        spans.push(styled(run, style));
    }
    spans
}

fn cell_style(cell: &vt100::Cell, cursor: bool) -> Style {
    let mut fg = color(cell.fgcolor()).unwrap_or(FOREGROUND);
    let mut bg = color(cell.bgcolor());
    // Bold text in a basic colour uses its bright version, as most terminals do.
    if cell.bold()
        && let vt100::Color::Idx(i @ 0..=7) = cell.fgcolor()
    {
        fg = ANSI[usize::from(i) + 8];
    }
    if cell.dim() {
        fg = Color { a: 0.6, ..fg };
    }
    if cell.inverse() != cursor {
        let swapped = bg.unwrap_or(BACKGROUND);
        bg = Some(fg);
        fg = swapped;
    }
    Style {
        fg,
        bg,
        bold: cell.bold(),
    }
}

fn styled(content: String, style: Style) -> Span<'static, (), Font> {
    let font = if style.bold {
        Font {
            weight: Weight::Bold,
            ..Font::MONOSPACE
        }
    } else {
        Font::MONOSPACE
    };
    let span = span(content).color(style.fg).font(font);
    match style.bg {
        Some(bg) => span.background(bg),
        None => span,
    }
}

/// A terminal colour, or `None` for the default.
fn color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Rgb(r, g, b) => Some(Color::from_rgb8(r, g, b)),
        vt100::Color::Idx(i) if i < 16 => Some(ANSI[usize::from(i)]),
        // The 6×6×6 colour cube.
        vt100::Color::Idx(i @ 16..=231) => {
            let i = i - 16;
            let level = |x: u8| if x == 0 { 0 } else { 55 + 40 * x };
            Some(Color::from_rgb8(level(i / 36), level(i / 6 % 6), level(i % 6)))
        }
        // The grey ramp.
        vt100::Color::Idx(i) => {
            let grey = 8 + 10 * (i - 232);
            Some(Color::from_rgb8(grey, grey, grey))
        }
    }
}
