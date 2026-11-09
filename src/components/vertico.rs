use iced::widget::{column, container, row, rule, text};
use iced::{Color, Element, Length, Padding};

#[derive(Debug, Clone)]
pub struct Vertico {
    pub visible: bool,
    pub query: String,
    pub selected: usize,
    candidates: Vec<String>,
    values: Vec<Vec<String>>,
    pub action: Option<String>,
    pub title: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerticoItem {
    pub label: String,
    pub arguments: Vec<String>,
}

impl From<&str> for VerticoItem {
    fn from(value: &str) -> Self {
        Self {
            label: value.to_string(),
            arguments: vec![value.to_string()],
        }
    }
}

impl Default for Vertico {
    fn default() -> Self {
        Self {
            visible: false,
            query: String::new(),
            selected: 0,
            candidates: Vec::new(),
            values: Vec::new(),
            action: None,
            title: "Vertico".into(),
            prompt: String::new(),
        }
    }
}

impl Vertico {
    pub fn replace_items(&mut self, items: Vec<VerticoItem>) {
        self.candidates = items.iter().map(|item| item.label.clone()).collect();
        self.values = items.into_iter().map(|item| item.arguments).collect();
        self.selected = 0;
        self.query.clear();
    }

    pub fn selected_arguments(&self) -> Option<&[String]> {
        let candidate_index = {
            let filtered: Vec<_> = self.filtered();
            filtered.get(self.selected)?;
            self.candidates
                .iter()
                .position(|candidate| candidate == filtered[self.selected])?
        };
        self.values.get(candidate_index).map(Vec::as_slice)
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.move_selection_internal(delta);
    }

    pub fn query_mut(&mut self) -> &mut String {
        &mut self.query
    }

    pub fn reset_selection(&mut self) {
        self.selected = 0;
    }

    pub fn insert_text(&mut self, text: &str) {
        self.query.push_str(text);
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

    fn move_selection_internal(&mut self, delta: isize) {
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
                text(if self.prompt.is_empty() {
                    &self.title
                } else {
                    &self.prompt
                })
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
        vertico.replace_items(
            ["save-buffer", "switch-buffer", "list-buffers"]
                .into_iter()
                .map(VerticoItem::from)
                .collect(),
        );
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

        vertico.insert_text("f");
        assert_eq!(vertico.query, "f");
        assert_eq!(vertico.selected, 0);
    }

    #[test]
    fn selection_navigation_wraps() {
        let mut vertico = Vertico::default();
        vertico.replace_items(
            ["save-buffer", "switch-buffer"]
                .into_iter()
                .map(VerticoItem::from)
                .collect(),
        );

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

        vertico.query_mut().pop();
        vertico.reset_selection();
        assert_eq!(vertico.query, "fil");
        assert_eq!(vertico.selected, 0);
    }

    #[test]
    fn escape_hides_vertico() {
        let mut vertico = Vertico::default();

        vertico.visible = false;
        assert!(!vertico.visible);
    }

    #[test]
    fn selected_item_reports_its_python_value() {
        let mut vertico = Vertico::default();
        vertico.replace_items(vec![VerticoItem {
            label: "Open README".into(),
            arguments: vec!["open-readme".into(), "README.md".into()],
        }]);

        assert_eq!(
            vertico.selected_arguments(),
            Some(vec![String::from("open-readme"), String::from("README.md")].as_slice())
        );
    }
}
