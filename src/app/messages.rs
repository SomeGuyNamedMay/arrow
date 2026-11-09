use crate::components::lsp::{CompletionCandidate, LspClient};
use crate::components::text_editor::Message;
use iced::keyboard;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(super) enum AppMessage {
    Editor(Message),
    ScrollFrame,
    ViewportHeight(f32),
    Key {
        key: keyboard::Key,
        text: Option<String>,
    },
    LspServerStarted {
        generation: u64,
        result: Result<Arc<LspClient>, String>,
    },
    CompletionReady {
        generation: u64,
        version: i32,
        result: Result<Vec<CompletionCandidate>, String>,
    },
}
