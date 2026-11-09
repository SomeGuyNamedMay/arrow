use super::state::Arrow;
use crate::components::python::{PythonAction, PythonRuntime};
use crate::components::text_editor::{Mode, TextEditor};
use crate::components::vertico::VerticoItem;

pub(super) fn apply_python_actions(app: &mut Arrow) {
    for action in PythonRuntime::take_actions() {
        match action {
            PythonAction::OpenBuffer { contents, path } => {
                app.editor = TextEditor::default().with_text(contents);
                configure_syntax(app, std::path::Path::new(&path));
            }
            PythonAction::EnterInsertMode => {
                app.editor.mode = Mode::Insert;
            }
            PythonAction::LeaveInsertMode => {
                app.editor.normal_mode();
            }
            PythonAction::ReplaceRanges(ranges) => app.editor.replace_ranges(&ranges),
            PythonAction::ToggleFileExplorer => app.explorer.toggle(),
            PythonAction::ShowVertico {
                title,
                prompt,
                action,
                items,
            } => {
                app.vertico.title = title;
                app.vertico.prompt = prompt;
                app.vertico.replace_items(
                    items
                        .into_iter()
                        .map(|(label, arguments)| VerticoItem { label, arguments })
                        .collect(),
                );
                app.vertico.action = Some(action);
                app.vertico.visible = true;
            }
            PythonAction::CloseFileExplorer => app.explorer.visible = false,
            PythonAction::ExplorerSelectionUp => app.explorer.select_previous(),
            PythonAction::ExplorerSelectionDown => app.explorer.select_next(),
            PythonAction::ExplorerParentDirectory => app.explorer.parent_directory(),
            PythonAction::ExplorerOpenSelected => app.explorer.open_selected(),
            PythonAction::CloseVertico => app.vertico.visible = false,
            PythonAction::VerticoSelectionUp => app.vertico.move_selection(-1),
            PythonAction::VerticoSelectionDown => app.vertico.move_selection(1),
            PythonAction::VerticoBackspace => {
                app.vertico.query_mut().pop();
                app.vertico.reset_selection();
            }
            PythonAction::ExplorerActivateSelected => {
                if let Some(path) = app.explorer.selected_file() {
                    match std::fs::read_to_string(&path) {
                        Ok(contents) => {
                            app.editor = TextEditor::default().with_text(contents);
                            configure_syntax(app, &path);
                            app.explorer.visible = false;
                        }
                        Err(error) => eprintln!("Failed to open {}: {error}", path.display()),
                    }
                } else {
                    app.explorer.open_selected();
                }
            }
            PythonAction::ExecuteVertico => {
                if let Some(action) = app.vertico.action.clone() {
                    let arguments = app
                        .vertico
                        .selected_arguments()
                        .unwrap_or_default()
                        .to_vec();
                    app.vertico.visible = false;
                    if let Some(python) = &app.python {
                        if let Err(error) = python.execute_with_args(&action, &arguments) {
                            eprintln!("{error}");
                        }
                    }
                }
            }
            PythonAction::ExecuteCommand(command) => {
                execute_python(app, &command);
            }
            PythonAction::ReplaceDots(dots) => app.editor.set_dots(&dots),
            PythonAction::SelectFunction => app.editor.select_function_at_focused_dots(),
        }
    }
}

fn configure_syntax(app: &mut Arrow, path: &std::path::Path) {
    let languages = match &app.python {
        Some(python) => match python.syntax_languages() {
            Ok(languages) => languages,
            Err(error) => {
                eprintln!("Failed to load Python syntax configuration: {error}");
                return;
            }
        },
        None => Vec::new(),
    };
    if let Err(error) = app.editor.set_syntax_languages(languages) {
        eprintln!("Invalid syntax configuration: {error}");
        return;
    }
    if let Err(error) = app.editor.set_file_path(path) {
        eprintln!("{error}");
    }
}

pub(super) fn execute_python(app: &mut Arrow, command: &str) {
    let Some(python) = &app.python else {
        return;
    };
    if let Err(error) = python.set_editor_dots(&app.editor.dots()) {
        eprintln!("{error}");
        return;
    }
    if let Err(error) = python.set_editor_text(&app.editor.buffer.to_string()) {
        eprintln!("{error}");
        return;
    }
    if let Err(error) = python.execute(command) {
        eprintln!("{error}");
    }
    apply_python_actions(app);
}
