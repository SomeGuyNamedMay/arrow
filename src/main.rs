mod components;

use components::vertico::Vertico;
use components::text_editor::{Mode, Message, TextEditor};
use iced::widget::{column, container, text};
use iced::{Element, Subscription, keyboard, Task};

struct Arrow {
    editor: TextEditor,
    menu: Vertico,
}

impl Default for Arrow {
    fn default() -> Self {
        Self {
            editor: TextEditor::default().with_text(
                "Welcome to Arrow.\n\nPress i to insert and Escape to return to normal mode.",
            ),
            menu: Vertico::default(),
        }
    }
}

#[derive(Debug, Clone)]
enum AppMessage {
    Editor(Message),
    Key { key: keyboard::Key, text: Option<String> },
}

fn update(app: &mut Arrow, message: AppMessage) -> Task<AppMessage> {
    match message {
        AppMessage::Editor(message) => app.editor.update(message),
        AppMessage::Key { key, text } => {
            if matches!(key.as_ref(), keyboard::Key::Named(keyboard::key::Named::Tab)) {
                app.menu.toggle();
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
    let mode = match app.editor.mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
    };

    column![
        container(app.editor.view().map(AppMessage::Editor)).height(iced::Length::Fill),
        app.menu.view().map(|_| unreachable!()),
        text(format!("{mode} | h j k l move | x select line | i insert | c change | d delete"))
            .font(iced::Font::MONOSPACE),
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