use iced::Color;
use ropey::Rope;
use std::collections::BTreeMap;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor, Tree};

#[derive(Clone)]
pub(crate) struct SyntaxLanguageConfig {
    pub(crate) name: String,
    pub(crate) grammar: String,
    pub(crate) extensions: Vec<String>,
    pub(crate) highlights_query: String,
    pub(crate) function_nodes: Vec<String>,
    pub(crate) capture_colors: BTreeMap<String, (Color, u8)>,
    pub(crate) completion_server: Option<CompletionServerConfig>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompletionServerConfig {
    pub(crate) command: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) root_markers: Vec<String>,
}

fn grammar_language(grammar: &str) -> Result<Language, String> {
    match grammar {
        "rust" => Ok(tree_sitter_rust::LANGUAGE.into()),
        "python" => Ok(tree_sitter_python::LANGUAGE.into()),
        _ => Err(format!("Unsupported Tree-sitter grammar '{grammar}'")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HighlightSpan {
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) color: Color,
    pub(crate) priority: u8,
}

#[derive(Default)]
pub(crate) struct SyntaxState {
    languages_by_extension: BTreeMap<String, SyntaxLanguageConfig>,
    language: Option<SyntaxLanguageConfig>,
    parser: Option<Parser>,
    query: Option<Query>,
    tree: Option<Tree>,
    highlights_by_line: Vec<Vec<HighlightSpan>>,
}

impl SyntaxState {
    pub(crate) fn set_file_path(
        &mut self,
        path: &std::path::Path,
        source: &str,
    ) -> Result<(), String> {
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            self.clear();
            return Ok(());
        };
        let extension = extension.to_ascii_lowercase();
        let Some(language) = self.languages_by_extension.get(&extension).cloned() else {
            self.clear();
            return Ok(());
        };
        if self
            .language
            .as_ref()
            .is_some_and(|active| active.name == language.name)
        {
            return self.reparse(source);
        }

        let grammar = grammar_language(&language.grammar)?;
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|error| format!("Failed to configure Tree-sitter: {error}"))?;
        let query = Query::new(&grammar, &language.highlights_query)
            .map_err(|error| format!("Failed to load syntax highlighting query: {error}"))?;

        self.language = Some(language);
        self.parser = Some(parser);
        self.query = Some(query);
        self.reparse(source)
    }

    pub(crate) fn reparse(&mut self, source: &str) -> Result<(), String> {
        let (Some(parser), Some(query)) = (&mut self.parser, &self.query) else {
            self.tree = None;
            self.highlights_by_line.clear();
            return Ok(());
        };
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| "Tree-sitter failed to parse the editor buffer".to_string())?;
        let mut query_cursor = QueryCursor::new();
        let mut matches = query_cursor.matches(query, tree.root_node(), source.as_bytes());
        let mut highlights = Vec::new();
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures() {
                let name = query.capture_names()[capture.index as usize];
                if let Some((color, priority)) = self
                    .language
                    .as_ref()
                    .and_then(|language| language.capture_colors.get(name))
                    .copied()
                {
                    let start_byte = capture.node.start_byte();
                    let end_byte = capture.node.end_byte();
                    if start_byte < end_byte {
                        highlights.push(HighlightSpan {
                            start_byte,
                            end_byte,
                            color,
                            priority,
                        });
                    }
                }
            }
        }
        highlights.sort_by_key(|span| (span.start_byte, span.end_byte));
        let mut line_starts = vec![0];
        line_starts.extend(source.match_indices('\n').map(|(index, _)| index + 1));
        let mut highlights_by_line = vec![Vec::new(); line_starts.len()];
        for span in highlights {
            let first_line = line_starts
                .partition_point(|&start| start <= span.start_byte)
                .saturating_sub(1);
            let last_line = line_starts
                .partition_point(|&start| start < span.end_byte)
                .saturating_sub(1);
            for line in first_line..=last_line.min(highlights_by_line.len() - 1) {
                highlights_by_line[line].push(span);
            }
        }
        self.tree = Some(tree);
        self.highlights_by_line = highlights_by_line;
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        self.language = None;
        self.parser = None;
        self.query = None;
        self.tree = None;
        self.highlights_by_line.clear();
    }

    pub(crate) fn set_languages(
        &mut self,
        languages: Vec<SyntaxLanguageConfig>,
    ) -> Result<(), String> {
        let mut languages_by_extension = BTreeMap::new();
        for language in languages {
            let grammar = grammar_language(&language.grammar)?;
            Query::new(&grammar, &language.highlights_query).map_err(|error| {
                format!(
                    "Failed to load highlight query for language '{}': {error}",
                    language.name
                )
            })?;
            if language.extensions.is_empty() {
                return Err(format!(
                    "Language '{}' must register at least one file extension",
                    language.name
                ));
            }
            for extension in &language.extensions {
                let extension = extension.trim_start_matches('.').to_ascii_lowercase();
                if extension.is_empty() {
                    return Err(format!(
                        "Language '{}' contains an empty file extension",
                        language.name
                    ));
                }
                if languages_by_extension
                    .insert(extension.clone(), language.clone())
                    .is_some()
                {
                    return Err(format!(
                        "More than one syntax language registers the '.{extension}' extension"
                    ));
                }
            }
        }
        self.languages_by_extension = languages_by_extension;
        self.clear();
        Ok(())
    }

    #[cfg(test)]
    fn highlights(&self) -> impl Iterator<Item = &HighlightSpan> {
        self.highlights_by_line.iter().flatten()
    }

    pub(crate) fn line_highlights(&self, line: usize) -> &[HighlightSpan] {
        self.highlights_by_line.get(line).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn enclosing_function(
        &self,
        buffer: &Rope,
        cursor: usize,
    ) -> Option<(usize, usize)> {
        let language = self.language.as_ref()?;
        let root = self.tree.as_ref()?.root_node();
        let byte = buffer.char_to_byte(cursor.min(buffer.len_chars()));
        let mut stack = vec![root];
        let mut selected = None;

        while let Some(node) = stack.pop() {
            let start_byte = node.start_byte();
            let end_byte = node.end_byte();
            if byte < start_byte || byte > end_byte {
                continue;
            }
            if language
                .function_nodes
                .iter()
                .any(|kind| kind == node.kind())
                && selected.is_none_or(|(start, end)| end_byte - start_byte < end - start)
            {
                selected = Some((start_byte, end_byte));
            }
            for index in 0..node.child_count() {
                if let Some(child) = node.child(index) {
                    stack.push(child);
                }
            }
        }

        selected.map(|(start, end)| (buffer.byte_to_char(start), buffer.byte_to_char(end)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin_languages() -> Vec<SyntaxLanguageConfig> {
        let colors = BTreeMap::from([
            ("keyword".to_string(), (Color::from_rgb(0.7, 0.4, 0.9), 2)),
            (
                "function.macro".to_string(),
                (Color::from_rgb(0.9, 0.7, 0.4), 3),
            ),
        ]);
        vec![
            SyntaxLanguageConfig {
                name: "rust".to_string(),
                grammar: "rust".to_string(),
                extensions: vec!["rs".to_string()],
                highlights_query: tree_sitter_rust::HIGHLIGHTS_QUERY.to_string(),
                function_nodes: vec![
                    "function_item".to_string(),
                    "function_signature_item".to_string(),
                ],
                capture_colors: colors.clone(),
                completion_server: None,
            },
            SyntaxLanguageConfig {
                name: "python".to_string(),
                grammar: "python".to_string(),
                extensions: vec!["py".to_string(), "pyi".to_string()],
                highlights_query: tree_sitter_python::HIGHLIGHTS_QUERY.to_string(),
                function_nodes: vec!["function_definition".to_string()],
                capture_colors: colors,
                completion_server: None,
            },
        ]
    }

    #[test]
    fn rust_highlights_and_function_ranges_are_parsed() {
        let source = "fn greet() {\n    println!(\"hi\");\n}\n";
        let mut syntax = SyntaxState::default();
        syntax.set_languages(builtin_languages()).unwrap();
        syntax
            .set_file_path(std::path::Path::new("greet.rs"), source)
            .unwrap();

        assert!(
            syntax
                .highlights()
                .any(|span| { &source[span.start_byte..span.end_byte] == "fn" })
        );
        assert!(
            syntax
                .line_highlights(1)
                .iter()
                .any(|span| &source[span.start_byte..span.end_byte] == "println")
        );
        let buffer = Rope::from_str(source);
        let cursor = source.find("println").unwrap();
        let range = syntax.enclosing_function(&buffer, cursor).unwrap();
        assert_eq!(
            &source[buffer.char_to_byte(range.0)..buffer.char_to_byte(range.1)],
            source.trim_end()
        );
    }

    #[test]
    fn python_selects_the_smallest_enclosing_function_and_supports_unicode_offsets() {
        let source = "def outer():\n    label = 'café'\n    def inner():\n        return label\n";
        let mut syntax = SyntaxState::default();
        syntax.set_languages(builtin_languages()).unwrap();
        syntax
            .set_file_path(std::path::Path::new("example.py"), source)
            .unwrap();

        let buffer = Rope::from_str(source);
        let cursor = source.find("return").unwrap();
        let cursor = source[..cursor].chars().count();
        let range = syntax.enclosing_function(&buffer, cursor).unwrap();
        let selected = &source[buffer.char_to_byte(range.0)..buffer.char_to_byte(range.1)];
        assert!(selected.starts_with("def inner()"));
        assert!(selected.contains("return label"));
    }

    #[test]
    fn unsupported_file_extensions_disable_syntax() {
        let mut syntax = SyntaxState::default();
        syntax.set_languages(builtin_languages()).unwrap();
        syntax
            .set_file_path(std::path::Path::new("example.rs"), "fn main() {}")
            .unwrap();
        syntax
            .set_file_path(std::path::Path::new("notes.txt"), "plain text")
            .unwrap();

        assert_eq!(syntax.highlights().count(), 0);
        assert!(
            syntax
                .enclosing_function(&Rope::from_str("plain text"), 0)
                .is_none()
        );
    }
}
