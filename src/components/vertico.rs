use iced::keyboard;
use iced::widget::{column, container, row, rule, text};
use iced::{Color, Element, Length, Padding};

#[derive(Debug, Clone)]
pub struct Vertico {
    pub visible: bool,
    pub query: String,
    pub selected: usize,
    candidates: Vec<String>,
}

impl Default for Vertico {
    fn default() -> Self {
        Self {
            visible: false,
            query: String::new(),
            selected: 0,
            candidates: vec![
                "open-file".into(),
                "save-buffer".into(),
                "switch-buffer".into(),
                "find-file".into(),
                "execute-command".into(),
                "grep-project".into(),
                "list-buffers".into(),
            ],
        }
    }
}

impl Vertico {
    pub fn update_key(&mut self, key: &keyboard::Key, text: Option<&str>) -> bool {
        match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                self.visible = false;
                true
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                self.move_selection(-1);
                true
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                self.move_selection(1);
                true
            }
            keyboard::Key::Named(keyboard::key::Named::Backspace) => {
                self.query.pop();
                self.selected = 0;
                true
            }
            keyboard::Key::Character(character) => {
                if let Some(text) = text
                    && !text.chars().all(char::is_control)
                {
                    self.query.push_str(text);
                    self.selected = 0;
                    return true;
                }

                match character {
                    "Tab" => {
                        self.visible = !self.visible;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.query.clear();
        self.selected = 0;
    }

    fn filtered(&self) -> Vec<&str> {
        let query: String = self.query.chars().flat_map(char::to_lowercase).collect();

        self.candidates
            .iter()
            .filter(|candidate| {
                if query.is_empty() {
                    return true;
                }

                let mut haystack = candidate.chars().flat_map(char::to_lowercase);
                query
                    .chars()
                    .all(|needle| haystack.any(|character| character == needle))
            })
            .map(String::as_str)
            .collect()
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.filtered().len();
        if count == 0 {
            self.selected = 0;
            return;
        }

        let current = self.selected as isize;
        self.selected = current.saturating_add(delta).rem_euclid(count as isize) as usize;
    }

    pub fn view(&self) -> Element<'_, ()> {
        if !self.visible {
            return container(text("")).height(Length::Shrink).into();
        }

        let filtered = self.filtered();
        let mut candidates = column![].spacing(1).width(Length::Fill);

        for (index, candidate) in filtered.iter().take(8).enumerate() {
            let selected = index == self.selected;
            let marker = if selected { ">" } else { " " };

            let candidate_row = row![
                text(marker)
                    .font(iced::Font::MONOSPACE)
                    .width(Length::Fixed(18.0)),
                text(*candidate)
                    .font(iced::Font::MONOSPACE)
                    .width(Length::Fill),
            ]
            .padding(Padding::from([1, 8]));

            candidates = candidates.push(container(candidate_row).width(Length::Fill).style(
                move |theme: &iced::Theme| {
                    let palette = theme.palette();
                    container::Style {
                        background: selected.then(|| Color::from_rgb(0.20, 0.35, 0.55).into()),
                        text_color: Some(if selected { Color::WHITE } else { palette.text }),
                        border: iced::Border::default(),
                        shadow: iced::Shadow::default(),
                        snap: false,
                    }
                },
            ));
        }

        column![
            rule::horizontal(1),
            container(row![
                text("M-x")
                    .font(iced::Font::MONOSPACE)
                    .width(Length::Fixed(38.0)),
                text(if self.query.is_empty() {
                    "_"
                } else {
                    &self.query
                })
                .font(iced::Font::MONOSPACE),
            ])
            .padding(Padding::from([8, 10]))
            .width(Length::Fill),
            rule::horizontal(1),
            candidates,
        ]
        .width(Length::Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_candidates_with_a_fuzzy_query() {
        let mut vertico = Vertico::default();
        vertico.query = "sf".into();

        assert_eq!(
            vertico.filtered(),
            vec!["save-buffer", "switch-buffer", "list-buffers"]
        );
    }

    #[test]
    fn typing_updates_query_and_resets_selection() {
        let mut vertico = Vertico::default();
        vertico.selected = 4;

        assert!(vertico.update_key(&keyboard::Key::Character("f".into()), Some("f"),));
        assert_eq!(vertico.query, "f");
        assert_eq!(vertico.selected, 0);
    }

    #[test]
    fn selection_navigation_wraps() {
        let mut vertico = Vertico::default();

        vertico.move_selection(-1);
        assert_eq!(vertico.selected, vertico.filtered().len() - 1);

        vertico.move_selection(1);
        assert_eq!(vertico.selected, 0);
    }

    #[test]
    fn backspace_removes_the_last_query_character() {
        let mut vertico = Vertico::default();
        vertico.query = "file".into();
        vertico.selected = 3;

        assert!(vertico.update_key(&keyboard::Key::Named(keyboard::key::Named::Backspace), None,));
        assert_eq!(vertico.query, "fil");
        assert_eq!(vertico.selected, 0);
    }

    #[test]
    fn escape_hides_vertico() {
        let mut vertico = Vertico::default();

        assert!(vertico.update_key(&keyboard::Key::Named(keyboard::key::Named::Escape), None,));
        assert!(!vertico.visible);
    }

    #[test]
    fn toggle_clears_query_and_selection() {
        let mut vertico = Vertico::default();
        vertico.query = "file".into();
        vertico.selected = 2;

        vertico.toggle();
        assert!(vertico.visible);
        assert!(vertico.query.is_empty());
        assert_eq!(vertico.selected, 0);

        vertico.toggle();
        assert!(!vertico.visible);
    }
}
