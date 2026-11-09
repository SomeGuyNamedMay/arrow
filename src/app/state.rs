use crate::components::file_explorer::FileExplorer;
use crate::components::python::PythonRuntime;
use crate::components::text_editor::TextEditor;
use crate::components::vertico::Vertico;

pub(super) struct Arrow {
    pub(super) editor: TextEditor,
    pub(super) vertico: Vertico,
    pub(super) explorer: FileExplorer,
    pub(super) python: Option<PythonRuntime>,
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
        }
    }
}
