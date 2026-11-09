use super::messages::AppMessage;
use super::state::Arrow;
use iced::widget::{column, container};
use iced::{Element, Subscription, event, keyboard, window};
use std::time::Duration;

pub(super) fn subscription(app: &Arrow) -> Subscription<AppMessage> {
    let keyboard = keyboard::listen().map(|event| {
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
    });
    let scrolling = if app.editor.is_scrolling() {
        iced::time::every(Duration::from_millis(16)).map(|_| AppMessage::ScrollFrame)
    } else {
        Subscription::none()
    };
    let viewport = event::listen_with(viewport_height);

    Subscription::batch([keyboard, scrolling, viewport])
}

fn viewport_height(
    event: event::Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<AppMessage> {
    match event {
        event::Event::Window(window::Event::Opened { size, .. }) => {
            Some(AppMessage::ViewportHeight(size.height))
        }
        event::Event::Window(window::Event::Resized(size)) => {
            Some(AppMessage::ViewportHeight(size.height))
        }
        _ => None,
    }
}

pub(super) fn view(app: &Arrow) -> Element<'_, AppMessage> {
    column![
        app.explorer.view().map(|_| unreachable!()),
        container(app.editor.view().map(AppMessage::Editor)).height(iced::Length::Fill),
        app.vertico.view().map(|_| unreachable!()),
    ]
    .height(iced::Length::Fill)
    .into()
}
