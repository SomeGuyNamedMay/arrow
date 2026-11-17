use super::state::Arrow;
use crate::components::python::{PythonAction, PythonRuntime};
use crate::components::text_editor::{Mode, TextEditor};
use crate::components::vertico::VerticoItem;

pub(super) fn apply_python_actions(app: &mut Arrow) {
    for action in PythonRuntime::take_actions() {
        match action {
            PythonAction::OpenBuffer { contents, path } => {
                app.editor = TextEditor::default().with_text(contents);
                configure_language(app, std::path::Path::new(&path));
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
                            configure_language(app, &path);
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
                    if action == "lsp-completion" {
                        if let [start, end, text] = arguments.as_slice() {
                            match (start.parse::<usize>(), end.parse::<usize>()) {
                                (Ok(start), Ok(end)) => {
                                    app.editor.apply_completion(start, end, text)
                                }
                                (Err(error), _) | (_, Err(error)) => {
                                    eprintln!("Invalid LSP completion range: {error}");
                                }
                            }
                        } else {
                            eprintln!("LSP completion item has an invalid argument list");
                        }
                    } else if let Some(python) = &app.python {
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
            PythonAction::SelectFunction => {
                if let Some(config) = &app.syntax_config {
                    app.editor
                        .select_text_object_at_focused_dots(&config.text_object_kinds);
                }
            }
        }
    }
}

fn configure_language(app: &mut Arrow, path: &std::path::Path) {
    app.lsp_generation = app.lsp_generation.wrapping_add(1);
    app.lsp = None;
    app.editor.clear_highlights();
    app.lsp_version = 0;
    app.lsp_text.clear();
    app.syntax_text.clear();
    app.completion_pending = false;
    app.lsp_start_pending = false;
    app.lsp_path = Some(path.to_path_buf());
    app.syntax_config = None;
    app.lsp_config = None;
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
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    app.syntax_config = languages.into_iter().find(|language| {
        language.extensions.iter().any(|candidate| {
            candidate
                .trim_start_matches('.')
                .eq_ignore_ascii_case(&extension)
        })
    });
    app.lsp_config = app
        .syntax_config
        .as_ref()
        .filter(|language| language.language_server.is_some())
        .cloned();
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
