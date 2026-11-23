//! `terminal.toml` in the user config directory: which shell to run, and how
//! tall the panel is.
//!
//! ```toml
//! shell = "pwsh.exe"     # default: powershell.exe on Windows, $SHELL elsewhere
//! args = ["-NoLogo"]
//! rows = 12
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalConfig {
    /// The program to run. `None` picks the platform's usual shell.
    pub shell: Option<String>,
    pub args: Vec<String>,
    /// Height of the panel, in lines.
    pub rows: u16,
    /// Where the transcript is kept between runs, so the terminal's history
    /// survives a restart. `None` keeps nothing.
    #[serde(skip)]
    pub transcript: Option<PathBuf>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: None,
            args: Vec::new(),
            rows: 12,
            transcript: None,
        }
    }
}

impl TerminalConfig {
    /// Read `terminal.toml` from `dir` (a missing file means the defaults), and
    /// keep the transcript in `dir/state/terminal.log`.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join("terminal.toml");
        let mut config: Self = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        config.rows = config.rows.clamp(3, 200);
        config.transcript = Some(dir.join("state").join("terminal.log"));
        Ok(config)
    }

    /// The program and arguments to start.
    pub fn command(&self) -> (String, Vec<String>) {
        match &self.shell {
            Some(shell) => (shell.clone(), self.args.clone()),
            None if cfg!(windows) => ("powershell.exe".to_string(), vec!["-NoLogo".to_string()]),
            None => (
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
                self.args.clone(),
            ),
        }
    }
}
