//! A Vertico-style command menu: a minibuffer prompt above a vertical list of
//! candidates that is filtered as you type.
//!
//! Matching works like Vertico with `orderless`: the input is split on spaces and a
//! candidate matches when its name contains every part, in any order. Recently run
//! commands sort first, then shorter names, then alphabetically.

use iced::widget::text::{LineHeight, Span};
use iced::widget::{Column, column, container, rich_text, row, span, text};
use iced::{Color, Element, Fill, Font, Length};

use super::keys::{EditorKey, KeyPress};

/// Candidates shown at once, like `vertico-count`.
const ROWS: usize = 10;
const FONT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 20.0;
const NAME_WIDTH: f32 = 260.0;
const BINDING_WIDTH: f32 = 70.0;

const BACKGROUND: Color = Color::from_rgb8(24, 25, 32);
const TEXT: Color = Color::from_rgb8(220, 220, 225);
const DIM: Color = Color::from_rgb8(120, 123, 140);
const PROMPT: Color = Color::from_rgb8(120, 200, 255);
const MATCH: Color = Color::from_rgb8(255, 196, 92);
const SELECTED_ROW: Color = Color::from_rgb8(50, 56, 82);
const CURSOR: Color = Color::from_rgb8(220, 220, 225);

#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub description: String,
    /// Key binding shown next to the name, like Emacs shows in `M-x`.
    pub binding: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The user chose this command.
    Execute(String),
    /// The menu was dismissed.
    Closed,
}

#[derive(Default)]
pub struct CommandMenu {
    open: bool,
    input: String,
    candidates: Vec<Candidate>,
    /// Indices into `candidates` that match `input`, best first.
    matches: Vec<usize>,
    /// Index into `matches`.
    selected: usize,
    /// First visible row of `matches`.
    scroll: usize,
    /// Names of executed commands, most recent first.
    history: Vec<String>,
}

impl CommandMenu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the menu with an empty prompt over `candidates`.
    pub fn open(&mut self, candidates: Vec<Candidate>) {
        self.open = true;
        self.input.clear();
        self.candidates = candidates;
        self.refilter();
    }

    /// Handle a key while the menu is open. Returns an event when the menu closes.
    pub fn handle_key(&mut self, press: KeyPress) -> Option<Event> {
        let ctrl = press.ctrl && !press.alt;
        match press.key {
            EditorKey::Escape => return Some(self.close()),
            EditorKey::Char('g') if ctrl => return Some(self.close()),
            EditorKey::Enter => {
                // With nothing matching, Enter does nothing and the menu stays open.
                let name = self.selected_candidate()?.name.clone();
                self.close();
                self.history.retain(|entry| *entry != name);
                self.history.insert(0, name.clone());
                return Some(Event::Execute(name));
            }
            EditorKey::Up => self.select_by(-1),
            EditorKey::Down => self.select_by(1),
            EditorKey::Char('p') if ctrl => self.select_by(-1),
            EditorKey::Char('n') if ctrl => self.select_by(1),
            // Like minibuffer completion: fill the prompt with the selected name.
            EditorKey::Tab => {
                if let Some(candidate) = self.selected_candidate() {
                    self.input = candidate.name.clone();
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
        self.open = false;
        Event::Closed
    }

    fn selected_candidate(&self) -> Option<&Candidate> {
        self.matches
            .get(self.selected)
            .map(|&index| &self.candidates[index])
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
        let terms = self.terms();
        let exact = self.input.trim().to_lowercase();
        let history = &self.history;
        let candidates = &self.candidates;

        let mut matches: Vec<usize> = (0..candidates.len())
            .filter(|&i| {
                let name = candidates[i].name.to_lowercase();
                terms.iter().all(|term| name.contains(term.as_str()))
            })
            .collect();
        matches.sort_by_key(|&i| {
            let name = &candidates[i].name;
            let recent = history
                .iter()
                .position(|entry| entry == name)
                .unwrap_or(usize::MAX);
            (name.to_lowercase() != exact, recent, name.len(), name.clone())
        });

        self.matches = matches;
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn view<'a, Message: 'a>(&'a self) -> Element<'a, Message> {
        let position = if self.matches.is_empty() {
            0
        } else {
            self.selected + 1
        };
        let prompt = rich_text(vec![
            span(format!("{position}/{} ", self.matches.len())).color(DIM),
            span("M-x ").color(PROMPT),
            span(self.input.clone()).color(TEXT),
            // Block cursor at the end of the input.
            span(" ").background(CURSOR),
        ] as Vec<Span<'static, (), Font>>)
        .font(Font::MONOSPACE)
        .size(FONT_SIZE)
        .line_height(line_height());

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
                    let candidate = &self.candidates[index];
                    let name = rich_text(highlighted(&candidate.name, &terms))
                        .font(Font::MONOSPACE)
                        .size(FONT_SIZE)
                        .line_height(line_height())
                        .width(Length::Fixed(NAME_WIDTH));
                    let content = row![
                        name,
                        label(candidate.binding.clone(), PROMPT)
                            .width(Length::Fixed(BINDING_WIDTH)),
                        label(candidate.description.clone(), DIM),
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
        .color(color)
}

/// Which characters of `name` match one of `terms` (case-insensitive).
fn matched_chars(name: &str, terms: &[String]) -> Vec<bool> {
    let lower: Vec<char> = name
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

/// Spans for a candidate name with the matched parts highlighted.
fn highlighted(name: &str, terms: &[String]) -> Vec<Span<'static, (), Font>> {
    let marked = matched_chars(name, terms);
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_marked = marked.first().copied().unwrap_or(false);
    for (c, &is_match) in name.chars().zip(&marked) {
        if is_match != run_marked {
            spans.push(name_span(std::mem::take(&mut run), run_marked));
            run_marked = is_match;
        }
        run.push(c);
    }
    spans.push(name_span(run, run_marked));
    spans
}

fn name_span(content: String, is_match: bool) -> Span<'static, (), Font> {
    span(content).color(if is_match { MATCH } else { TEXT })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str) -> Candidate {
        Candidate {
            name: name.to_string(),
            description: String::new(),
            binding: String::new(),
        }
    }

    fn menu() -> CommandMenu {
        let mut menu = CommandMenu::new();
        menu.open(
            ["select-all", "select-line", "delete-selection", "undo", "quit"]
                .into_iter()
                .map(candidate)
                .collect(),
        );
        menu
    }

    fn type_text(menu: &mut CommandMenu, text: &str) {
        for c in text.chars() {
            menu.handle_key(KeyPress::plain(EditorKey::Char(c)));
        }
    }

    fn names(menu: &CommandMenu) -> Vec<&str> {
        menu.matches
            .iter()
            .map(|&i| menu.candidates[i].name.as_str())
            .collect()
    }

    #[test]
    fn empty_input_sorts_by_length_then_name() {
        let menu = menu();
        assert_eq!(
            names(&menu),
            ["quit", "undo", "select-all", "select-line", "delete-selection"]
        );
    }

    #[test]
    fn terms_match_in_any_order() {
        let mut menu = menu();
        type_text(&mut menu, "sel");
        assert_eq!(names(&menu), ["select-all", "select-line", "delete-selection"]);
        type_text(&mut menu, " del");
        assert_eq!(names(&menu), ["delete-selection"]);
        type_text(&mut menu, "x");
        assert!(names(&menu).is_empty());
        // Enter with no match keeps the menu open.
        assert_eq!(menu.handle_key(KeyPress::plain(EditorKey::Enter)), None);
        assert!(menu.is_open());
    }

    #[test]
    fn navigate_and_execute() {
        let mut menu = menu();
        type_text(&mut menu, "select");
        menu.handle_key(KeyPress::plain(EditorKey::Down));
        assert_eq!(
            menu.handle_key(KeyPress::plain(EditorKey::Enter)),
            Some(Event::Execute("select-line".into()))
        );
        assert!(!menu.is_open());
    }

    #[test]
    fn recent_commands_sort_first() {
        let mut menu = menu();
        type_text(&mut menu, "delete");
        menu.handle_key(KeyPress::plain(EditorKey::Enter));
        menu.open(menu.candidates.clone());
        assert_eq!(names(&menu)[0], "delete-selection");
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
