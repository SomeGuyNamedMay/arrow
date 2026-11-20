//! A generic tree menu, laid out like the file manager ranger: three columns
//! showing the parent level, the current level, and a preview of the highlighted
//! node (its children, or its detail text for a leaf).
//!
//! It knows nothing about what it shows. Plugins open it with a [`TreeSpec`],
//! sending the tree as a flat list of nodes that each name their parent. Branches
//! can be filled in lazily: the first time a branch's children are needed, the
//! menu asks for them ([`TreeMenu::take_expand_request`]) and the plugin answers
//! with [`TreeMenu::add_nodes`].
//!
//! Keys: `j`/`k` or arrows to move, `h` to go up a level, `l` or Enter to enter a
//! branch or choose a leaf, `g`/`G` for top and bottom, Escape or `q` to close.

use std::collections::{HashMap, HashSet};

use iced::widget::text::{LineHeight, Wrapping};
use iced::widget::{Column, column, container, row, text};
use iced::{Color, Element, Fill, Font, Length};

use super::keys::{EditorKey, KeyPress};

/// Rows shown in each column, matching the list menu's height.
const ROWS: usize = 10;
const FONT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 20.0;

const BACKGROUND: Color = Color::from_rgb8(24, 25, 32);
const TEXT: Color = Color::from_rgb8(220, 220, 225);
const DIM: Color = Color::from_rgb8(120, 123, 140);
const BRANCH: Color = Color::from_rgb8(120, 200, 255);
const HEADER: Color = Color::from_rgb8(120, 200, 140);
const SELECTED_ROW: Color = Color::from_rgb8(50, 56, 82);
const PARENT_ROW: Color = Color::from_rgb8(38, 41, 56);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub label: String,
    /// Identifies the node; handed back when it is chosen or expanded.
    pub value: String,
    /// Shown beside the label, e.g. a file size.
    pub annotation: String,
    /// Shown in the preview column for a highlighted leaf.
    pub detail: String,
    /// `value` of the parent node, or `None` at the top level.
    pub parent: Option<String>,
    pub branch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSpec {
    pub title: String,
    pub nodes: Vec<TreeNode>,
    /// Command to run with a leaf's value when it is chosen.
    pub on_select: String,
    /// Command to run with a branch's value when its children are first needed.
    pub on_expand: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A leaf was chosen: run `command` with `value`.
    Selected { command: String, value: String },
    /// The menu was dismissed.
    Closed,
}

/// Key of a level in the tree: a branch's value, or `None` for the top level.
type Level = Option<String>;

#[derive(Clone, Default)]
pub struct TreeMenu {
    open: bool,
    title: String,
    on_select: String,
    on_expand: Option<String>,
    nodes: Vec<TreeNode>,
    /// Node index by value.
    index: HashMap<String, usize>,
    /// Children of each level, as node indices, in the order they were added.
    children: HashMap<Level, Vec<usize>>,
    /// Branches whose children have been delivered or asked for.
    loaded: HashSet<String>,
    /// The level listed in the middle column.
    level: Level,
    /// Highlighted row in each level visited, so going back keeps your place.
    cursor: HashMap<Level, usize>,
    /// A branch whose children should be asked for.
    expand_request: Option<String>,
}

impl TreeMenu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open a tree menu, replacing any that is open.
    pub fn open(&mut self, spec: TreeSpec) {
        *self = Self {
            open: true,
            title: spec.title,
            on_select: spec.on_select,
            on_expand: spec.on_expand,
            ..Self::default()
        };
        // Branches that come with children are already complete.
        let complete: HashSet<String> = spec.nodes.iter().filter_map(|n| n.parent.clone()).collect();
        self.loaded = complete;
        self.add_nodes(None, spec.nodes);
        self.request_expand_if_needed();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Add nodes, usually a branch's children in answer to an expand request.
    /// `parent` is marked complete. Nodes whose value is already present are
    /// ignored.
    pub fn add_nodes(&mut self, parent: Level, nodes: Vec<TreeNode>) {
        if let Some(parent) = parent {
            self.loaded.insert(parent);
        }
        for node in nodes {
            if self.index.contains_key(&node.value) {
                continue;
            }
            let position = self.nodes.len();
            self.index.insert(node.value.clone(), position);
            self.children
                .entry(node.parent.clone())
                .or_default()
                .push(position);
            self.nodes.push(node);
        }
        // The highlighted node may have just appeared.
        self.request_expand_if_needed();
    }

    /// Value of the highlighted node.
    #[cfg(test)]
    pub fn highlighted_value(&self) -> Option<&str> {
        self.highlighted().map(|node| node.value.as_str())
    }

    /// A branch whose children the plugin should provide, as (command, value).
    pub fn take_expand_request(&mut self) -> Option<(String, String)> {
        let value = self.expand_request.take()?;
        Some((self.on_expand.clone()?, value))
    }

    /// Handle a key while the menu is open. Returns an event when it closes.
    pub fn handle_key(&mut self, press: KeyPress) -> Option<Event> {
        let ctrl = press.ctrl && !press.alt;
        match press.key {
            EditorKey::Escape | EditorKey::Char('q') => return Some(self.dismiss()),
            EditorKey::Char('g') if ctrl => return Some(self.dismiss()),
            EditorKey::Down | EditorKey::Char('j') => self.move_by(1),
            EditorKey::Up | EditorKey::Char('k') => self.move_by(-1),
            EditorKey::Char('n') if ctrl => self.move_by(1),
            EditorKey::Char('p') if ctrl => self.move_by(-1),
            EditorKey::Char('g') | EditorKey::Home => self.move_by(isize::MIN / 2),
            EditorKey::Char('G') | EditorKey::End => self.move_by(isize::MAX / 2),
            EditorKey::Left | EditorKey::Backspace | EditorKey::Char('h') => self.go_up(),
            EditorKey::Right | EditorKey::Enter | EditorKey::Char('l') => {
                let node = self.highlighted()?.clone();
                if node.branch {
                    self.level = Some(node.value);
                    self.request_expand_if_needed();
                } else {
                    self.open = false;
                    return Some(Event::Selected {
                        command: self.on_select.clone(),
                        value: node.value,
                    });
                }
            }
            _ => {}
        }
        None
    }

    fn dismiss(&mut self) -> Event {
        self.open = false;
        Event::Closed
    }

    fn node(&self, value: &str) -> Option<&TreeNode> {
        self.index.get(value).map(|&i| &self.nodes[i])
    }

    /// Children of a level, as node indices.
    fn level_children(&self, level: &Level) -> &[usize] {
        self.children.get(level).map_or(&[], Vec::as_slice)
    }

    fn cursor(&self, level: &Level) -> usize {
        let count = self.level_children(level).len();
        self.cursor
            .get(level)
            .copied()
            .unwrap_or(0)
            .min(count.saturating_sub(1))
    }

    fn highlighted(&self) -> Option<&TreeNode> {
        let children = self.level_children(&self.level);
        children
            .get(self.cursor(&self.level))
            .map(|&i| &self.nodes[i])
    }

    fn move_by(&mut self, delta: isize) {
        let count = self.level_children(&self.level).len();
        if count == 0 {
            return;
        }
        let cursor = self.cursor(&self.level) as isize;
        let moved = cursor.saturating_add(delta).clamp(0, count as isize - 1) as usize;
        self.cursor.insert(self.level.clone(), moved);
        self.request_expand_if_needed();
    }

    fn go_up(&mut self) {
        if let Some(level) = &self.level {
            let parent = self.node(level).and_then(|node| node.parent.clone());
            self.level = parent;
            self.request_expand_if_needed();
        }
    }

    /// Ask for the highlighted branch's children if they haven't arrived, so the
    /// preview column can show them.
    fn request_expand_if_needed(&mut self) {
        if self.on_expand.is_none() {
            return;
        }
        let Some(node) = self.highlighted() else {
            return;
        };
        if node.branch && !self.loaded.contains(&node.value) {
            let value = node.value.clone();
            self.loaded.insert(value.clone());
            self.expand_request = Some(value);
        }
    }

    /// Labels from the top level down to the current level.
    fn path(&self) -> String {
        let mut labels = Vec::new();
        let mut level = self.level.clone();
        while let Some(value) = level {
            let Some(node) = self.node(&value) else { break };
            labels.push(node.label.clone());
            level = node.parent.clone();
        }
        labels.reverse();
        labels.concat()
    }

    pub fn view<'a, Message: 'a>(&'a self) -> Element<'a, Message> {
        let children = self.level_children(&self.level);
        let cursor = self.cursor(&self.level);
        let position = if children.is_empty() { 0 } else { cursor + 1 };
        let header = row![
            label(format!("{} ", self.title), HEADER),
            label(self.path(), TEXT).width(Fill),
            label(format!("{position}/{}", children.len()), DIM),
        ];

        // Parent level, with the current level highlighted in it.
        let parent_column = match &self.level {
            Some(value) => {
                let parent = self.node(value).and_then(|node| node.parent.clone());
                let siblings = self.level_children(&parent);
                let here = siblings.iter().position(|&i| self.nodes[i].value == *value);
                self.list(siblings, here, PARENT_ROW)
            }
            None => self.list(&[], None, PARENT_ROW),
        };

        let current_column = self.list(children, Some(cursor), SELECTED_ROW);

        // Preview: a branch's children, or a leaf's detail.
        let preview_column: Element<'a, Message> = match self.highlighted() {
            Some(node) if node.branch => {
                let level = Some(node.value.clone());
                if !self.children.contains_key(&level) {
                    let note = if self.loaded.contains(&node.value) {
                        "empty"
                    } else {
                        "…"
                    };
                    fixed_column(vec![label(note.to_string(), DIM).into()])
                } else {
                    self.list(self.level_children(&level), None, SELECTED_ROW)
                }
            }
            Some(node) => fixed_column(
                node.detail
                    .lines()
                    .take(ROWS)
                    .map(|line| label(line.to_string(), DIM).into())
                    .collect(),
            ),
            None => fixed_column(vec![label("empty".to_string(), DIM).into()]),
        };

        container(column![
            header,
            row![
                container(parent_column).width(Length::FillPortion(2)),
                container(current_column).width(Length::FillPortion(3)),
                container(preview_column).width(Length::FillPortion(4)),
            ]
            .spacing(12),
        ])
        .padding([4.0, 8.0])
        .width(Fill)
        .style(|_| container::Style {
            background: Some(BACKGROUND.into()),
            text_color: Some(TEXT),
            ..Default::default()
        })
        .into()
    }

    /// One column: the given nodes, scrolled to keep `highlight` visible.
    fn list<'a, Message: 'a>(
        &'a self,
        nodes: &[usize],
        highlight: Option<usize>,
        highlight_color: Color,
    ) -> Element<'a, Message> {
        let first = highlight.map_or(0, |h| (h + 1).saturating_sub(ROWS));
        let rows = nodes
            .iter()
            .enumerate()
            .skip(first)
            .take(ROWS)
            .map(|(i, &node)| {
                let node = &self.nodes[node];
                let color = if node.branch { BRANCH } else { TEXT };
                let highlighted = highlight == Some(i);
                container(row![
                    label(node.label.clone(), color).width(Fill),
                    label(node.annotation.clone(), DIM),
                ])
                .width(Fill)
                .padding([0.0, 4.0])
                .style(move |_| container::Style {
                    background: highlighted.then(|| highlight_color.into()),
                    ..Default::default()
                })
                .into()
            })
            .collect();
        fixed_column(rows)
    }
}

/// A column of rows with the same height as the list menu, however many rows.
fn fixed_column<'a, Message: 'a>(rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    Column::with_children(rows)
        .height(Length::Fixed(ROWS as f32 * LINE_HEIGHT))
        .into()
}

fn label<'a>(content: String, color: Color) -> iced::widget::Text<'a> {
    text(content)
        .font(Font::MONOSPACE)
        .size(FONT_SIZE)
        .line_height(LineHeight::Absolute(LINE_HEIGHT.into()))
        .wrapping(Wrapping::None)
        .color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(value: &str, parent: Option<&str>, branch: bool) -> TreeNode {
        TreeNode {
            label: value.rsplit('/').next().unwrap().to_string(),
            value: value.to_string(),
            annotation: String::new(),
            detail: format!("detail of {value}"),
            parent: parent.map(String::from),
            branch,
        }
    }

    fn press(menu: &mut TreeMenu, key: char) -> Option<Event> {
        menu.handle_key(KeyPress::plain(EditorKey::Char(key)))
    }

    fn highlighted(menu: &TreeMenu) -> &str {
        &menu.highlighted().unwrap().value
    }

    /// A lazily loaded tree: only the top level is given up front.
    fn lazy_menu() -> TreeMenu {
        let mut menu = TreeMenu::new();
        menu.open(TreeSpec {
            title: "files".into(),
            nodes: vec![node("src", None, true), node("README", None, false)],
            on_select: "open".into(),
            on_expand: Some("expand".into()),
        });
        menu
    }

    #[test]
    fn asks_for_children_of_the_highlighted_branch_once() {
        let mut menu = lazy_menu();
        assert_eq!(
            menu.take_expand_request(),
            Some(("expand".into(), "src".into()))
        );
        assert_eq!(menu.take_expand_request(), None);
        // Moving away and back doesn't ask again.
        press(&mut menu, 'j');
        press(&mut menu, 'k');
        assert_eq!(menu.take_expand_request(), None);
    }

    #[test]
    fn navigates_levels_and_remembers_the_cursor() {
        let mut menu = lazy_menu();
        menu.take_expand_request();
        menu.add_nodes(
            Some("src".into()),
            vec![node("src/a", Some("src"), false), node("src/b", Some("src"), false)],
        );
        press(&mut menu, 'l');
        assert_eq!(menu.path(), "src");
        press(&mut menu, 'j');
        assert_eq!(highlighted(&menu), "src/b");
        press(&mut menu, 'h');
        assert_eq!(highlighted(&menu), "src");
        press(&mut menu, 'l');
        assert_eq!(highlighted(&menu), "src/b", "the cursor is remembered");
        press(&mut menu, 'g');
        assert_eq!(highlighted(&menu), "src/a");
    }

    #[test]
    fn choosing_a_leaf_selects_it_and_closes() {
        let mut menu = lazy_menu();
        press(&mut menu, 'j');
        assert_eq!(
            press(&mut menu, 'l'),
            Some(Event::Selected {
                command: "open".into(),
                value: "README".into(),
            })
        );
        assert!(!menu.is_open());
    }

    #[test]
    fn full_trees_need_no_expansion_and_escape_closes() {
        let mut menu = TreeMenu::new();
        menu.open(TreeSpec {
            title: "t".into(),
            nodes: vec![node("a", None, true), node("a/b", Some("a"), false)],
            on_select: "pick".into(),
            on_expand: None,
        });
        assert_eq!(menu.take_expand_request(), None);
        press(&mut menu, 'l');
        assert_eq!(highlighted(&menu), "a/b");
        assert_eq!(
            menu.handle_key(KeyPress::plain(EditorKey::Escape)),
            Some(Event::Closed)
        );
    }
}
