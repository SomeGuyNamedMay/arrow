mod components;
mod plugins;

use std::collections::VecDeque;

use components::keys::{self, KeyChord, KeyPress};
use components::menu::{self, Menu};
use components::tree_menu::{self, TreeMenu};
use components::text_editor::{Selection, TextEditor};
use iced::widget::column;
use iced::{Element, Subscription, Task, Theme, keyboard};
use plugins::{Core, Event, EventKind, Lookup, PluginHost};
use ropey::Rope;

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

/// Commands provided by the app itself, because they control the window.
const APP_COMMANDS: &[(&str, &str)] = &[("quit", "Close Arrow")];

/// Most events and queued commands handled after one action, so plugins that keep
/// triggering each other can't hang the editor.
const MAX_FOLLOW_UPS: usize = 1000;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Arrow")
        .theme(Theme::Dark)
        .subscription(App::subscription)
        .window_size((900.0, 600.0))
        .centered()
        .run()
}

// ----- The window: hands key presses to the session, asynchronously -----

/// The iced application. Editing runs asynchronously: each batch of key presses
/// is handled by a [`Task`] that owns the [`Session`] while plugins run, and
/// hands it back when they're done. Meanwhile the window keeps drawing the last
/// [`Frame`] and queues further keys, so a slow plugin never blocks the UI and
/// keys are still handled in the order they were pressed.
struct App {
    /// The editor and its plugins. `None` while a task has them.
    session: Option<Box<Session>>,
    /// What the session looked like when it left, drawn until it returns.
    frame: Frame,
    /// Keys pressed while the session was busy.
    inbox: VecDeque<KeyPress>,
}

enum Message {
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

impl App {
    /// Start with the welcome text on screen while plugins load in the background.
    fn new() -> (Self, Task<Message>) {
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

    fn update(&mut self, message: Message) -> Task<Message> {
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
        self.frame = session.frame();
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

    fn view(&self) -> Element<'_, Message> {
        match &self.session {
            Some(session) => view(&session.core.editor, &session.menu, &session.tree),
            None => view(&self.frame.editor, &self.frame.menu, &self.frame.tree),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
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

// ----- The session: the editor, its plugins and its menus -----

struct Session {
    core: Core,
    plugins: PluginHost,
    /// The menu plugins open (the command menu, and anything else that picks
    /// from a list).
    menu: Menu,
    /// The tree menu plugins open (like the file manager). It takes the same
    /// space as `menu`; only one of the two is open at a time.
    tree: TreeMenu,
    /// Keys typed so far in an unfinished key sequence.
    pending_keys: Vec<KeyPress>,
    /// A command waiting for a typed character (like `r`).
    awaiting_char: Option<String>,
    /// The `quit` command ran: close the window.
    quit: bool,
}

/// Editor state before an action, to work out which events it caused.
struct Observed {
    /// Only captured when a plugin listens for buffer changes. Taking it is
    /// cheap (a rope clone); comparing it afterwards is the costly part.
    text: Option<Rope>,
    dots: Vec<Selection>,
    mode: String,
}

impl Session {
    /// A session with the bundled plugins, and the user's plugins if asked.
    async fn start(text: &str, user_plugins: bool) -> Self {
        let mut core = Core {
            editor: TextEditor::new(text),
            app_commands: APP_COMMANDS.to_vec(),
            ..Default::default()
        };
        let mut plugins = PluginHost::new();
        let mut errors = plugins.load_bundled(&mut core).await;
        if user_plugins {
            errors.extend(plugins.load_user_plugins(&mut core).await);
        }

        let mut session = Self {
            core,
            plugins,
            menu: Menu::new(),
            tree: TreeMenu::new(),
            pending_keys: Vec::new(),
            awaiting_char: None,
            quit: false,
        };
        session.core.events.push_back(Event::Startup);
        session.follow_up().await;
        session.sync_menus().await;
        for error in errors {
            session.report(error);
        }
        session
    }

    fn frame(&self) -> Frame {
        Frame {
            editor: self.core.editor.clone(),
            menu: self.menu.clone(),
            tree: self.tree.clone(),
        }
    }

    /// Handle a key press, then deliver the events it caused and any commands
    /// plugins queued along the way.
    async fn press(&mut self, press: KeyPress) {
        let before = self.observe();
        self.handle_key(press).await;
        self.emit_changes(before);
        self.follow_up().await;
        self.sync_menus().await;
    }

    /// Bring the menus up to date with what plugins asked for: open a requested
    /// menu or tree menu, add tree nodes, and ask plugins for the children of
    /// tree branches the tree menu needs, repeating until nothing is pending.
    async fn sync_menus(&mut self) {
        for _ in 0..MAX_FOLLOW_UPS {
            if let Some(spec) = self.core.menu_request.take() {
                self.tree.close();
                self.menu.open(spec);
            }
            if let Some(spec) = self.core.tree_request.take() {
                self.menu = Menu::new();
                self.tree.open(spec);
            }
            for (parent, nodes) in std::mem::take(&mut self.core.tree_additions) {
                self.tree.add_nodes(parent, nodes);
            }
            let Some((command, value)) = self.tree.take_expand_request() else {
                break;
            };
            let before = self.observe();
            self.run_command(&command, Some(value)).await;
            self.emit_changes(before);
            self.follow_up().await;
        }
    }

    /// Turn a key press into a command using the keymaps plugins registered.
    async fn handle_key(&mut self, press: KeyPress) {
        // While the tree menu is open it gets every key. Choosing a leaf runs the
        // command the plugin named, with the leaf's value.
        if self.tree.is_open() {
            if let Some(tree_menu::Event::Selected { command, value }) = self.tree.handle_key(press) {
                self.run_command(&command, Some(value)).await;
            }
            return;
        }

        // While a menu is open it gets every key. Choosing an item runs the
        // command the plugin named, with the item's value.
        if self.menu.is_open() {
            if let Some(menu::Event::Selected { command, value }) = self.menu.handle_key(press) {
                self.run_command(&command, Some(value)).await;
            }
            return;
        }

        // Messages and hints last until the next key.
        self.core.editor.editor_mut().clear_message();

        if let Some(command) = self.awaiting_char.take() {
            // Any key that doesn't type a character cancels.
            if let Some(c) = press.typed_char() {
                self.run_command(&command, Some(c.to_string())).await;
            }
            return;
        }

        self.pending_keys.push(press);
        let chords: Vec<KeyChord> = self.pending_keys.iter().map(|&p| p.into()).collect();
        let mode = self.core.editor.editor().mode().to_string();
        let registry = &self.core.registry;
        match registry.lookup(&mode, &chords) {
            Lookup::Command(command) => {
                self.pending_keys.clear();
                self.run_command(&command, None).await;
            }
            Lookup::Prefix => {
                let hint = registry.prefix_hint(&mode, &chords);
                self.core.editor.editor_mut().set_message(hint);
            }
            Lookup::Unbound => {
                let in_sequence = self.pending_keys.len() > 1;
                self.pending_keys.clear();
                let fallback = registry.fallback(&mode).map(str::to_string);
                match (press.typed_char(), fallback) {
                    // An unknown key after a prefix just cancels the sequence.
                    _ if in_sequence => {}
                    // Typing modes send unbound characters to their fallback.
                    (Some(c), Some(fallback)) => {
                        self.run_command(&fallback, Some(c.to_string())).await;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Run a command by name: an app command or a plugin command.
    async fn run_command(&mut self, name: &str, arg: Option<String>) {
        match name {
            "quit" => self.quit = true,
            _ => {
                let Some(command) = self.core.registry.command(name) else {
                    self.report(format!("no such command: {name}"));
                    return;
                };
                if command.takes_char && arg.is_none() {
                    self.awaiting_char = Some(name.to_string());
                    let hint = format!("{name}: type a character");
                    self.core.editor.editor_mut().set_message(hint);
                    return;
                }
                let plugin = command.plugin;
                let arg = arg.as_deref();
                let result = self.plugins.run_command(plugin, &mut self.core, name, arg).await;
                if let Err(error) = result {
                    self.report(error);
                }
            }
        }
        self.core
            .events
            .push_back(Event::CommandExecuted(name.to_string()));
    }

    fn observe(&self) -> Observed {
        let editor = self.core.editor.editor();
        Observed {
            text: self
                .core
                .registry
                .has_subscribers(EventKind::BufferChanged)
                .then(|| editor.rope().clone()),
            dots: editor.dots().to_vec(),
            mode: editor.mode().to_string(),
        }
    }

    fn emit_changes(&mut self, before: Observed) {
        let editor = self.core.editor.editor();
        let events = &mut self.core.events;
        if before.text.is_some_and(|text| text != *editor.rope()) {
            events.push_back(Event::BufferChanged);
        }
        if editor.dots() != before.dots {
            events.push_back(Event::SelectionChanged(editor.selection().into()));
        }
        if editor.mode() != before.mode {
            events.push_back(Event::ModeChanged(editor.mode().to_string()));
        }
    }

    /// Deliver queued events and run queued commands until both queues are empty.
    /// Edits made by event hooks don't cause further events; queued commands do.
    async fn follow_up(&mut self) {
        for _ in 0..MAX_FOLLOW_UPS {
            if let Some(event) = self.core.events.pop_front() {
                self.dispatch(&event).await;
            } else if let Some((name, arg)) = self.core.queued_commands.pop_front() {
                let before = self.observe();
                self.run_command(&name, arg).await;
                self.emit_changes(before);
            } else {
                return;
            }
        }
        self.core.events.clear();
        self.core.queued_commands.clear();
        self.report(format!(
            "stopped after {MAX_FOLLOW_UPS} plugin events and commands; a plugin may be looping"
        ));
    }

    async fn dispatch(&mut self, event: &Event) {
        let kind = match event {
            Event::Startup => EventKind::Startup,
            Event::BufferChanged => EventKind::BufferChanged,
            Event::SelectionChanged(_) => EventKind::SelectionChanged,
            Event::ModeChanged(_) => EventKind::ModeChanged,
            Event::CommandExecuted(_) => EventKind::CommandExecuted,
        };
        for plugin in self.core.registry.subscribers(kind) {
            if let Err(error) = self.plugins.send_event(plugin, &mut self.core, event).await {
                self.report(error);
            }
        }
    }

    fn report(&mut self, error: String) {
        eprintln!("{error}");
        self.core.editor.editor_mut().set_message(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests play the window's part: they drive the session's futures to
    /// completion, one key at a time.
    type App = Session;

    fn block_on<F: Future>(future: F) -> F::Output {
        thread_local! {
            static RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a tokio runtime for the tests");
        }
        RUNTIME.with(|runtime| runtime.block_on(future))
    }

    fn app(text: &str) -> App {
        let app = block_on(Session::start(text, false));
        assert!(
            message(&app).starts_with("text-tools loaded"),
            "plugins should load cleanly: {}",
            message(&app)
        );
        app
    }

    /// Type keys written in Kakoune notation, e.g. "wd", "itext<esc>".
    fn type_keys(app: &mut App, keys: &str) {
        for chord in keys::parse_keys(keys).unwrap() {
            block_on(app.press(KeyPress {
                key: chord.key,
                alt: chord.alt,
                ctrl: chord.ctrl,
                shift: chord.shift,
            }));
        }
    }

    fn text(app: &App) -> String {
        let editor = app.core.editor.editor();
        editor.slice(0, editor.len())
    }

    fn selected(app: &App) -> String {
        let editor = app.core.editor.editor();
        let selection = editor.selection();
        editor.slice(selection.start(), selection.end() + 1)
    }

    fn message(app: &App) -> String {
        let message = app.core.editor.editor().message();
        message.unwrap_or_default().to_string()
    }

    fn position(app: &App) -> (usize, usize) {
        app.core.editor.editor().cursor_position()
    }

    /// The items in the open menu, in display order.
    fn menu_items(app: &App) -> Vec<menu::MenuItem> {
        app.menu.matching_items().into_iter().cloned().collect()
    }

    fn menu_item(app: &App, label: &str) -> menu::MenuItem {
        menu_items(app)
            .into_iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("no menu item `{label}`"))
    }

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
            frame: session.frame(),
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

    // ----- Core plugin: editing behaviour -----

    #[test]
    fn core_plugin_provides_modes_commands_and_bindings() {
        let mut app = app("");
        assert_eq!(app.core.editor.editor().mode(), "normal");
        assert!(app.core.registry.fallback("insert").is_some());
        // The command menu comes from core too, through the generic menu.
        type_keys(&mut app, ":");
        assert!(app.menu.is_open());
        let items = menu_items(&app);
        assert!(items.len() > 60, "{} commands", items.len());
        assert_eq!(menu_item(&app, "command-menu").annotation, ":");
        assert_eq!(menu_item(&app, "quit").detail, "Close Arrow (app)");
        type_keys(&mut app, "<esc>");
        assert!(!app.menu.is_open());
    }

    #[test]
    fn describe_bindings_lists_and_explains_every_binding() {
        let mut app = app("abc");
        type_keys(&mut app, "?");
        assert!(app.menu.is_open());
        let items = menu_items(&app);
        // Normal-mode bindings come first, then insert mode.
        assert_eq!(items[0].annotation, "normal");
        assert_eq!(items.last().unwrap().annotation, "insert");
        // Bindings from other plugins are listed and named.
        let duplicate = items
            .iter()
            .find(|item| item.label.ends_with("duplicate-line"))
            .unwrap();
        assert!(duplicate.label.starts_with("<space>d"), "{}", duplicate.label);
        assert!(duplicate.detail.ends_with("(text-tools)"), "{}", duplicate.detail);

        // Search by keys and command name (goto-line-start is also on <home>);
        // choosing explains the binding without running it.
        type_keys(&mut app, "gh<space>goto<ret>");
        assert!(!app.menu.is_open());
        assert_eq!(
            message(&app),
            "gh in normal mode runs goto-line-start: Go to the start of the line"
        );
        assert_eq!(text(&app), "abc\n");
    }

    #[test]
    fn command_menu_lists_recent_commands_first() {
        let mut app = app("abc");
        type_keys(&mut app, ":select-line<ret>");
        assert_eq!(selected(&app), "abc\n");
        type_keys(&mut app, ":");
        assert_eq!(menu_items(&app)[0].label, "select-line");
        // A takes-char command chosen from the menu still waits for its character.
        type_keys(&mut app, "<c-backspace>replace-chars<ret>");
        assert_eq!(message(&app), "replace-chars: type a character");
        type_keys(&mut app, "z");
        assert_eq!(text(&app), "zzz\n");
    }

    #[test]
    fn word_motion_selects_word_and_trailing_space() {
        let mut app = app("hello world");
        type_keys(&mut app, "w");
        assert_eq!(selected(&app), "hello ");
        type_keys(&mut app, "w");
        assert_eq!(selected(&app), "world");
        type_keys(&mut app, "<a-;>");
        assert_eq!(app.core.editor.editor().selection().cursor, 6);
    }

    #[test]
    fn delete_word_and_undo() {
        let mut app = app("hello world");
        type_keys(&mut app, "wd");
        assert_eq!(text(&app), "world\n");
        type_keys(&mut app, "u");
        assert_eq!(text(&app), "hello world\n");
        type_keys(&mut app, "U");
        assert_eq!(text(&app), "world\n");
    }

    #[test]
    fn insert_session_types_and_undoes_as_one_step() {
        let mut app = app("hello world");
        type_keys(&mut app, "eshi<esc>");
        assert_eq!(text(&app), "hi world\n");
        assert_eq!(app.core.editor.editor().mode(), "normal");
        type_keys(&mut app, "u");
        assert_eq!(text(&app), "hello world\n");
    }

    #[test]
    fn insert_mode_newlines_keep_indent() {
        let mut app = app("    foo");
        type_keys(&mut app, "obar<ret>baz<esc>");
        assert_eq!(text(&app), "    foo\n    bar\n    baz\n");
    }

    #[test]
    fn line_selection_yank_and_paste() {
        let mut app = app("one\ntwo\nthree");
        type_keys(&mut app, "x");
        assert_eq!(selected(&app), "one\n");
        type_keys(&mut app, "x");
        assert_eq!(selected(&app), "one\ntwo\n");
        type_keys(&mut app, ";xyp");
        assert_eq!(text(&app), "one\ntwo\ntwo\nthree\n");
    }

    #[test]
    fn replace_whole_buffer_with_yanked_line() {
        let mut app = app("one\ntwo");
        type_keys(&mut app, "xy%R");
        assert_eq!(text(&app), "one\n");
    }

    #[test]
    fn vertical_motion_remembers_column() {
        let mut app = app("abcdef\nab\nabcdef");
        type_keys(&mut app, "glj");
        assert_eq!(position(&app), (1, 2));
        type_keys(&mut app, "j");
        assert_eq!(position(&app), (2, 5));
        type_keys(&mut app, "hk");
        assert_eq!(position(&app), (1, 2));
    }

    #[test]
    fn replace_chars_waits_for_a_character() {
        let mut app = app("abc");
        type_keys(&mut app, "Lr");
        assert_eq!(message(&app), "replace-chars: type a character");
        type_keys(&mut app, "x");
        assert_eq!(text(&app), "xxc\n");
        type_keys(&mut app, "%~");
        assert_eq!(text(&app), "XXC\n");
    }

    #[test]
    fn indent_and_dedent() {
        let mut app = app("a\nb");
        type_keys(&mut app, "xx>");
        assert_eq!(text(&app), "    a\n    b\n");
        type_keys(&mut app, "<lt>");
        assert_eq!(text(&app), "a\nb\n");
    }

    #[test]
    fn prefix_shows_hint_and_unknown_key_cancels() {
        let mut app = app("abc");
        type_keys(&mut app, "g");
        let hint = message(&app);
        assert!(hint.starts_with("goto: h goto-line-start"), "{hint}");
        type_keys(&mut app, "z");
        assert_eq!(message(&app), "");
        assert_eq!(text(&app), "abc\n");
    }

    // ----- Dots -----

    fn dots(app: &App) -> Vec<Selection> {
        app.core.editor.editor().dots().to_vec()
    }

    #[test]
    fn c_spawns_dots_below_and_edits_happen_at_every_dot() {
        let mut app = app("abc\ndef\nghi");
        type_keys(&mut app, "lcc");
        assert_eq!(
            dots(&app),
            [Selection::point(1), Selection::point(5), Selection::point(9)]
        );
        type_keys(&mut app, "iX<esc>");
        assert_eq!(text(&app), "aXbc\ndXef\ngXhi\n");
        // The whole multi-dot insert undoes as one step.
        type_keys(&mut app, "u");
        assert_eq!(text(&app), "abc\ndef\nghi\n");
    }

    #[test]
    fn spawning_past_the_last_line_says_so() {
        let mut app = app("abc\ndef");
        type_keys(&mut app, "cc");
        assert_eq!(dots(&app).len(), 2);
        assert_eq!(message(&app), "no line below for a new dot");
    }

    #[test]
    fn comma_removes_the_oldest_dot() {
        let mut app = app("abc\ndef\nghi");
        type_keys(&mut app, "cc,");
        assert_eq!(dots(&app), [Selection::point(4), Selection::point(8)]);
        type_keys(&mut app, "d");
        assert_eq!(text(&app), "abc\nef\nhi\n");
        type_keys(&mut app, ",,");
        assert_eq!(dots(&app).len(), 1);
        assert_eq!(message(&app), "only one dot");
    }

    #[test]
    fn dots_grow_into_selections() {
        let mut app = app("one two\nthree four");
        type_keys(&mut app, "cw");
        assert_eq!(
            dots(&app),
            [
                Selection {
                    anchor: 0,
                    cursor: 3
                },
                Selection {
                    anchor: 8,
                    cursor: 13
                }
            ]
        );
        type_keys(&mut app, "~");
        assert_eq!(text(&app), "ONE two\nTHREE four\n");
        // `s` changes the text of every dot.
        type_keys(&mut app, "sX<esc>");
        assert_eq!(text(&app), "Xtwo\nXfour\n");
    }

    #[test]
    fn each_dot_pastes_its_own_yank() {
        let mut app = app("ab\ncd");
        type_keys(&mut app, "cyp");
        assert_eq!(text(&app), "aab\nccd\n");
    }

    #[test]
    fn dots_that_meet_merge() {
        let mut app = app("abc\ndef");
        type_keys(&mut app, "ck");
        assert_eq!(dots(&app), [Selection::point(0)]);
    }

    // ----- text-tools plugin: commands, bindings and hooks -----

    #[test]
    fn plugin_command_runs_from_the_menu_and_undoes_in_one_step() {
        let mut app = app("b\na\nc");
        type_keys(&mut app, "%:sort-lines<ret>");
        assert!(!app.menu.is_open());
        assert_eq!(text(&app), "a\nb\nc\n");
        type_keys(&mut app, "u");
        assert_eq!(text(&app), "b\na\nc\n");
    }

    #[test]
    fn plugin_key_bindings() {
        let mut app = app("one\ntwo");
        type_keys(&mut app, "<space>d");
        assert_eq!(text(&app), "one\none\ntwo\n");
        // Bound by text-tools to a command from core.
        type_keys(&mut app, "<space>a");
        assert_eq!(selected(&app), text(&app));
        type_keys(&mut app, ":");
        let sort = menu_item(&app, "sort-lines");
        assert_eq!(sort.annotation, "<a-s>");
        assert!(sort.detail.ends_with("(text-tools)"), "{}", sort.detail);
    }

    #[test]
    fn other_plugins_edits_keep_dots_in_place_and_undo_separately() {
        let mut app = app("ab\ncd\nef");
        // Dots on "c" and "e"; then text-tools duplicates the primary dot's line.
        type_keys(&mut app, "jc<space>d");
        assert_eq!(text(&app), "ab\ncd\nef\nef\n");
        // The older dot is still on "c".
        assert_eq!(dots(&app)[0], Selection::point(3));
        type_keys(&mut app, "x");
        type_keys(&mut app, "u");
        assert_eq!(text(&app), "ab\ncd\nef\n");
    }

    // ----- files plugin: the tree menu -----

    /// Move the file manager's highlight down to the node with this value.
    fn highlight(app: &mut App, value: &str) {
        for _ in 0..50 {
            if app.tree.highlighted_value() == Some(value) {
                return;
            }
            type_keys(app, "j");
        }
        panic!("`{value}` not found in the file manager");
    }

    #[test]
    fn file_manager_browses_and_opens_files() {
        let mut app = app("scratch");
        type_keys(&mut app, "<space>f");
        assert!(app.tree.is_open());
        // Directories come first, sorted by name; hidden ones like .git are left out.
        assert_eq!(app.tree.highlighted_value(), Some("plugins"));

        highlight(&mut app, "src");
        type_keys(&mut app, "l");
        highlight(&mut app, "src/main.rs");
        // h goes back up and l returns to the same place.
        type_keys(&mut app, "hl");
        assert_eq!(app.tree.highlighted_value(), Some("src/main.rs"));

        type_keys(&mut app, "<ret>");
        assert!(!app.tree.is_open());
        assert!(text(&app).starts_with("mod components;"), "{}", &text(&app)[..40]);
        assert_eq!(app.core.editor.editor().name(), "src/main.rs");
        assert_eq!(message(&app), "opened src/main.rs");

        // Loading a file is an undoable change like any other.
        type_keys(&mut app, "u");
        assert_eq!(text(&app), "scratch\n");
    }

    #[test]
    fn file_manager_closes_with_escape_and_only_it_can_read_files() {
        let mut app = app("");
        type_keys(&mut app, "<space>f<esc>");
        assert!(!app.tree.is_open());
        // Plugins without file access can't open files: the command belongs to
        // the files plugin, which runs it in its own sandbox.
        type_keys(&mut app, ":open-file<ret>");
        assert!(message(&app).contains("needs a path"), "{}", message(&app));
    }

    #[test]
    fn event_hooks_see_commands_and_changes() {
        let mut app = app("one");
        type_keys(&mut app, "<space>d:plugin-stats<ret>");
        let stats = message(&app);
        // Opening the menu (command-menu) and choosing from it (execute-command,
        // which then runs plugin-stats) are commands too.
        assert!(
            stats.starts_with("seen: 3 commands (last: execute-command), 1 buffer changes"),
            "{stats}"
        );
    }
}
