mod components;
mod plugins;

use components::command_menu::{self, Candidate, CommandMenu};
use components::keys::{self, KeyChord, KeyPress};
use components::text_editor::{Selection, TextEditor};
use iced::widget::column;
use iced::{Element, Subscription, Task, Theme, keyboard};
use plugins::{Core, Event, EventKind, Lookup, PluginHost};

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

All of the above comes from the bundled core plugin. The text-tools plugin adds
:sort-lines (Alt-s), :duplicate-line (<space>d), :count-words (<space>c) and
:plugin-stats.
";

/// Commands provided by the app itself, because they control its UI.
const APP_COMMANDS: &[(&str, &str)] = &[
    ("command-menu", "Open the command menu"),
    ("quit", "Close Arrow"),
];

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

struct App {
    core: Core,
    plugins: PluginHost,
    menu: CommandMenu,
    /// Keys typed so far in an unfinished key sequence.
    pending_keys: Vec<KeyPress>,
    /// A command waiting for a typed character (like `r`).
    awaiting_char: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    Key(KeyPress),
}

/// Editor state before an action, to work out which events it caused.
struct Observed {
    /// Only captured when a plugin listens for buffer changes.
    text: Option<Vec<char>>,
    dots: Vec<Selection>,
    mode: String,
}

impl App {
    fn new() -> Self {
        Self::with_plugins(WELCOME, true)
    }

    /// An app with the bundled plugins, and the user's plugins if asked.
    fn with_plugins(text: &str, user_plugins: bool) -> Self {
        let mut core = Core {
            editor: TextEditor::new(text),
            app_commands: APP_COMMANDS.iter().map(|(name, _)| *name).collect(),
            ..Default::default()
        };
        let mut plugins = PluginHost::new();
        let mut errors = plugins.load_bundled(&mut core);
        if user_plugins {
            errors.extend(plugins.load_user_plugins(&mut core));
        }

        let mut app = Self {
            core,
            plugins,
            menu: CommandMenu::new(),
            pending_keys: Vec::new(),
            awaiting_char: None,
        };
        app.core.events.push_back(Event::Startup);
        let _ = app.follow_up();
        for error in errors {
            app.report(error);
        }
        app
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let Message::Key(press) = message;
        self.perform(|app| app.handle_key(press))
    }

    /// Run an action, then deliver the events it caused and any commands plugins
    /// queued along the way.
    fn perform(&mut self, action: impl FnOnce(&mut Self) -> Task<Message>) -> Task<Message> {
        let before = self.observe();
        let task = action(self);
        self.emit_changes(before);
        Task::batch([task, self.follow_up()])
    }

    /// Turn a key press into a command using the keymaps plugins registered.
    fn handle_key(&mut self, press: KeyPress) -> Task<Message> {
        // While the menu is open it gets every key.
        if self.menu.is_open() {
            return match self.menu.handle_key(press) {
                Some(command_menu::Event::Execute(name)) => self.run_command(&name, None),
                _ => Task::none(),
            };
        }

        // Messages and hints last until the next key.
        self.core.editor.editor_mut().clear_message();

        if let Some(command) = self.awaiting_char.take() {
            // Any key that doesn't type a character cancels.
            return match press.typed_char() {
                Some(c) => self.run_command(&command, Some(c)),
                None => Task::none(),
            };
        }

        self.pending_keys.push(press);
        let chords: Vec<KeyChord> = self.pending_keys.iter().map(|&p| p.into()).collect();
        let mode = self.core.editor.editor().mode().to_string();
        let registry = &self.core.registry;
        match registry.lookup(&mode, &chords) {
            Lookup::Command(command) => {
                self.pending_keys.clear();
                self.run_command(&command, None)
            }
            Lookup::Prefix => {
                let hint = registry.prefix_hint(&mode, &chords);
                self.core.editor.editor_mut().set_message(hint);
                Task::none()
            }
            Lookup::Unbound => {
                let in_sequence = self.pending_keys.len() > 1;
                self.pending_keys.clear();
                match (press.typed_char(), registry.fallback(&mode)) {
                    // An unknown key after a prefix just cancels the sequence.
                    _ if in_sequence => Task::none(),
                    // Typing modes send unbound characters to their fallback.
                    (Some(c), Some(fallback)) => {
                        let fallback = fallback.to_string();
                        self.run_command(&fallback, Some(c))
                    }
                    _ => Task::none(),
                }
            }
        }
    }

    /// Run a command by name: an app command or a plugin command.
    fn run_command(&mut self, name: &str, arg: Option<char>) -> Task<Message> {
        let task = match name {
            "quit" => iced::exit(),
            "command-menu" => {
                self.menu.open(self.candidates());
                Task::none()
            }
            _ => {
                let Some(command) = self.core.registry.command(name) else {
                    self.report(format!("no such command: {name}"));
                    return Task::none();
                };
                if command.takes_char && arg.is_none() {
                    self.awaiting_char = Some(name.to_string());
                    let hint = format!("{name}: type a character");
                    self.core.editor.editor_mut().set_message(hint);
                    return Task::none();
                }
                let plugin = command.plugin;
                if let Err(error) = self.plugins.run_command(plugin, &mut self.core, name, arg) {
                    self.report(error);
                }
                Task::none()
            }
        };
        self.core
            .events
            .push_back(Event::CommandExecuted(name.to_string()));
        task
    }

    fn observe(&self) -> Observed {
        let editor = self.core.editor.editor();
        Observed {
            text: self
                .core
                .registry
                .has_subscribers(EventKind::BufferChanged)
                .then(|| editor.chars().to_vec()),
            dots: editor.dots().to_vec(),
            mode: editor.mode().to_string(),
        }
    }

    fn emit_changes(&mut self, before: Observed) {
        let editor = self.core.editor.editor();
        let events = &mut self.core.events;
        if before.text.is_some_and(|text| text != editor.chars()) {
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
    fn follow_up(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for _ in 0..MAX_FOLLOW_UPS {
            if let Some(event) = self.core.events.pop_front() {
                self.dispatch(&event);
            } else if let Some(name) = self.core.queued_commands.pop_front() {
                let before = self.observe();
                tasks.push(self.run_command(&name, None));
                self.emit_changes(before);
            } else {
                return Task::batch(tasks);
            }
        }
        self.core.events.clear();
        self.core.queued_commands.clear();
        self.report(format!(
            "stopped after {MAX_FOLLOW_UPS} plugin events and commands; a plugin may be looping"
        ));
        Task::batch(tasks)
    }

    fn dispatch(&mut self, event: &Event) {
        let kind = match event {
            Event::Startup => EventKind::Startup,
            Event::BufferChanged => EventKind::BufferChanged,
            Event::SelectionChanged(_) => EventKind::SelectionChanged,
            Event::ModeChanged(_) => EventKind::ModeChanged,
            Event::CommandExecuted(_) => EventKind::CommandExecuted,
        };
        for plugin in self.core.registry.subscribers(kind) {
            if let Err(error) = self.plugins.send_event(plugin, &mut self.core, event) {
                self.report(error);
            }
        }
    }

    fn report(&mut self, error: String) {
        eprintln!("{error}");
        self.core.editor.editor_mut().set_message(error);
    }

    /// Everything the command menu can run, with key bindings where known.
    fn candidates(&self) -> Vec<Candidate> {
        let registry = &self.core.registry;
        let plugin_commands = registry.commands().into_iter().map(|(name, command)| {
            // Name the plugin, except for the core one that provides the basics.
            let plugin = self.plugins.name(command.plugin);
            let description = if plugin == "core" {
                command.description.clone()
            } else {
                format!("{} ({plugin})", command.description)
            };
            Candidate {
                name: name.to_string(),
                description,
                binding: registry.binding_for(name).unwrap_or_default(),
            }
        });
        let app = APP_COMMANDS.iter().map(|(name, description)| Candidate {
            name: name.to_string(),
            description: description.to_string(),
            binding: registry.binding_for(name).unwrap_or_default(),
        });
        plugin_commands.chain(app).collect()
    }

    fn view(&self) -> Element<'_, Message> {
        if self.menu.is_open() {
            column![self.core.editor.view(), self.menu.view()].into()
        } else {
            self.core.editor.view()
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        keyboard::listen().filter_map(key_message)
    }
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    keys::from_event(event).map(Message::Key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(text: &str) -> App {
        let app = App::with_plugins(text, false);
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
            let _ = app.update(Message::Key(KeyPress {
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

    // ----- Core plugin: editing behaviour -----

    #[test]
    fn core_plugin_provides_modes_commands_and_bindings() {
        let app = app("");
        assert_eq!(app.core.editor.editor().mode(), "normal");
        assert!(app.core.registry.fallback("insert").is_some());
        let candidates = app.candidates();
        assert!(candidates.len() > 60, "{} commands", candidates.len());
        let menu = candidates.iter().find(|c| c.name == "command-menu").unwrap();
        assert_eq!(menu.binding, ":");
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
        let candidates = app.candidates();
        let sort = candidates.iter().find(|c| c.name == "sort-lines").unwrap();
        assert_eq!(sort.binding, "<a-s>");
        assert!(sort.description.ends_with("(text-tools)"));
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

    #[test]
    fn event_hooks_see_commands_and_changes() {
        let mut app = app("one");
        type_keys(&mut app, "<space>d:plugin-stats<ret>");
        let stats = message(&app);
        // command-menu also counts as a command.
        assert!(
            stats.starts_with("seen: 2 commands (last: command-menu), 1 buffer changes"),
            "{stats}"
        );
    }
}
