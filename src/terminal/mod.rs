//! A scratchpad terminal: a real shell in a panel under the status bar.
//!
//! The shell runs in a pseudo-terminal (`portable-pty`: ConPTY on Windows, a
//! pty elsewhere), so prompts, colours, line editing and full-screen programs
//! work. Its output is turned into a grid of styled cells by `vt100`, and
//! drawn by [`view`].
//!
//! The terminal belongs to the window ([`crate::app`]), not the session, so it
//! keeps running, and stays responsive, while plugins work. Hiding the panel
//! leaves the shell running with its scrollback, directory and history. Its
//! output is also kept in a transcript file (a capped tail of the raw bytes),
//! replayed when the terminal next starts, so the history survives a restart
//! too, although the shell itself is a new one.
//!
//! - [`config`]: `terminal.toml`.
//! - [`keys`]: key presses as terminal bytes.
//! - [`panel`]: showing, hiding and focusing it, for the window.
//! - [`view`]: drawing the screen.

pub mod config;
mod keys;
mod panel;
mod view;

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::Path;

use iced::futures::channel::mpsc;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::components::keys::{EditorKey, KeyPress};
pub use config::TerminalConfig;
pub use panel::{Panel, Started};
pub use view::view;

/// Lines of scrollback kept in memory.
const SCROLLBACK: usize = 5000;
/// Most of the transcript kept between runs.
const TRANSCRIPT_LIMIT: usize = 256 * 1024;
/// Shown after a replayed transcript. It first leaves the alternate screen and
/// resets colours, in case the transcript ended inside a full-screen program.
const RESTORED: &[u8] = b"\x1b[?1049l\x1b[0m\r\n\x1b[2m--- restored from the last session ---\x1b[0m\r\n";

/// What the shell's threads report.
#[derive(Debug, Clone)]
pub enum Event {
    Output(Vec<u8>),
    Exited,
}

/// Everything the shell reports, in order. The window streams it into
/// messages.
pub type Events = mpsc::UnboundedReceiver<Event>;

/// A running (or finished) shell and its screen.
pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    parser: vt100::Parser<Replies>,
    transcript: Option<File>,
    exited: bool,
}

impl Terminal {
    /// Start the configured shell in the working directory, on a screen of
    /// `rows` × `cols`, after replaying the transcript of earlier runs.
    pub fn spawn(config: &TerminalConfig, rows: u16, cols: u16) -> Result<(Self, Events), String> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        let pty = native_pty_system()
            .openpty(size(rows, cols))
            .map_err(|e| format!("can't open a terminal: {e}"))?;
        let (program, args) = config.command();
        let mut command = CommandBuilder::new(&program);
        command.args(&args);
        if let Ok(dir) = std::env::current_dir() {
            command.cwd(dir);
        }
        command.env("TERM", "xterm-256color");
        let mut child = pty
            .slave
            .spawn_command(command)
            .map_err(|e| format!("can't start {program}: {e}"))?;
        // The shell holds its own end now. Keeping ours open would stop the
        // reader from ever seeing the end of the output.
        drop(pty.slave);
        let master = pty.master;
        let reader = master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = master.take_writer().map_err(|e| e.to_string())?;
        let killer = child.clone_killer();

        let (sender, events) = mpsc::unbounded();
        let output = sender.clone();
        spawn_thread("terminal-output", move || read_output(reader, output));
        // Waiting separately catches the exit even where the reader blocks
        // until the pty is closed (ConPTY does).
        spawn_thread("terminal-wait", move || {
            let _ = child.wait();
            let _ = sender.unbounded_send(Event::Exited);
        });

        let mut terminal = Self {
            master,
            writer,
            killer,
            parser: vt100::Parser::new_with_callbacks(nonzero(rows), nonzero(cols), SCROLLBACK, Replies::default()),
            transcript: None,
            exited: false,
        };
        if let Some(path) = &config.transcript {
            terminal.transcript = terminal.restore(path);
        }
        Ok((terminal, events))
    }

    /// Replay the saved transcript onto the screen, trim the file to its tail,
    /// and keep it open to add to.
    fn restore(&mut self, path: &Path) -> Option<File> {
        let saved = fs::read(path).unwrap_or_default();
        let tail = transcript_tail(&saved, TRANSCRIPT_LIMIT);
        if !tail.is_empty() {
            self.parser.process(tail);
            self.parser.process(RESTORED);
            // The old shell's questions were answered long ago.
            self.parser.callbacks_mut().replies.clear();
        }
        fs::create_dir_all(path.parent()?).ok()?;
        fs::write(path, tail).ok()?;
        OpenOptions::new().append(true).open(path).ok()
    }

    /// Take in the shell's output: draw it, keep it in the transcript, and
    /// answer what it asked (like where the cursor is).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.parser.screen_mut().set_scrollback(0);
        if let Some(file) = &mut self.transcript
            && file.write_all(bytes).is_err()
        {
            self.transcript = None;
        }
        let replies = std::mem::take(&mut self.parser.callbacks_mut().replies);
        if !replies.is_empty() {
            self.send(&replies);
        }
    }

    /// Handle a key press while the terminal has focus: Shift+PageUp and
    /// Shift+PageDown scroll back through the history; everything else goes
    /// to the shell.
    pub fn press(&mut self, press: KeyPress) {
        let page = usize::from(self.size().0);
        let screen = self.parser.screen_mut();
        match press.key {
            EditorKey::PageUp if press.shift => {
                screen.set_scrollback(screen.scrollback() + page);
                return;
            }
            EditorKey::PageDown if press.shift => {
                screen.set_scrollback(screen.scrollback().saturating_sub(page));
                return;
            }
            _ => {}
        }
        screen.set_scrollback(0);
        let application = screen.application_cursor();
        if let Some(bytes) = keys::encode(press, application) {
            self.send(&bytes);
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        // A shell that has gone away can't be written to; it's reported as
        // exited separately.
        let _ = self.writer.write_all(bytes).and_then(|()| self.writer.flush());
    }

    /// (rows, cols).
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (rows.get(), cols.get())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if (rows, cols) != self.size() {
            self.parser.set_size(nonzero(rows), nonzero(cols));
            let _ = self.master.resize(size(rows, cols));
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn mark_exited(&mut self) {
        if !self.exited {
            self.exited = true;
            self.parser.process(b"\r\n\x1b[2m[process exited; press a key to start a new one]\x1b[0m");
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

/// Answers to the queries programs send the terminal. ConPTY asks where the
/// cursor is while a shell starts, and waits for the answer.
#[derive(Default)]
struct Replies {
    replies: Vec<u8>,
}

impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        match (c, i1, params) {
            // Device status report: "are you there?"
            ('n', None, [[5]]) => self.replies.extend_from_slice(b"\x1b[0n"),
            // Cursor position report, 1-based.
            ('n', None, [[6]]) => {
                let (row, col) = screen.cursor_position();
                let reply = format!("\x1b[{};{}R", row + 1, col + 1);
                self.replies.extend_from_slice(reply.as_bytes());
            }
            // Primary device attributes: a VT102-compatible terminal.
            ('c', None, [] | [[0]]) => self.replies.extend_from_slice(b"\x1b[?6c"),
            _ => {}
        }
    }
}

fn read_output(mut reader: Box<dyn Read + Send>, output: mpsc::UnboundedSender<Event>) {
    let mut buffer = [0u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if output.unbounded_send(Event::Output(buffer[..n].to_vec())).is_err() {
                    return;
                }
            }
        }
    }
}

fn spawn_thread(name: &str, work: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(work);
}

/// The last `limit` bytes of a transcript, starting at a line so it doesn't
/// begin halfway through an escape sequence.
fn transcript_tail(bytes: &[u8], limit: usize) -> &[u8] {
    if bytes.len() <= limit {
        return bytes;
    }
    let start = bytes.len() - limit;
    match bytes[start..].iter().position(|&b| b == b'\n') {
        Some(newline) => &bytes[start + newline + 1..],
        None => &[],
    }
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn nonzero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_are_cut_at_a_line() {
        assert_eq!(transcript_tail(b"short", 10), b"short");
        assert_eq!(transcript_tail(b"one\ntwo\nthree", 8), b"three");
        assert_eq!(transcript_tail(b"no newline at all", 4), b"");
    }

    #[test]
    fn cursor_queries_are_answered() {
        let mut parser = vt100::Parser::new_with_callbacks(nonzero(4), nonzero(20), 0, Replies::default());
        parser.process(b"abc\x1b[6n\x1b[5n");
        assert_eq!(parser.callbacks().replies, b"\x1b[1;4R\x1b[0n");
    }

    /// Start a real shell, run one command and read its output.
    #[test]
    fn a_shell_runs_and_its_output_reaches_the_screen() {
        let dir = Path::new("target/arrow-test-writes/terminal");
        let _ = fs::remove_dir_all(dir);
        let transcript = dir.join("terminal.log");
        let (shell, args) = if cfg!(windows) {
            ("cmd.exe", vec!["/C", "echo arrow-terminal-test"])
        } else {
            ("/bin/sh", vec!["-c", "echo arrow-terminal-test"])
        };
        let config = TerminalConfig {
            shell: Some(shell.into()),
            args: args.into_iter().map(String::from).collect(),
            transcript: Some(transcript.clone()),
            ..TerminalConfig::default()
        };
        let (mut terminal, mut events) = Terminal::spawn(&config, 10, 60).unwrap();
        // The exit can be reported before the last output arrives, so keep
        // reading until things have been quiet for a moment.
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut last_event = start;
        while start.elapsed() < Duration::from_secs(20) {
            match events.try_recv() {
                Ok(Event::Output(bytes)) => {
                    terminal.feed(&bytes);
                    last_event = Instant::now();
                }
                Ok(Event::Exited) => {
                    terminal.mark_exited();
                    last_event = Instant::now();
                }
                Err(_) if terminal.exited() && last_event.elapsed() > Duration::from_millis(500) => {
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert!(terminal.exited(), "the shell should finish");
        let shown = terminal.screen().contents();
        assert!(shown.contains("arrow-terminal-test"), "{shown}");
        let saved = fs::read(&transcript).unwrap();
        assert!(String::from_utf8_lossy(&saved).contains("arrow-terminal-test"));

        // The next terminal starts with the old one's output on screen.
        drop(terminal);
        let (terminal, _events) = Terminal::spawn(&config, 10, 60).unwrap();
        let shown = terminal.screen().contents();
        assert!(shown.contains("arrow-terminal-test"), "{shown}");
        assert!(shown.contains("restored from the last session"), "{shown}");
    }
}
