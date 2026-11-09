use iced::keyboard;

pub(super) fn key_name(key: &keyboard::Key) -> Option<String> {
    match key.as_ref() {
        keyboard::Key::Named(keyboard::key::Named::Escape) => Some("escape".into()),
        keyboard::Key::Named(keyboard::key::Named::Space) => Some("space".into()),
        keyboard::Key::Named(keyboard::key::Named::Tab) => Some("tab".into()),
        keyboard::Key::Named(keyboard::key::Named::Enter) => Some("enter".into()),
        keyboard::Key::Named(keyboard::key::Named::Backspace) => Some("backspace".into()),
        keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => Some("left".into()),
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some("down".into()),
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some("up".into()),
        keyboard::Key::Named(keyboard::key::Named::ArrowRight) => Some("right".into()),
        keyboard::Key::Character(character) => {
            if character.chars().all(char::is_whitespace) {
                Some("space".into())
            } else {
                Some(character.to_lowercase())
            }
        }
        _ => None,
    }
}
