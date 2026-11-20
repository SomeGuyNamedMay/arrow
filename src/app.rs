//! The window: hands key presses to the session, asynchronously.

use std::collections::VecDeque;

use iced::widget::column;
use iced::{Element, Subscription, Task, keyboard};

use crate::components::keys::{self, KeyPress};
use crate::components::menu::Menu;
use crate::components::text_editor::TextEditor;
use crate::components::tree_menu::TreeMenu;
use crate::session::Session;

/// Shown while plugins load, and the scratch buffer's text afterwards.
const WELCOME: &str = "\
Welcome to Arrow, a Kakoune-style editor.

Editing happens at dots: cursors that are also selections. c adds a dot on the
line below the newest one, and , removes the oldest. Every command below acts at
every dot.

Normal mode: h j k l move, w b e select words, x selects a line, % selects all.
Uppercase (H J K L W B E) extends a dot instead of moving it.
d deletes, s changes, y yanks, p pastes, u undoes, U redoes.
i or a enters insert mode, and Esc returns to normal mode.

Press : or Alt-x for the command menu. Type to filter, arrows or Ctrl-n/p to
choose, Tab to complete, Enter to run, Esc to close.

All of the above, the command menu included, comes from the bundled core plugin.
The menu itself is generic: any plugin can open one. The text-tools plugin adds
:sort-lines (Alt-s), :duplicate-line (<space>d), :count-words (<space>c) and
:plugin-stats.
";

/// The iced application. Editing runs asynchronously: each batch of key presses
/// is handled by a [`Task`] that owns the [`Session`] while plugins run, and
/// hands it back when they're done. Meanwhile the window keeps drawing the last
/// [`Frame`] and queues further keys, so a slow plugin never blocks the UI and
/// keys are still handled in the order they were pressed.
pub struct App {
    /// The editor and its plugins. `None` while a task has them.
    session: Option<Box<Session>>,
    /// What the session looked like when it left, drawn until it returns.
    frame: Frame,
    /// Keys pressed while the session was busy.
    inbox: VecDeque<KeyPress>,
}

pub enum Message {
    Key(KeyPress),
    /// A task finished with the session.
    Done(Box<Session>),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::Key(press) => f.debug_tuple("Key").field(press).finish(),
            Message::Done(_) => f.write_str("Done"),
        }
    }
}

/// A copy of everything the view draws. Cheap to take, because the text is a
/// rope whose clones share their storage.
struct Frame {
    editor: TextEditor,
    menu: Menu,
    tree: TreeMenu,
}

impl Frame {
    fn of(session: &Session) -> Self {
        Self {
            editor: session.core.editor.clone(),
            menu: session.menu.clone(),
            tree: session.tree.clone(),
        }
    }
}

impl App {
    /// Start with the welcome text on screen while plugins load in the background.
    pub fn new() -> (Self, Task<Message>) {
        let app = Self {
            session: None,
            frame: Frame {
                editor: TextEditor::new(WELCOME),
                menu: Menu::new(),
                tree: TreeMenu::new(),
            },
            inbox: VecDeque::new(),
        };
        let start = Task::perform(Session::start(WELCOME, true), |session| {
            Message::Done(Box::new(session))
        });
        (app, start)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Key(press) => self.inbox.push_back(press),
            Message::Done(session) => {
                if session.quit {
                    return iced::exit();
                }
                self.session = Some(session);
            }
        }
        self.dispatch()
    }

    /// If the session is free and keys are waiting, send them all to it.
    fn dispatch(&mut self) -> Task<Message> {
        if self.inbox.is_empty() {
            return Task::none();
        }
        let Some(mut session) = self.session.take() else {
            // Busy: the keys go with the next batch.
            return Task::none();
        };
        self.frame = Frame::of(&session);
        let presses: Vec<KeyPress> = self.inbox.drain(..).collect();
        Task::perform(
            async move {
                for press in presses {
                    session.press(press).await;
                    if session.quit {
                        break;
                    }
                }
                session
            },
            Message::Done,
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.session {
            Some(session) => view(&session.core.editor, &session.menu, &session.tree),
            None => view(&self.frame.editor, &self.frame.menu, &self.frame.tree),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        keyboard::listen().filter_map(key_message)
    }
}

fn view<'a>(editor: &'a TextEditor, menu: &'a Menu, tree: &'a TreeMenu) -> Element<'a, Message> {
    if menu.is_open() {
        column![editor.view(), menu.view()].into()
    } else if tree.is_open() {
        column![editor.view(), tree.view()].into()
    } else {
        editor.view()
    }
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    keys::from_event(event).map(Message::Key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::app;

    fn press(key: char) -> Message {
        let chord = keys::parse_keys(&key.to_string()).unwrap()[0];
        Message::Key(KeyPress {
            key: chord.key,
            alt: chord.alt,
            ctrl: chord.ctrl,
            shift: chord.shift,
        })
    }

    #[test]
    fn keys_pressed_while_the_session_is_busy_wait_their_turn() {
        let session = app("abc");
        let mut window = super::App {
            frame: Frame::of(&session),
            session: Some(Box::new(session)),
            inbox: VecDeque::new(),
        };
        // The first key sends the session off in a task; the view falls back to
        // the frame it left behind.
        let _ = window.update(press('i'));
        assert!(window.session.is_none());
        assert_eq!(window.frame.editor.editor().slice(0, 3), "abc");
        // Keys arriving meanwhile are kept, in order, for its return.
        let _ = window.update(press('x'));
        let _ = window.update(press('y'));
        assert_eq!(window.inbox.len(), 2);
    }
}
