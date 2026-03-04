use tree_sitter::Tree;

use crate::helpers::compute_line_byte_offsets;

pub(crate) struct ParsedFile {
    pub source: String,
    pub line_count: usize,
    pub line_byte_offsets: Vec<usize>,
    pub tree: Option<Tree>,
}

impl ParsedFile {
    pub fn new(source: String, tree: Option<Tree>) -> Self {
        let offsets = compute_line_byte_offsets(&source);

        Self {
            source,
            line_count: offsets.len(),
            line_byte_offsets: offsets,
            tree,
        }
    }

    pub fn line_bounds_for_byte_range(&self, start: usize, end: usize) -> (usize, usize) {
        // 0-based indices. Add 1 at presentation boundaries for human-facing line numbers.
        let line_start = self.line_for_byte(start);

        // end-exclusive
        let line_end = if end > start {
            self.line_for_byte(end.saturating_sub(1))
        } else {
            line_start
        };

        (line_start, line_end)
    }

    fn line_for_byte(&self, byte: usize) -> usize {
        // 0-based index clamped to the last known line.
        let idx = self.line_byte_offsets.partition_point(|&s| s <= byte);
        idx.saturating_sub(1).min(self.line_count.saturating_sub(1))
    }
}
