use std::path::Path;

use crate::LanguageKind;

pub fn language_for_path(path: &str) -> Option<LanguageKind> {
    let extension = Path::new(path).extension()?.to_str()?;
    match extension {
        "rs" => Some(LanguageKind::Rust),
        "ts" | "js" => Some(LanguageKind::TypeScript),
        "tsx" | "jsx" => Some(LanguageKind::Tsx),
        _ => None,
    }
}

pub fn compute_line_offsets(source: &str) -> (Vec<usize>, Vec<usize>) {
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut offset = 0usize;

    for segment in source.split_inclusive('\n') {
        starts.push(offset);
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        ends.push(offset.saturating_add(line.len()));
        offset = offset.saturating_add(segment.len());
    }

    (starts, ends)
}

pub fn node_lines_with_highlight(
    source: &str,
    lines: &[String],
    line_starts: &[usize],
    line_ends: &[usize],
    node: tree_sitter::Node<'_>,
    match_start: usize,
    match_end: usize,
) -> (usize, usize, String) {
    if lines.is_empty() {
        return (0, 0, String::new());
    }

    let (from, to) = node_row_bounds(lines.len(), node);
    let segment_start = line_starts[from];
    let segment_end = line_ends[to];
    let rendered =
        render_with_highlight(source, segment_start, segment_end, match_start, match_end);

    (from + 1, to + 1, rendered)
}

fn node_row_bounds(line_count: usize, node: tree_sitter::Node<'_>) -> (usize, usize) {
    if line_count == 0 {
        return (0, 0);
    }

    let start_row = node.start_position().row;
    let mut end_row = node.end_position().row;
    if node.end_position().column == 0 && end_row > start_row {
        end_row = end_row.saturating_sub(1);
    }

    let last = line_count - 1;
    let from = start_row.min(last);
    let to = end_row.min(last);
    (from, to)
}

fn render_with_highlight(
    source: &str,
    segment_start: usize,
    segment_end: usize,
    highlight_start: usize,
    highlight_end: usize,
) -> String {
    let overlap_start = highlight_start.max(segment_start);
    let overlap_end = highlight_end.min(segment_end);

    if overlap_start >= overlap_end {
        return String::from_utf8_lossy(&source.as_bytes()[segment_start..segment_end]).to_string();
    }

    let bytes = source.as_bytes();
    let prefix = String::from_utf8_lossy(&bytes[segment_start..overlap_start]);
    let highlighted = String::from_utf8_lossy(&bytes[overlap_start..overlap_end]);
    let suffix = String::from_utf8_lossy(&bytes[overlap_end..segment_end]);

    format!(
        "{prefix}{}{highlighted}{}{suffix}",
        crate::COLOR_HIGHLIGHT,
        crate::COLOR_RESET
    )
}
