use crate::components::file_explorer::FileExplorer;
use crate::components::lsp::LspClient;
use crate::components::python::PythonRuntime;
use crate::components::syntax::CompletionServerConfig;
use crate::components::text_editor::TextEditor;
use crate::components::vertico::Vertico;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) struct Arrow {
    pub(super) editor: TextEditor,
    pub(super) vertico: Vertico,
    pub(super) explorer: FileExplorer,
    pub(super) python: Option<PythonRuntime>,
    pub(super) lsp_config: Option<(String, CompletionServerConfig)>,
    pub(super) lsp_path: Option<PathBuf>,
    pub(super) lsp: Option<Arc<LspClient>>,
    pub(super) lsp_generation: u64,
    pub(super) lsp_version: i32,
    pub(super) lsp_text: String,
    pub(super) completion_pending: bool,
}

impl Default for Arrow {
    fn default() -> Self {
        let python = match PythonRuntime::new() {
            Ok(python) => Some(python),
            Err(error) => {
                eprintln!("Python scripting disabled: {error}");
                None
            }
        };
        if let Some(python) = &python
            && let Err(error) = python.run_hook("startup")
        {
            eprintln!("{error}");
        }
        let vertico = Vertico::default();
        Self {
            editor: TextEditor::default().with_text(
                "Welcome to Arrow.\n\nPress i to insert and Escape to return to normal mode.",
            ),
            vertico,
            explorer: FileExplorer::default(),
            python,
            lsp_config: None,
            lsp_path: None,
            lsp: None,
            lsp_generation: 0,
            lsp_version: 0,
            lsp_text: String::new(),
            completion_pending: false,
        }
    }
}
