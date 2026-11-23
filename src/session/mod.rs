//! The session: the editor, its plugins and its menus.
//!
//! A key press becomes a command through the keymaps plugins registered; the
//! command runs in its plugin; then the events it caused (buffer, selection and
//! mode changes) go to the plugins that subscribed, along with any commands
//! plugins queued, until nothing is left. Finally the syntax tree catches up.
//! Everything here is async, so the window runs it in a task off the UI thread.

use ropey::Rope;

use crate::components::keys::{KeyChord, KeyPress};
use crate::components::menu::{self, Menu};
use crate::components::text_editor::{Selection, TextEditor};
use crate::components::tree_menu::{self, TreeMenu};
use crate::config::user_config_dir;
use crate::plugins::{Core, Event, EventKind, Lookup, PluginHost};
use crate::syntax::{Languages, Syntax};

#[cfg(test)]
pub(crate) mod tests;
mod write;

/// Commands provided by the app itself: they control the window, or reach
/// outside the plugins' sandbox.
const APP_COMMANDS: &[(&str, &str)] = &[
    ("quit", "Close Arrow"),
    ("write", "Save the buffer to its file (or to the path given)"),
    ("terminal", "Show the terminal and type into it"),
    ("terminal-toggle", "Show or hide the terminal"),
];

/// What the window should do with the terminal, which it owns. Set by the
/// `terminal` commands, and taken by the window when the session comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalRequest {
    /// Show it (starting a shell if there isn't one) and give it the keys.
    Focus,
    /// Hide it if it's showing, otherwise show and focus it.
    Toggle,
}

/// Most events and queued commands handled after one action, so plugins that keep
/// triggering each other can't hang the editor.
const MAX_FOLLOW_UPS: usize = 1000;

pub struct Session {
    /// The editor and the registry, lent to plugins during their calls.
    pub(crate) core: Core,
    plugins: PluginHost,
    /// The menu plugins open (the command menu, and anything else that picks
    /// from a list).
    pub(crate) menu: Menu,
    /// The tree menu plugins open (like the file manager). It takes the same
    /// space as `menu`; only one of the two is open at a time.
    pub(crate) tree: TreeMenu,
    /// Keys typed so far in an unfinished key sequence.
    pending_keys: Vec<KeyPress>,
    /// A command waiting for a typed character (like `r`).
    awaiting_char: Option<String>,
    /// The `quit` command ran: close the window.
    pub(crate) quit: bool,
    /// A `terminal` command ran; the window acts on it.
    pub(crate) terminal_request: Option<TerminalRequest>,
    /// Parses the buffer for highlighting. `None` if tree-sitter couldn't start.
    syntax: Option<Syntax>,
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
    pub async fn start(text: &str, user_plugins: bool) -> Self {
        let mut core = Core::new(TextEditor::new(text), APP_COMMANDS.to_vec());
        let mut plugins = PluginHost::new();
        let mut errors = plugins.load_bundled(&mut core).await;
        if user_plugins {
            errors.extend(plugins.load_user_plugins(&mut core).await);
        }
        // User grammars come with user plugins: both are the user's setup.
        let languages = match user_config_dir().filter(|_| user_plugins) {
            Some(dir) => Languages::load(&dir).unwrap_or_else(|error| {
                errors.push(error);
                Languages::none()
            }),
            None => Languages::none(),
        };
        let syntax = Syntax::new(languages)
            .inspect_err(|error| errors.push(error.clone()))
            .ok();

        let mut session = Self {
            core,
            plugins,
            menu: Menu::new(),
            tree: TreeMenu::new(),
            pending_keys: Vec::new(),
            awaiting_char: None,
            quit: false,
            terminal_request: None,
            syntax,
        };
        session.core.events.push_back(Event::Startup);
        session.follow_up().await;
        session.sync_menus().await;
        session.update_syntax();
        // Edits plugins made while starting up (like core adding the final
        // newline) don't count as unsaved changes.
        session.core.editor.editor_mut().mark_saved();
        for error in errors {
            session.report(error);
        }
        session
    }

    /// Handle key presses in order until one asks the window to do something
    /// (quit, or show the terminal). Returns the presses not handled, which now
    /// belong to the window: keys typed after Ctrl-t go to the terminal.
    pub async fn press_all(&mut self, presses: Vec<KeyPress>) -> Vec<KeyPress> {
        let mut presses = presses.into_iter();
        for press in presses.by_ref() {
            self.press(press).await;
            if self.quit || self.terminal_request.is_some() {
                break;
            }
        }
        presses.collect()
    }

    /// Handle a key press, then deliver the events it caused and any commands
    /// plugins queued along the way.
    pub async fn press(&mut self, press: KeyPress) {
        let before = self.observe();
        self.handle_key(press).await;
        self.emit_changes(before);
        self.follow_up().await;
        self.sync_menus().await;
        self.update_syntax();
    }

    /// Reparse after the key's edits (and pick a language if a plugin renamed
    /// the buffer). This runs in the session's task, so off the UI thread.
    fn update_syntax(&mut self) {
        let Some(syntax) = &mut self.syntax else {
            return;
        };
        if let Err(error) = syntax.update(self.core.buffer_id, &mut self.core.editor) {
            self.report(error);
        }
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

    /// Turn a key press into a command, and run it.
    async fn handle_key(&mut self, press: KeyPress) {
        // An open menu or tree menu gets every key. Choosing an entry runs the
        // command its plugin named, with the entry's value.
        if self.menu.is_open() || self.tree.is_open() {
            if let Some((command, value)) = self.menu_key(press) {
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

        if let Some((command, arg)) = self.keymap_command(press) {
            self.run_command(&command, arg).await;
        }
    }

    /// Give a key to whichever menu is open. Returns the command and value of
    /// the entry it chose, if it chose one.
    fn menu_key(&mut self, press: KeyPress) -> Option<(String, String)> {
        if self.tree.is_open() {
            match self.tree.handle_key(press)? {
                tree_menu::Event::Selected { command, value } => Some((command, value)),
                tree_menu::Event::Closed => None,
            }
        } else {
            match self.menu.handle_key(press)? {
                menu::Event::Selected { command, value } => Some((command, value)),
                menu::Event::Closed => None,
            }
        }
    }

    /// Look the keys typed so far up in the current mode's keymap. Returns the
    /// command to run, and its argument, once a key completes a binding (or is a
    /// character for the mode's fallback); a key that starts a longer binding
    /// shows what can follow instead.
    fn keymap_command(&mut self, press: KeyPress) -> Option<(String, Option<String>)> {
        self.pending_keys.push(press);
        let chords: Vec<KeyChord> = self.pending_keys.iter().map(|&p| p.into()).collect();
        let mode = self.core.editor.editor().mode().to_string();
        let registry = &self.core.registry;
        match registry.lookup(&mode, &chords) {
            Lookup::Command(command) => {
                self.pending_keys.clear();
                Some((command, None))
            }
            Lookup::Prefix => {
                let hint = registry.prefix_hint(&mode, &chords);
                self.core.editor.editor_mut().set_message(hint);
                None
            }
            Lookup::Unbound => {
                let in_sequence = self.pending_keys.len() > 1;
                self.pending_keys.clear();
                // An unknown key after a prefix just cancels the sequence.
                if in_sequence {
                    return None;
                }
                // Typing modes send unbound characters to their fallback.
                let fallback = registry.fallback(&mode)?.to_string();
                Some((fallback, Some(press.typed_char()?.to_string())))
            }
        }
    }

    /// Run a command by name: an app command or a plugin command.
    async fn run_command(&mut self, name: &str, arg: Option<String>) {
        match name {
            "quit" => self.quit = true,
            "terminal" => self.terminal_request = Some(TerminalRequest::Focus),
            "terminal-toggle" => self.terminal_request = Some(TerminalRequest::Toggle),
            "write" => {
                let editor = self.core.editor.editor_mut();
                match write::write_buffer(editor, arg.as_deref()) {
                    Ok(path) => editor.set_message(format!("wrote {path}")),
                    Err(error) => self.report(error),
                }
            }
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
