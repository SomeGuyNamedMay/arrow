use iced::Color;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct AstNode {
    pub(crate) kind: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) capture: Option<String>,
    pub(crate) children: Vec<AstNode>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HighlightSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) color: Color,
    pub(crate) priority: u8,
}

#[derive(Clone)]
pub(crate) struct SyntaxLanguageConfig {
    pub(crate) name: String,
    pub(crate) extensions: Vec<String>,
    pub(crate) highlight_colors: BTreeMap<String, (Color, u8)>,
    pub(crate) text_object_kinds: Vec<String>,
    pub(crate) language_server: Option<LanguageServerConfig>,
}

#[derive(Clone, Debug)]
pub(crate) struct LanguageServerConfig {
    pub(crate) command: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) root_markers: Vec<String>,
}

impl AstNode {
    pub(crate) fn highlights(
        &self,
        colors: &BTreeMap<String, (Color, u8)>,
        highlights: &mut Vec<HighlightSpan>,
    ) {
        if let Some((color, priority)) = self
            .capture
            .as_ref()
            .and_then(|capture| colors.get(capture))
        {
            highlights.push(HighlightSpan {
                start: self.start,
                end: self.end,
                color: *color,
                priority: *priority,
            });
        }
        for child in &self.children {
            child.highlights(colors, highlights);
        }
    }

    pub(crate) fn enclosing_text_object(
        &self,
        cursor: usize,
        kinds: &[String],
    ) -> Option<(usize, usize)> {
        let mut selected = kinds
            .iter()
            .any(|kind| kind == &self.kind)
            .then_some((self.start, self.end));
        for child in &self.children {
            if cursor >= child.start
                && cursor <= child.end
                && let Some(candidate) = child.enclosing_text_object(cursor, kinds)
                && selected.is_none_or(|(start, end)| candidate.1 - candidate.0 < end - start)
            {
                selected = Some(candidate);
            }
        }

        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ast_collects_highlights_and_selects_smallest_enclosing_text_object() {
        let tree = AstNode {
            kind: "module".to_string(),
            start: 0,
            end: 22,
            capture: None,
            children: vec![AstNode {
                kind: "function".to_string(),
                start: 0,
                end: 22,
                capture: None,
                children: vec![
                    AstNode {
                        kind: "keyword".to_string(),
                        start: 0,
                        end: 2,
                        capture: Some("keyword".to_string()),
                        children: Vec::new(),
                    },
                    AstNode {
                        kind: "function".to_string(),
                        start: 10,
                        end: 20,
                        capture: None,
                        children: Vec::new(),
                    },
                ],
            }],
        };
        let color = Color::from_rgb(0.7, 0.4, 0.9);
        let colors = BTreeMap::from([("keyword".to_string(), (color, 2))]);
        let mut highlights = Vec::new();
        tree.highlights(&colors, &mut highlights);

        assert_eq!(highlights.len(), 1);
        assert_eq!((highlights[0].start, highlights[0].end), (0, 2));
        assert_eq!(highlights[0].color, color);
        assert_eq!(
            tree.enclosing_text_object(12, &["function".to_string()]),
            Some((10, 20))
        );
    }
}
