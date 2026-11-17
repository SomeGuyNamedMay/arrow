use super::messages::AppMessage;
use super::python_actions::execute_python;
use super::{keymap, state::Arrow};
use crate::components::lsp::{CompletionCandidate, LspClient};
use crate::components::text_editor::{Message, Mode};
use crate::components::vertico::VerticoItem;
use iced::Task;
use std::sync::Arc;
use std::time::Duration;

pub(super) fn update(app: &mut Arrow, message: AppMessage) -> Task<AppMessage> {
    match message {
        AppMessage::Editor(Message::ScrollBy {
            delta,
            viewport_height,
        }) => app.editor.update(Message::ScrollBy {
            delta,
            viewport_height,
        }),
        AppMessage::Editor(Message::ScrollFrame) | AppMessage::ScrollFrame => {
            app.editor.update(Message::ScrollFrame);
        }
        AppMessage::ViewportHeight(height) => app.editor.set_viewport_height(height),
        AppMessage::Editor(message) => app.editor.update(message),
        AppMessage::Key { key, text } => {
            if !app.vertico.visible
                && !app.explorer.visible
                && keymap::key_name(&key).as_deref() == Some("tab")
                && app.lsp_config.is_some()
            {
                return request_completion(app);
            }
            let python_mode = if app.vertico.visible {
                "vertico"
            } else if app.explorer.visible {
                "explorer"
            } else if app.editor.mode == Mode::Insert {
                "insert"
            } else {
                "normal"
            };
            if let Some(command) = keymap::key_name(&key)
                .and_then(|key| app.python.as_ref()?.command_for_key(python_mode, &key))
            {
                execute_python(app, &command);
            } else if app.vertico.visible {
                if let Some(text) = text
                    && !text.chars().all(char::is_control)
                {
                    app.vertico.insert_text(&text);
                }
            } else if !app.explorer.visible {
                app.editor.update(Message::Key { key, text });
            }
        }
        AppMessage::LspServerStarted { generation, result } => {
            if generation != app.lsp_generation {
                return Task::none();
            }
            app.lsp_start_pending = false;
            match result {
                Ok(client) => {
                    let text = app.editor.buffer.to_string();
                    if let Err(error) = client.did_open(&text) {
                        eprintln!("{error}");
                        app.completion_pending = false;
                        return Task::none();
                    }
                    app.lsp_version = 1;
                    app.lsp_text = text;
                    app.lsp = Some(client);
                    if app.completion_pending {
                        app.completion_pending = false;
                        return request_completion(app);
                    }
                }
                Err(error) => {
                    app.completion_pending = false;
                    eprintln!("{error}");
                }
            }
        }
        AppMessage::CompletionReady {
            generation,
            version,
            result,
        } => {
            if generation != app.lsp_generation || version != app.lsp_version {
                return Task::none();
            }
            match result {
                Ok(completions) => show_completions(app, completions),
                Err(error) => eprintln!("{error}"),
            }
        }
    }

    let syntax_task = refresh_syntax(app);
    let lsp_sync_task = synchronize_lsp_document(app);
    let lsp_start_task = ensure_lsp_started(app);
    Task::batch([syntax_task, lsp_sync_task, lsp_start_task])
}

fn refresh_syntax(app: &mut Arrow) -> Task<AppMessage> {
    let source = app.editor.buffer.to_string();
    if source == app.syntax_text {
        return Task::none();
    }
    app.syntax_text.clone_from(&source);
    let Some(config) = app.syntax_config.as_ref() else {
        app.editor.clear_highlights();
        return Task::none();
    };
    let Some(python) = app.python.as_ref() else {
        app.editor.clear_highlights();
        return Task::none();
    };
    match python.parse_syntax(&config.name, &source) {
        Ok(tree) => app.editor.set_syntax_tree(tree, &config.highlight_colors),
        Err(error) => {
            app.editor.clear_highlights();
            eprintln!("{error}");
        }
    }
    Task::none()
}

fn synchronize_lsp_document(app: &mut Arrow) -> Task<AppMessage> {
    let text = app.editor.buffer.to_string();
    if text != app.lsp_text
        && let Some(lsp) = &app.lsp
    {
        app.lsp_version = app.lsp_version.saturating_add(1);
        if let Err(error) = lsp.did_change(app.lsp_version, &text) {
            eprintln!("{error}");
        } else {
            app.lsp_text = text;
        }
    }
    Task::none()
}

fn ensure_lsp_started(app: &mut Arrow) -> Task<AppMessage> {
    if app.lsp.is_some() || app.lsp_start_pending {
        return Task::none();
    }
    let (Some(language), Some(path)) = (app.lsp_config.clone(), app.lsp_path.clone()) else {
        return Task::none();
    };
    let Some(config) = language.language_server else {
        return Task::none();
    };
    app.lsp_start_pending = true;
    let generation = app.lsp_generation;
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                LspClient::start(config, &path, language.name).map(Arc::new)
            })
            .await
            .unwrap_or_else(|error| Err(format!("Language server startup failed: {error}")))
        },
        move |result| AppMessage::LspServerStarted { generation, result },
    )
}

fn request_completion(app: &mut Arrow) -> Task<AppMessage> {
    let Some(dot) = app.editor.dots().into_iter().find(|dot| dot.is_focused) else {
        return Task::none();
    };
    let source = app.editor.buffer.to_string();
    let generation = app.lsp_generation;

    if let Some(client) = app.lsp.clone() {
        let version = app.lsp_version;
        return match client.completion(source, dot.cursor) {
            Ok(receiver) => Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        receiver
                            .recv_timeout(Duration::from_secs(12))
                            .map_err(|error| format!("Completion response failed: {error}"))?
                    })
                    .await
                    .unwrap_or_else(|error| Err(format!("Completion task failed: {error}")))
                },
                move |result| AppMessage::CompletionReady {
                    generation,
                    version,
                    result,
                },
            ),
            Err(error) => {
                eprintln!("{error}");
                Task::none()
            }
        };
    }

    if app.completion_pending {
        return Task::none();
    }
    if app.lsp_config.is_none() {
        return Task::none();
    }
    app.completion_pending = true;
    ensure_lsp_started(app)
}

fn show_completions(app: &mut Arrow, completions: Vec<CompletionCandidate>) {
    if completions.is_empty() {
        app.vertico.visible = false;
        return;
    }
    app.vertico.replace_items_with_title(
        "Completion",
        completions
            .into_iter()
            .map(|completion| VerticoItem {
                label: completion.label,
                arguments: vec![
                    completion.start.to_string(),
                    completion.end.to_string(),
                    completion.insert_text,
                ],
            })
            .collect(),
    );
    app.vertico.action = Some("lsp-completion".to_string());
}
