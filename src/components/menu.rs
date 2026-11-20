//! A generic Vertico-style menu: a minibuffer prompt above a vertical list of
//! items that is filtered as you type.
//!
//! It knows nothing about what it lists. Plugins open it with a [`MenuSpec`] (the
//! command menu, and later things like LSP completions) and get the chosen item's
//! value back through the spec's `on_select` command.
//!
//! Matching works like Vertico with `orderless`: the input is split on spaces and
//! an item matches when its label contains every part, in any order. Items keep
//! the order the plugin gave them, except that an exact match moves to the top,
//! so ranking is up to the plugin.

use iced::widget::text::{LineHeight, Span, Wrapping};
use iced::widget::{Column, column, container, rich_text, row, span, text};
use iced::{Color, Element, Fill, Font, Length};

use super::keys::{EditorKey, KeyPress};

/// Items shown at once, like `vertico-count`.
const ROWS: usize = 10;
const FONT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 20.0;
/// Narrowest the label and annotation columns get. They widen to fit the longest
/// entry in the menu.
const MIN_LABEL_WIDTH: f32 = 260.0;
const MIN_ANNOTATION_WIDTH: f32 = 70.0;
/// Approximate width of one character in the monospace font, for sizing columns.
const CHAR_WIDTH: f32 = FONT_SIZE * 0.62;
const COLUMN_GAP: f32 = 16.0;

const BACKGROUND: Color = Color::from_rgb8(24, 25, 32);
const TEXT: Color = Color::from_rgb8(220, 220, 225);
const DIM: Color = Color::from_rgb8(120, 123, 140);
const PROMPT: Color = Color::from_rgb8(120, 200, 255);
const MATCH: Color = Color::from_rgb8(255, 196, 92);
const SELECTED_ROW: Color = Color::from_rgb8(50, 56, 82);
const CURSOR: Color = Color::from_rgb8(220, 220, 225);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Shown first, and what the input is matched against.
    pub label: String,
    /// Handed back when the item is chosen.
    pub value: String,
    /// Shown highlighted in its own column (a key binding, a completion kind).
    pub annotation: String,
    /// Shown dimmed after it (a description, a type signature).
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSpec {
    pub prompt: String,
    /// Input to start with.
    pub input: String,
    /// In ranking order.
    pub items: Vec<MenuItem>,
    /// Command to run with the chosen item's value.
    pub on_select: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// An item was chosen: run `command` with `value`.
    Selected { command: String, value: String },
    /// The menu was dismissed.
    Closed,
}

#[derive(Clone, Default)]
pub struct Menu {
    /// The open menu, if any.
    spec: Option<MenuSpec>,
    input: String,
    /// Indices into the spec's items that match `input`, in display order.
    matches: Vec<usize>,
    /// Index into `matches`.
    selected: usize,
    /// First visible row of `matches`.
    scroll: usize,
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.spec.is_some()
    }

    /// Open a menu, replacing any that is open.
    pub fn open(&mut self, spec: MenuSpec) {
        self.input = spec.input.clone();
        self.spec = Some(spec);
        self.refilter();
    }

    /// Items matching the current input, in display order.
    #[cfg(test)]
    pub fn matching_items(&self) -> Vec<&MenuItem> {
        let Some(spec) = &self.spec else {
            return Vec::new();
        };
        self.matches.iter().map(|&i| &spec.items[i]).collect()
    }

    /// Handle a key while the menu is open. Returns an event when the menu closes.
    pub fn handle_key(&mut self, press: KeyPress) -> Option<Event> {
        let ctrl = press.ctrl && !press.alt;
        match press.key {
            EditorKey::Escape => return Some(self.close()),
            EditorKey::Char('g') if ctrl => return Some(self.close()),
            EditorKey::Enter => {
                // With nothing matching, Enter does nothing and the menu stays open.
                let value = self.selected_item()?.value.clone();
                let spec = self.spec.take()?;
                return Some(Event::Selected {
                    command: spec.on_select,
                    value,
                });
            }
            EditorKey::Up => self.select_by(-1),
            EditorKey::Down => self.select_by(1),
            EditorKey::Char('p') if ctrl => self.select_by(-1),
            EditorKey::Char('n') if ctrl => self.select_by(1),
            // Like minibuffer completion: fill the input with the selected label.
            EditorKey::Tab => {
                if let Some(item) = self.selected_item() {
                    self.input = item.label.clone();
                    self.refilter();
                }
            }
            EditorKey::Backspace => {
                if ctrl {
                    self.input.clear();
                } else {
                    self.input.pop();
                }
                self.refilter();
            }
            _ => {
                if let Some(c) = press.typed_char() {
                    self.input.push(c);
                    self.refilter();
                }
            }
        }
        None
    }

    fn close(&mut self) -> Event {
        self.spec = None;
        Event::Closed
    }

    fn selected_item(&self) -> Option<&MenuItem> {
        let spec = self.spec.as_ref()?;
        self.matches.get(self.selected).map(|&i| &spec.items[i])
    }

    fn select_by(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let last = self.matches.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + ROWS {
            self.scroll = self.selected + 1 - ROWS;
        }
    }

    fn terms(&self) -> Vec<String> {
        self.input
            .split_whitespace()
            .map(str::to_lowercase)
            .collect()
    }

    fn refilter(&mut self) {
        let Some(spec) = &self.spec else {
            return;
        };
        let terms = self.terms();
        let exact = self.input.trim().to_lowercase();
        let mut matches: Vec<usize> = (0..spec.items.len())
            .filter(|&i| {
                let label = spec.items[i].label.to_lowercase();
                terms.iter().all(|term| label.contains(term.as_str()))
            })
            .collect();
        // Keep the plugin's order, but put an exact match first.
        if let Some(position) = matches
            .iter()
            .position(|&i| spec.items[i].label.to_lowercase() == exact)
        {
            let exact_match = matches.remove(position);
            matches.insert(0, exact_match);
        }

        self.matches = matches;
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn view<'a, Message: 'a>(&'a self) -> Element<'a, Message> {
        let Some(spec) = &self.spec else {
            return column![].into();
        };
        let position = if self.matches.is_empty() {
            0
        } else {
            self.selected + 1
        };
        let prompt = rich_text(vec![
            span(format!("{position}/{} ", self.matches.len())).color(DIM),
            span(format!("{} ", spec.prompt)).color(PROMPT),
            span(self.input.clone()).color(TEXT),
            // Block cursor at the end of the input.
            span(" ").background(CURSOR),
        ] as Vec<Span<'static, (), Font>>)
        .font(Font::MONOSPACE)
        .size(FONT_SIZE)
        .line_height(line_height());

        // Size the columns to the longest entry so they line up without wrapping.
        let column_width = |longest: usize, min: f32| {
            (longest as f32 * CHAR_WIDTH + COLUMN_GAP).max(min)
        };
        let longest = |f: fn(&MenuItem) -> &str| {
            spec.items
                .iter()
                .map(|item| f(item).chars().count())
                .max()
                .unwrap_or(0)
        };
        let label_width = column_width(longest(|item| &item.label), MIN_LABEL_WIDTH);
        let annotation_width =
            column_width(longest(|item| &item.annotation), MIN_ANNOTATION_WIDTH);

        let terms = self.terms();
        let rows: Vec<Element<'a, Message>> = if self.matches.is_empty() {
            vec![label("No match".to_string(), DIM).into()]
        } else {
            self.matches
                .iter()
                .enumerate()
                .skip(self.scroll)
                .take(ROWS)
                .map(|(row_index, &index)| {
                    let item = &spec.items[index];
                    let name = rich_text(highlighted(&item.label, &terms))
                        .font(Font::MONOSPACE)
                        .size(FONT_SIZE)
                        .line_height(line_height())
                        .wrapping(Wrapping::None)
                        .width(Length::Fixed(label_width));
                    let content = row![
                        name,
                        label(item.annotation.clone(), PROMPT)
                            .width(Length::Fixed(annotation_width)),
                        label(item.detail.clone(), DIM),
                    ];
                    let selected = row_index == self.selected;
                    container(content)
                        .width(Fill)
                        .padding([0.0, 4.0])
                        .style(move |_| container::Style {
                            background: selected.then(|| SELECTED_ROW.into()),
                            ..Default::default()
                        })
                        .into()
                })
                .collect()
        };

        container(column![prompt, Column::with_children(rows)])
            .padding([4.0, 8.0])
            .width(Fill)
            .style(|_| container::Style {
                background: Some(BACKGROUND.into()),
                text_color: Some(TEXT),
                ..Default::default()
            })
            .into()
    }
}

fn line_height() -> LineHeight {
    LineHeight::Absolute(LINE_HEIGHT.into())
}

fn label<'a>(content: String, color: Color) -> iced::widget::Text<'a> {
    text(content)
        .font(Font::MONOSPACE)
        .size(FONT_SIZE)
        .line_height(line_height())
        .wrapping(Wrapping::None)
        .color(color)
}

/// Which characters of `label` match one of `terms` (case-insensitive).
fn matched_chars(label: &str, terms: &[String]) -> Vec<bool> {
    let lower: Vec<char> = label
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let mut marked = vec![false; lower.len()];
    for term in terms {
        let term: Vec<char> = term.chars().collect();
        if term.is_empty() || term.len() > lower.len() {
            continue;
        }
        if let Some(start) = lower.windows(term.len()).position(|window| window == term) {
            marked[start..start + term.len()].fill(true);
        }
    }
    marked
}

/// Spans for an item label with the matched parts highlighted.
fn highlighted(label: &str, terms: &[String]) -> Vec<Span<'static, (), Font>> {
    let marked = matched_chars(label, terms);
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_marked = marked.first().copied().unwrap_or(false);
    for (c, &is_match) in label.chars().zip(&marked) {
        if is_match != run_marked {
            spans.push(label_span(std::mem::take(&mut run), run_marked));
            run_marked = is_match;
        }
        run.push(c);
    }
    spans.push(label_span(run, run_marked));
    spans
}

fn label_span(content: String, is_match: bool) -> Span<'static, (), Font> {
    span(content).color(if is_match { MATCH } else { TEXT })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> MenuItem {
        MenuItem {
            label: label.to_string(),
            value: format!("value of {label}"),
            annotation: String::new(),
            detail: String::new(),
        }
    }

    fn menu() -> Menu {
        let mut menu = Menu::new();
        menu.open(MenuSpec {
            prompt: "pick".into(),
            input: String::new(),
            items: ["select-line", "select-all", "delete-selection", "undo", "select"]
                .into_iter()
                .map(item)
                .collect(),
            on_select: "use-it".into(),
        });
        menu
    }

    fn type_text(menu: &mut Menu, text: &str) {
        for c in text.chars() {
            menu.handle_key(KeyPress::plain(EditorKey::Char(c)));
        }
    }

    fn labels(menu: &Menu) -> Vec<&str> {
        menu.matching_items()
            .iter()
            .map(|item| item.label.as_str())
            .collect()
    }

    #[test]
    fn keeps_the_given_order() {
        let menu = menu();
        assert_eq!(
            labels(&menu),
            ["select-line", "select-all", "delete-selection", "undo", "select"]
        );
    }

    #[test]
    fn terms_match_in_any_order_and_exact_matches_come_first() {
        let mut menu = menu();
        type_text(&mut menu, "sel");
        assert_eq!(
            labels(&menu),
            ["select-line", "select-all", "delete-selection", "select"]
        );
        type_text(&mut menu, "ect");
        assert_eq!(labels(&menu)[0], "select");
        type_text(&mut menu, " del");
        assert_eq!(labels(&menu), ["delete-selection"]);
        type_text(&mut menu, "x");
        assert!(labels(&menu).is_empty());
        // Enter with no match keeps the menu open.
        assert_eq!(menu.handle_key(KeyPress::plain(EditorKey::Enter)), None);
        assert!(menu.is_open());
    }

    #[test]
    fn selecting_returns_the_value_and_command() {
        let mut menu = menu();
        menu.handle_key(KeyPress::plain(EditorKey::Down));
        assert_eq!(
            menu.handle_key(KeyPress::plain(EditorKey::Enter)),
            Some(Event::Selected {
                command: "use-it".into(),
                value: "value of select-all".into(),
            })
        );
        assert!(!menu.is_open());
    }

    #[test]
    fn tab_completes_and_escape_closes() {
        let mut menu = menu();
        type_text(&mut menu, "lin");
        menu.handle_key(KeyPress::plain(EditorKey::Tab));
        assert_eq!(menu.input, "select-line");
        assert_eq!(
            menu.handle_key(KeyPress::plain(EditorKey::Escape)),
            Some(Event::Closed)
        );
        assert!(!menu.is_open());
    }

    #[test]
    fn highlights_matched_parts() {
        let marked = matched_chars("select-all", &["all".into(), "se".into()]);
        let shown: String = marked.iter().map(|m| if *m { '^' } else { ' ' }).collect();
        assert_eq!(shown, "^^     ^^^");
    }
}
