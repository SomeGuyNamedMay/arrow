//! Tree-sitter syntax highlighting, with grammars the user adds.
//!
//! Grammars are WebAssembly modules built with `tree-sitter build --wasm`, loaded
//! at runtime into tree-sitter's own sandboxed wasm store, so a grammar can't
//! touch the system and one file works on every platform. Everything lives in
//! the user config directory (next to `plugins/`):
//!
//! ```text
//! arrow/
//!   languages.toml             which grammar to use for which files, and colours
//!   grammars/<grammar>.wasm
//!   queries/<language>/highlights.scm
//! ```
//!
//! ```toml
//! [[language]]
//! name = "rust"
//! extensions = ["rs"]
//! # grammar = "rust"                              (default: the name)
//! # highlights = "queries/rust/highlights.scm"    (default; relative to arrow/)
//!
//! [theme]
//! keyword = "#c678dd"     # capture names; "function.method" falls back to "function"
//! ```
//!
//! The language follows the buffer name's extension. Edits are recorded by the
//! editor as they happen and the tree is reparsed incrementally once per batch of
//! keys, inside the session's task, so parsing stays off the UI thread.
//! Highlighting runs at draw time over the visible lines only.
//!
//! - [`config`]: reading `languages.toml` and the colour theme.
//! - [`highlight`]: colouring the visible text from the tree.
//! - this module: loading grammars, choosing one per buffer, and parsing.

mod config;
mod highlight;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use ropey::Rope;
use tree_sitter::wasmtime::Engine;
use tree_sitter::{InputEdit, Language, Parser, Point, Query, Tree, WasmStore};

use crate::components::text_editor::TextEditor;
use config::LanguageConfig;
pub use config::Languages;
use highlight::HighlightQuery;
pub use highlight::Highlighter;

/// The engine tree-sitter runs grammars on. Separate from the plugin engine:
/// tree-sitter brings its own wasmtime.
fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(Engine::default)
}

/// A grammar that has been loaded, with its highlights query if it has one.
struct Loaded {
    language: Language,
    highlights: Option<Arc<HighlightQuery>>,
}

/// The buffer's language and its latest tree.
struct Current {
    language: String,
    tree: Option<Tree>,
}

/// Parses the buffer with the grammar its name calls for.
pub struct Syntax {
    languages: Languages,
    /// Owns the wasm store grammars are loaded into.
    parser: Parser,
    loaded: HashMap<String, Loaded>,
    current: Option<Current>,
    /// The buffer name the language was chosen for.
    buffer_name: Option<String>,
}

impl Syntax {
    pub fn new(languages: Languages) -> Result<Self, String> {
        let store = WasmStore::new(engine()).map_err(|e| format!("syntax: {}", e.message))?;
        let mut parser = Parser::new();
        parser
            .set_wasm_store(store)
            .map_err(|e| format!("syntax: {e}"))?;
        Ok(Self {
            languages,
            parser,
            loaded: HashMap::new(),
            current: None,
            buffer_name: None,
        })
    }

    /// Bring the tree up to date with the buffer: choose a language if the
    /// buffer was renamed, then reparse what changed and hand the result to the
    /// view. An error (a grammar that won't load, say) is returned once, and the
    /// buffer is left unhighlighted.
    pub fn update(&mut self, editor: &mut TextEditor) -> Result<(), String> {
        let edits = editor.editor_mut().take_edits();
        let name = editor.editor().name();
        if self.buffer_name.as_deref() != Some(name) {
            self.buffer_name = Some(name.to_string());
            let config = self.languages.for_buffer(name).cloned();
            self.current = None;
            editor.set_highlighter(None);
            let Some(config) = config else {
                return Ok(());
            };
            self.load(&config)?;
            self.current = Some(Current {
                language: config.name,
                tree: None,
            });
        } else if edits.is_empty() {
            return Ok(());
        }

        let Some(current) = &mut self.current else {
            return Ok(());
        };
        let loaded = &self.loaded[&current.language];
        if let Some(tree) = &mut current.tree {
            for edit in &edits {
                tree.edit(edit);
            }
        }
        self.parser
            .set_language(&loaded.language)
            .map_err(|e| format!("syntax: {e}"))?;
        let tree = parse(&mut self.parser, editor.editor().rope(), current.tree.as_ref());
        current.tree = tree.clone();
        editor.set_highlighter(match (tree, &loaded.highlights) {
            (Some(tree), Some(query)) => Some(Highlighter {
                tree,
                query: query.clone(),
            }),
            _ => None,
        });
        Ok(())
    }

    /// The latest tree, if the buffer has a language.
    #[cfg(test)]
    pub fn tree(&self) -> Option<&Tree> {
        self.current.as_ref()?.tree.as_ref()
    }

    /// Load a language's grammar and highlights query, once.
    fn load(&mut self, config: &LanguageConfig) -> Result<(), String> {
        if self.loaded.contains_key(&config.name) {
            return Ok(());
        }
        let dir = &self.languages.dir;
        let grammar = config.grammar.as_deref().unwrap_or(&config.name);
        let path = dir.join("grammars").join(format!("{grammar}.wasm"));
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("syntax: can't read {}: {e}", path.display()))?;
        let mut store = self
            .parser
            .take_wasm_store()
            .expect("the parser always has its wasm store");
        let language = store.load_language(grammar, &bytes);
        self.parser
            .set_wasm_store(store)
            .map_err(|e| format!("syntax: {e}"))?;
        let language =
            language.map_err(|e| format!("syntax: grammar {}: {}", path.display(), e.message))?;

        let query_path = match &config.highlights {
            Some(path) => dir.join(path),
            None => dir.join("queries").join(&config.name).join("highlights.scm"),
        };
        let highlights = match std::fs::read_to_string(&query_path) {
            Ok(source) => {
                let query = Query::new(&language, &source).map_err(|e| {
                    format!("syntax: {} line {}: {}", query_path.display(), e.row + 1, e.message)
                })?;
                let colors = query
                    .capture_names()
                    .iter()
                    .map(|name| self.languages.color(name))
                    .collect();
                Some(Arc::new(HighlightQuery { query, colors }))
            }
            // A grammar without a query still parses, for textobjects later.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("syntax: {}: {error}", query_path.display())),
        };
        self.loaded
            .insert(config.name.clone(), Loaded { language, highlights });
        Ok(())
    }
}

/// Parse straight from the rope's chunks, reusing `old` where nothing changed.
fn parse(parser: &mut Parser, rope: &Rope, old: Option<&Tree>) -> Option<Tree> {
    parser.parse_with_options(
        &mut |byte, _| {
            if byte >= rope.len_bytes() {
                return &[][..];
            }
            let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - chunk_start..]
        },
        old,
        None,
    )
}

/// Describe replacing `start..end` (char indices, end exclusive) of `rope`
/// with `text`, for tree-sitter. Call before making the change.
pub fn input_edit(rope: &Rope, start: usize, end: usize, text: &str) -> InputEdit {
    let start_byte = rope.char_to_byte(start);
    let old_end_byte = rope.char_to_byte(end);
    let start_position = point(rope, start_byte);
    // The new end's position: the start, advanced over the inserted text.
    let new_end_position = match text.rfind('\n') {
        Some(newline) => Point {
            row: start_position.row + text.matches('\n').count(),
            column: text.len() - newline - 1,
        },
        None => Point {
            row: start_position.row,
            column: start_position.column + text.len(),
        },
    };
    InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte: start_byte + text.len(),
        start_position,
        old_end_position: point(rope, old_end_byte),
        new_end_position,
    }
}

/// Row and byte column of a byte offset.
fn point(rope: &Rope, byte: usize) -> Point {
    let row = rope.byte_to_line(byte);
    Point {
        row,
        column: byte - rope.line_to_byte(row),
    }
}
