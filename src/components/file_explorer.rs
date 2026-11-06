use iced::keyboard;
use iced::widget::{column, container, row, text};
use iced::{Color, Element, Length};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileExplorer {
    pub visible: bool,
    pub current_dir: PathBuf,
    pub entries: Vec<PathBuf>,
    pub selected: usize,
}

impl Default for FileExplorer {
    fn default() -> Self {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut explorer = Self {
            visible: false,
            current_dir: current_dir.clone(),
            entries: Vec::new(),
            selected: 0,
        };
        explorer.load_entries();
        explorer
    }
}

impl FileExplorer {
    fn load_entries(&mut self) {
        let mut directories = Vec::new();
        let mut files = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.current_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    directories.push(path);
                } else {
                    files.push(path);
                }
            }
        }

        directories.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
        });
        files.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
        });
        directories.extend(files);
        self.entries = directories;
        if self.entries.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.entries.len() - 1);
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.load_entries();
        }
    }

    pub fn selected_file(&self) -> Option<PathBuf> {
        self.entries
            .get(self.selected)
            .filter(|path| !path.is_dir())
            .cloned()
    }

    pub fn update_key(&mut self, key: &keyboard::Key) -> bool {
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
            keyboard::Key::Character(character) => match character.as_ref() {
                "k" => {
                    self.move_selection(-1);
                    true
                }
                "j" => {
                    self.move_selection(1);
                    true
                }
                "h" => {
                    self.parent_directory();
                    true
                }
                "l" => {
                    self.open_selected();
                    true
                }
                _ => false,
            },
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
                self.parent_directory();
                true
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowRight)
            | keyboard::Key::Named(keyboard::key::Named::Enter) => {
                self.open_selected();
                true
            }
            _ => false,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            self.selected = 0;
            return;
        }
        let current = self.selected as isize;
        self.selected = current
            .saturating_add(delta)
            .rem_euclid(self.entries.len() as isize) as usize;
    }

    fn parent_directory(&mut self) {
        let Some(parent) = self.current_dir.parent() else {
            return;
        };
        self.current_dir = parent.to_path_buf();
        self.selected = 0;
        self.load_entries();
    }

    fn open_selected(&mut self) {
        let Some(path) = self.entries.get(self.selected).cloned() else {
            return;
        };
        if path.is_dir() {
            self.current_dir = path;
            self.selected = 0;
            self.load_entries();
        }
    }

    fn name(path: &Path) -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    }

    fn entries_view(&self, entries: &[PathBuf], selected: Option<usize>) -> Element<'_, ()> {
        let mut list = column![].spacing(1).width(Length::Fill);
        for (index, entry) in entries.iter().take(50).enumerate() {
            let selected = selected == Some(index);
            let marker = if entry.is_dir() { "/" } else { " " };
            let label = format!("{marker}{}", Self::name(entry));
            let row = container(text(label).font(iced::Font::MONOSPACE))
                .width(Length::Fill)
                .padding([1, 8])
                .style(move |theme: &iced::Theme| {
                    let palette = theme.palette();
                    container::Style {
                        background: selected.then(|| Color::from_rgb(0.20, 0.35, 0.55).into()),
                        text_color: Some(if selected { Color::WHITE } else { palette.text }),
                        border: iced::Border::default(),
                        shadow: iced::Shadow::default(),
                        snap: false,
                    }
                });
            list = list.push(row);
        }
        list.into()
    }

    pub fn view(&self) -> Element<'_, ()> {
        if !self.visible {
            return container(text("")).height(Length::Shrink).into();
        }

        let parent_entries = self
            .current_dir
            .parent()
            .and_then(|parent| read_entries(parent))
            .unwrap_or_default();
        let preview = self.preview();

        row![
            container(self.entries_view(&parent_entries, None))
                .width(Length::FillPortion(1))
                .padding(8),
            container(self.entries_view(&self.entries, Some(self.selected)))
                .width(Length::FillPortion(1))
                .padding(8),
            container(preview).width(Length::FillPortion(1)).padding(8),
        ]
        .spacing(1)
        .height(Length::Fixed(220.0))
        .into()
    }

    fn preview(&self) -> Element<'_, ()> {
        let Some(selected) = self.entries.get(self.selected) else {
            return text("").into();
        };

        if selected.is_dir() {
            let entries = read_entries(selected).unwrap_or_default();
            return self.entries_view(&entries, None);
        }

        let preview = fs::read_to_string(selected)
            .unwrap_or_default()
            .lines()
            .take(50)
            .collect::<Vec<_>>()
            .join("\n");
        text(preview).font(iced::Font::MONOSPACE).into()
    }
}

fn read_entries(path: &Path) -> Option<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        entry
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    });
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_defaults_to_closed() {
        assert!(!FileExplorer::default().visible);
    }

    #[test]
    fn selection_wraps() {
        let mut explorer = FileExplorer::default();
        explorer.entries = vec![PathBuf::from("one"), PathBuf::from("two")];
        explorer.selected = 0;
        explorer.move_selection(-1);
        assert_eq!(explorer.selected, 1);
        explorer.move_selection(1);
        assert_eq!(explorer.selected, 0);
    }

    #[test]
    fn selected_file_returns_only_files() {
        let temp_directory =
            std::env::temp_dir().join(format!("arrow-file-explorer-{}", std::process::id()));
        let file = temp_directory.join("selected.txt");
        fs::create_dir_all(&temp_directory).unwrap();
        fs::write(&file, "contents").unwrap();

        let mut explorer = FileExplorer::default();
        explorer.entries = vec![temp_directory.clone(), file.clone()];
        explorer.selected = 0;
        assert_eq!(explorer.selected_file(), None);

        explorer.selected = 1;
        assert_eq!(explorer.selected_file(), Some(file.clone()));

        fs::remove_file(file).unwrap();
        fs::remove_dir(temp_directory).unwrap();
    }
}
