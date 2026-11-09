use crate::components::text_editor::Message;
use iced::keyboard;

#[derive(Debug, Clone)]
pub(super) enum AppMessage {
    Editor(Message),
    ScrollFrame,
    ViewportHeight(f32),
    Key {
        key: keyboard::Key,
        text: Option<String>,
    },
}
