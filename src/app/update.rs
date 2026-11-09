use super::messages::AppMessage;
use super::python_actions::execute_python;
use super::{keymap, state::Arrow};
use crate::components::text_editor::{Message, Mode};
use iced::Task;

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
        AppMessage::Editor(message) => app.editor.update(message),
        AppMessage::Key { key, text } => {
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
                app.editor.update(Message::Key { key });
            }
        }
    }
    Task::none()
}
