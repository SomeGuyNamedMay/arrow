mod components;

use components::file_explorer::FileExplorer;
use components::python::PythonRuntime;
use components::text_editor::{Message, Mode, TextEditor};
use components::vertico::Vertico;
use iced::widget::{column, container};
use iced::{Element, Subscription, Task, keyboard};

struct Arrow {
    editor: TextEditor,
    menu: Vertico,
    explorer: FileExplorer,
    python: Option<PythonRuntime>,
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
        let mut menu = Vertico::default();
        if let Some(python) = &python {
            let mut commands = python.command_names();
            commands.sort();
            commands.dedup();
            menu.replace_candidates(commands);
        }
        Self {
            editor: TextEditor::default().with_text(
                "Welcome to Arrow.\n\nPress i to insert and Escape to return to normal mode.",
            ),
            menu,
            explorer: FileExplorer::default(),
            python,
        }
    }
}

#[derive(Debug, Clone)]
enum AppMessage {
    Editor(Message),
    Key {
        key: keyboard::Key,
        text: Option<String>,
    },
}

fn is_enter(key: &keyboard::Key) -> bool {
    matches!(
        key.as_ref(),
        keyboard::Key::Named(keyboard::key::Named::Enter)
    )
}

fn update(app: &mut Arrow, message: AppMessage) -> Task<AppMessage> {
    match message {
        AppMessage::Editor(message) => app.editor.update(message),
        AppMessage::Key { key, text } => {
            if matches!(
                key.as_ref(),
                keyboard::Key::Named(keyboard::key::Named::Tab)
            ) {
                app.menu.toggle();
            } else if app.explorer.visible && app.explorer.update_key(&key) {
                // The file explorer owns navigation while open.
                if is_enter(&key)
                    && let Some(path) = app.explorer.selected_file()
                {
                    match std::fs::read_to_string(&path) {
                        Ok(contents) => {
                            app.editor = TextEditor::default().with_text(contents);
                            app.explorer.visible = false;
                        }
                        Err(error) => {
                            eprintln!("Failed to open {}: {error}", path.display());
                        }
                    }
                }
            } else if app.editor.mode == Mode::Normal
                && matches!(key.as_ref(), keyboard::Key::Character(character) if character == "e")
            {
                app.explorer.toggle();
            } else if app.editor.mode == Mode::Normal
                && matches!(
                    key.as_ref(),
                    keyboard::Key::Named(keyboard::key::Named::Space)
                )
                || app.editor.mode == Mode::Normal
                    && matches!(key.as_ref(), keyboard::Key::Character(character) if character == " ")
            {
                if !app.menu.visible {
                    app.menu.toggle();
                }
            } else if app.menu.visible && is_enter(&key) {
                if let Some(command) = app.menu.selected_candidate().map(str::to_string) {
                    app.menu.visible = false;
                    if let Some(python) = &app.python
                        && python.has_command(&command)
                    {
                        if let Err(error) = python.execute(&command) {
                            eprintln!("{error}");
                        }
                    }
                }
            } else if app.menu.visible && app.menu.update_key(&key, text.as_deref()) {
                // Vertico owns navigation and minibuffer input.
            } else {
                app.editor.update(Message::Key { key, text });
            }
        }
    }
    Task::none()
}

fn subscription(_app: &Arrow) -> Subscription<AppMessage> {
    keyboard::listen().map(|event| {
        let keyboard::Event::KeyPressed { key, text, .. } = event else {
            return AppMessage::Key {
                key: keyboard::Key::Unidentified,
                text: None,
            };
        };

        AppMessage::Key {
            key,
            text: text.map(|text| text.to_string()),
        }
    })
}

fn view(app: &Arrow) -> Element<'_, AppMessage> {
    column![
        app.explorer.view().map(|_| unreachable!()),
        container(app.editor.view().map(AppMessage::Editor)).height(iced::Length::Fill),
        app.menu.view().map(|_| unreachable!()),
    ]
    .height(iced::Length::Fill)
    .into()
}

fn main() -> iced::Result {
    iced::application(Arrow::default, update, view)
        .subscription(subscription)
        .title("Arrow - Kakoune Text Editor")
        .run()
}
