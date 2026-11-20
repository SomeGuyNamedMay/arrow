//! Colouring the visible text from the syntax tree and a highlights query.

use std::ops::Range;
use std::sync::Arc;

use iced::Color;
use ropey::Rope;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

/// A compiled highlights query, with the colour for each of its captures.
pub struct HighlightQuery {
    pub(super) query: Query,
    /// By capture index.
    pub(super) colors: Vec<Option<Color>>,
}

/// What the view needs to colour the text: the tree and the query. Cheap to
/// clone (both are shared).
#[derive(Clone)]
pub struct Highlighter {
    pub(super) tree: Tree,
    pub(super) query: Arc<HighlightQuery>,
}

impl Highlighter {
    /// The colour of every character in `chars`, which should cover only what
    /// is on screen. Where captures nest, the innermost wins; where several
    /// patterns capture the same node, the first one in the query wins.
    pub fn colors(&self, rope: &Rope, chars: Range<usize>) -> Vec<Option<Color>> {
        let bytes = rope.char_to_byte(chars.start)..rope.char_to_byte(chars.end);
        let mut colors = vec![None; chars.len()];
        // Size of the node that set each character's colour.
        let mut owner = vec![usize::MAX; chars.len()];

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(bytes.clone());
        let text = |node: Node| {
            rope.byte_slice(node.byte_range())
                .chunks()
                .map(str::as_bytes)
        };
        let mut captures = cursor.captures(&self.query.query, self.tree.root_node(), text);
        while let Some((found, index)) = captures.next() {
            let capture = found.captures()[*index];
            let Some(color) = self.query.colors[capture.index as usize] else {
                continue;
            };
            let range = capture.node.byte_range();
            let (start, end) = (range.start.max(bytes.start), range.end.min(bytes.end));
            if start >= end {
                continue;
            }
            let first = rope.byte_to_char(start) - chars.start;
            let last = rope.byte_to_char(end) - chars.start;
            for i in first..last {
                if range.len() < owner[i] {
                    owner[i] = range.len();
                    colors[i] = Some(color);
                }
            }
        }
        colors
    }
}
