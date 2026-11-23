//! The window: hands key presses to the session, asynchronously, and owns the
//! terminal panel under the status bar.

use std::collections::VecDeque;

use iced::widget::column;
use iced::{Element, Subscription, Task, keyboard};

use crate::components::keys::{self, KeyPress};
use crate::components::menu::Menu;
use crate::components::text_editor::TextEditor;
use crate::components::tree_menu::TreeMenu;
use crate::config::user_config_dir;
use crate::session::{Session, TerminalRequest};
use crate::terminal::{self, Panel, Started, TerminalConfig};

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
    /// Keys waiting their turn: for the session while it is busy, or for the
    /// terminal once a batch has handed it the keys.
    inbox: VecDeque<KeyPress>,
    /// The terminal under the status bar. It lives here rather than in the
    /// session so it keeps running, and drawing, while plugins work.
    terminal: Panel,
}

pub enum Message {
    Key(KeyPress),
    /// A task finished with the session, with the key presses it left
    /// unhandled (those after a `terminal` command).
    Done(Box<Session>, Vec<KeyPress>),
    /// Something the shell with this generation reported.
    Terminal(u64, terminal::Event),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::Key(press) => f.debug_tuple("Key").field(press).finish(),
            Message::Done(_, rest) => f.debug_tuple("Done").field(rest).finish(),
            Message::Terminal(generation, _) => f.debug_tuple("Terminal").field(generation).finish(),
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
        let mut startup_error = None;
        let terminal_config = match user_config_dir() {
            Some(dir) => TerminalConfig::load(&dir).unwrap_or_else(|error| {
                startup_error = Some(error);
                TerminalConfig::default()
            }),
            None => TerminalConfig::default(),
        };
        let app = Self {
            session: None,
            frame: Frame {
                editor: TextEditor::new(WELCOME),
                menu: Menu::new(),
                tree: TreeMenu::new(),
            },
            inbox: VecDeque::new(),
            terminal: Panel::new(terminal_config),
        };
        let start = Task::perform(Session::start(WELCOME, true), move |mut session| {
            if let Some(error) = &startup_error {
                session.core.editor.editor_mut().set_message(error.clone());
            }
            Message::Done(Box::new(session), Vec::new())
        });
        (app, start)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let mut tasks = Vec::new();
        match message {
            Message::Key(press) => self.inbox.push_back(press),
            Message::Done(mut session, rest) => {
                if session.quit {
                    return iced::exit();
                }
                let request = session.terminal_request.take();
                self.session = Some(session);
                // Keys after the request come next, before any typed since.
                for press in rest.into_iter().rev() {
                    self.inbox.push_front(press);
                }
                let started = match request {
                    Some(TerminalRequest::Focus) => self.terminal.focus(),
                    Some(TerminalRequest::Toggle) => self.terminal.toggle(),
                    None => None,
                };
                tasks.push(stream(started));
            }
            Message::Terminal(generation, event) => self.terminal.handle(generation, event),
        }
        tasks.push(self.pump());
        self.terminal.sync_size();
        Task::batch(tasks)
    }

    /// Hand waiting keys to whoever has them: the terminal, one by one, while
    /// it has focus, then the session, as a batch.
    fn pump(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        while self.terminal.focused() {
            let Some(press) = self.inbox.pop_front() else {
                break;
            };
            tasks.push(stream(self.terminal.press(press)));
        }
        if !self.terminal.focused() {
            tasks.push(self.dispatch());
        }
        Task::batch(tasks)
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
                let rest = session.press_all(presses).await;
                (session, rest)
            },
            |(session, rest)| Message::Done(session, rest),
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        let main = match &self.session {
            Some(session) => view(&session.core.editor, &session.menu, &session.tree),
            None => view(&self.frame.editor, &self.frame.menu, &self.frame.tree),
        };
        match self.terminal.view() {
            Some(terminal) => column![main, terminal].into(),
            None => main,
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

/// Turn a newly started shell's events into messages, for as long as it runs.
fn stream(started: Started) -> Task<Message> {
    match started {
        Some((generation, events)) => {
            Task::run(events, move |event| Message::Terminal(generation, event))
        }
        None => Task::none(),
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
            terminal: Panel::new(TerminalConfig::default()),
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
