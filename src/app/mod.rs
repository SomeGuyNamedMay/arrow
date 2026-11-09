mod keymap;
mod messages;
mod python_actions;
mod state;
mod update;
mod view;

use state::Arrow;

pub fn run() -> iced::Result {
    iced::application(Arrow::default, update::update, view::view)
        .subscription(view::subscription)
        .title("Arrow - Kakoune Text Editor")
        .run()
}
