//! The terminal as the window sees it: whether it's showing, whether it has
//! the keys, and the shell behind it, which outlives both.

use std::cell::Cell;

use iced::Element;
use iced::widget::{container, text};

use super::{Event, Events, Terminal, TerminalConfig};
use crate::components::keys::{EditorKey, KeyPress};

/// A newly started shell's events, tagged with its generation.
pub type Started = Option<(u64, Events)>;

pub struct Panel {
    config: TerminalConfig,
    /// Started the first time the panel is shown, then kept.
    terminal: Option<Terminal>,
    shown: bool,
    focused: bool,
    /// Counts shells started, so events from one that was replaced are ignored.
    generation: u64,
    /// The size the view last measured, as (rows, cols).
    measured: Cell<(u16, u16)>,
    /// Why the shell couldn't start, shown in the panel.
    error: Option<String>,
}

impl Panel {
    pub fn new(config: TerminalConfig) -> Self {
        let rows = config.rows;
        Self {
            config,
            terminal: None,
            shown: false,
            focused: false,
            generation: 0,
            measured: Cell::new((rows, 80)),
            error: None,
        }
    }

    pub fn focused(&self) -> bool {
        self.shown && self.focused
    }

    /// Show the panel and give it the keys, starting a shell if there is none
    /// (or the last one exited).
    pub fn focus(&mut self) -> Started {
        self.shown = true;
        self.focused = true;
        self.start_if_needed()
    }

    /// Hide the panel if it's showing; otherwise show and focus it.
    pub fn toggle(&mut self) -> Started {
        if self.shown {
            self.hide();
            None
        } else {
            self.focus()
        }
    }

    /// Hide the panel. The shell keeps running.
    pub fn hide(&mut self) {
        self.shown = false;
        self.focused = false;
    }

    /// A key press while the panel has the keys. Ctrl-t gives the keys back to
    /// the editor and Alt-t hides the panel; those two never reach the shell.
    /// After the shell has exited, any key starts a new one.
    pub fn press(&mut self, press: KeyPress) -> Started {
        let chord = |c: char, ctrl: bool, alt: bool| {
            press.key == EditorKey::Char(c) && press.ctrl == ctrl && press.alt == alt
        };
        if chord('t', true, false) {
            self.focused = false;
            return None;
        }
        if chord('t', false, true) {
            self.hide();
            return None;
        }
        match &mut self.terminal {
            Some(terminal) if !terminal.exited() => {
                terminal.press(press);
                None
            }
            _ => self.start_if_needed(),
        }
    }

    /// Something the shell with this generation reported.
    pub fn handle(&mut self, generation: u64, event: Event) {
        let Some(terminal) = self.terminal.as_mut().filter(|_| generation == self.generation) else {
            return;
        };
        match event {
            Event::Output(bytes) => terminal.feed(&bytes),
            Event::Exited => terminal.mark_exited(),
        }
    }

    /// Resize the shell to what the view last measured.
    pub fn sync_size(&mut self) {
        let (rows, cols) = self.measured.get();
        if let Some(terminal) = &mut self.terminal {
            terminal.resize(rows, cols);
        }
    }

    fn start_if_needed(&mut self) -> Started {
        if self.terminal.as_ref().is_some_and(|t| !t.exited()) {
            return None;
        }
        // Drop the old shell first, so its transcript is closed before the new
        // one replays it.
        self.terminal = None;
        let (rows, cols) = self.measured.get();
        match Terminal::spawn(&self.config, rows, cols) {
            Ok((terminal, events)) => {
                self.terminal = Some(terminal);
                self.error = None;
                self.generation += 1;
                Some((self.generation, events))
            }
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }

    /// The panel, if it's showing.
    pub fn view<'a, Message: 'a>(&'a self) -> Option<Element<'a, Message>> {
        if !self.shown {
            return None;
        }
        Some(match (&self.terminal, &self.error) {
            (_, Some(error)) => container(text(format!("terminal: {error}")).size(13))
                .padding([4.0, 8.0])
                .into(),
            (Some(terminal), None) => {
                super::view(terminal, self.focused, self.config.rows, &self.measured)
            }
            (None, None) => container(text("terminal: starting…").size(13)).padding([4.0, 8.0]).into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn key(c: char) -> KeyPress {
        KeyPress::plain(EditorKey::Char(c))
    }

    fn ctrl(c: char) -> KeyPress {
        KeyPress {
            ctrl: true,
            ..key(c)
        }
    }

    fn alt(c: char) -> KeyPress {
        KeyPress { alt: true, ..key(c) }
    }

    /// Feed the shell's events to the panel until its screen shows `needle`.
    fn wait_for(panel: &mut Panel, events: &mut Events, generation: u64, needle: &str) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            match events.try_recv() {
                Ok(event) => panel.handle(generation, event),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
            let shown = panel.terminal.as_ref().unwrap().screen().contents();
            if shown.matches(needle).count() >= 2 {
                return true;
            }
        }
        false
    }

    #[test]
    fn the_panel_keeps_its_shell_while_hidden_and_owns_two_chords() {
        let (shell, args) = if cfg!(windows) {
            ("cmd.exe", vec!["/Q"])
        } else {
            ("/bin/sh", vec![])
        };
        let mut panel = Panel::new(TerminalConfig {
            shell: Some(shell.into()),
            args: args.into_iter().map(String::from).collect(),
            ..TerminalConfig::default()
        });
        let (generation, mut events) = panel.focus().expect("a shell starts");
        assert!(panel.focused());

        // Typed keys reach the shell; the command and its output both show.
        for c in "echo arrow-panel".chars() {
            panel.press(key(c));
        }
        panel.press(KeyPress::plain(EditorKey::Enter));
        assert!(wait_for(&mut panel, &mut events, generation, "arrow-panel"));

        // Ctrl-t hands the keys back; the panel stays up.
        assert!(panel.press(ctrl('t')).is_none());
        assert!(!panel.focused() && panel.shown);

        // Hiding and showing again keeps the same shell and its output.
        assert!(panel.toggle().is_none());
        assert!(!panel.shown);
        assert!(panel.toggle().is_none(), "no new shell is started");
        assert!(panel.focused());
        let shown = panel.terminal.as_ref().unwrap().screen().contents();
        assert!(shown.contains("arrow-panel"), "{shown}");

        // Alt-t hides it from inside.
        panel.press(alt('t'));
        assert!(!panel.shown && !panel.focused());

        // Events from a shell that has been replaced are ignored.
        panel.handle(generation + 1, Event::Exited);
        assert!(!panel.terminal.as_ref().unwrap().exited());
    }
}
