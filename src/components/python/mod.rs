mod api;
mod runtime;

pub use runtime::PythonRuntime;

use crate::components::text_editor::Dot;
use std::cell::RefCell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonAction {
    OpenBuffer {
        contents: String,
        path: String,
    },
    EnterInsertMode,
    LeaveInsertMode,
    ReplaceRanges(Vec<(usize, usize, String)>),
    ToggleFileExplorer,
    ShowVertico {
        title: String,
        prompt: String,
        action: String,
        items: Vec<(String, Vec<String>)>,
    },
    CloseFileExplorer,
    ExplorerSelectionUp,
    ExplorerSelectionDown,
    ExplorerParentDirectory,
    ExplorerOpenSelected,
    CloseVertico,
    VerticoSelectionUp,
    VerticoSelectionDown,
    VerticoBackspace,
    ExplorerActivateSelected,
    ExecuteVertico,
    ExecuteCommand(String),
    ReplaceDots(Vec<Dot>),
    SelectFunction,
}

thread_local! {
    pub(super) static PYTHON_ACTIONS: RefCell<Vec<PythonAction>> =
        const { RefCell::new(Vec::new()) };
    pub(super) static EDITOR_TEXT: RefCell<String> =
        const { RefCell::new(String::new()) };
}

pub fn config_directory() -> std::path::PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return std::path::PathBuf::from(config_home).join("arrow");
    }

    if let Some(app_data) = std::env::var_os("APPDATA") {
        return std::path::PathBuf::from(app_data).join("Arrow");
    }

    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".config")
        .join("arrow")
}
