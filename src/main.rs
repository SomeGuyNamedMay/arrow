//! Arrow, a Kakoune-style modal editor whose behaviour comes from plugins.
//!
//! - [`app`]: the iced window. It hands key presses to the session in async
//!   tasks and draws a snapshot while one runs.
//! - [`session`]: the editor and its plugins. Turns keys into commands and
//!   delivers the events plugins subscribed to.
//! - [`plugins`]: loads and calls WebAssembly plugins, and the API they use.
//! - [`syntax`]: tree-sitter highlighting with grammars the user adds.
//! - [`components`]: the widgets: text editor, menu and tree menu.
//! - [`config`]: where the user's plugins, grammars and settings live.

mod app;
mod components;
mod config;
mod plugins;
mod session;
mod syntax;

use app::App;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Arrow")
        .theme(iced::Theme::Dark)
        .subscription(App::subscription)
        .window_size((900.0, 600.0))
        .centered()
        .run()
}
