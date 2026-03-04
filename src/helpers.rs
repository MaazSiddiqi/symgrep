use std::path::Path;

use crate::{analyzer::LanguageKind, parsed_file::ParsedFile};

pub fn language_for_path(path: &str) -> Option<LanguageKind> {
    let extension = Path::new(path).extension()?.to_str()?;
    match extension {
        "rs" => Some(LanguageKind::Rust),
        "ts" | "js" => Some(LanguageKind::TypeScript),
        "tsx" | "jsx" => Some(LanguageKind::Tsx),
        _ => None,
    }
}

pub fn compute_line_byte_offsets(source: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = 0usize;

    for segment in source.split_inclusive('\n') {
        starts.push(offset);
        offset = offset.saturating_add(segment.len());
    }

    starts
}

pub fn render_segment_with_highlights(
    source: &str,
    segment_start: usize,
    segment_end: usize,
    highlights: &[(usize, usize)],
) -> String {
    let mut merged = highlights.to_vec();
    merged.sort_unstable_by_key(|&(s, e)| (s, e));
    merged = merge_ranges(&merged);

    let bytes = source.as_bytes();
    let mut rendered = String::new();
    let mut cursor = segment_start;

    for (start, end) in merged {
        let overlap_start = start.max(segment_start);
        let overlap_end = end.min(segment_end);
        if overlap_start >= overlap_end {
            continue;
        }

        if cursor < overlap_start {
            rendered.push_str(&String::from_utf8_lossy(&bytes[cursor..overlap_start]));
        }
        rendered.push_str(crate::COLOR_HIGHLIGHT);
        rendered.push_str(&String::from_utf8_lossy(&bytes[overlap_start..overlap_end]));
        rendered.push_str(crate::COLOR_RESET);
        cursor = overlap_end;
    }

    if cursor < segment_end {
        rendered.push_str(&String::from_utf8_lossy(&bytes[cursor..segment_end]));
    }

    rendered
}

pub fn merge_ranges(ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut merged: Vec<(usize, usize)> = Vec::new();

    for &(start, end) in ranges {
        if end <= start {
            continue;
        }

        match merged.last_mut() {
            Some((last_start, last_end)) if start <= *last_end => {
                *last_end = (*last_end).max(end);
                *last_start = (*last_start).min(start);
            }
            _ => merged.push((start, end)),
        }
    }

    merged
}

pub fn is_root_like(node: tree_sitter::Node<'_>, root: tree_sitter::Node<'_>) -> bool {
    let same_as_root_range =
        node.start_byte() == root.start_byte() && node.end_byte() == root.end_byte();
    let root_like_kind = matches!(node.kind(), "source_file" | "program");
    same_as_root_range || root_like_kind
}

pub fn node_line_span(parsed: &ParsedFile, node: tree_sitter::Node<'_>) -> usize {
    let start_row = node.start_position().row;
    let mut end_row = node.end_position().row;
    if node.end_position().column == 0 && end_row > start_row {
        end_row = end_row.saturating_sub(1);
    }

    if parsed.line_count == 0 {
        return 0;
    }

    let last = parsed.line_count;
    let from = start_row.min(last);
    let to = end_row.min(last);
    to.saturating_sub(from).saturating_add(1)
}

pub fn kind_is_tiny(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "string_content"
    )
}

pub fn kind_is_context(language: LanguageKind, kind: &str) -> bool {
    match language {
        LanguageKind::Rust => matches!(
            kind,
            "let_declaration"
                | "assignment_expression"
                | "call_expression"
                | "if_expression"
                | "for_expression"
                | "while_expression"
                | "match_expression"
                | "block"
                | "function_item"
        ),
        LanguageKind::TypeScript | LanguageKind::Tsx => matches!(
            kind,
            "variable_declarator"
                | "assignment_expression"
                | "call_expression"
                | "if_statement"
                | "for_statement"
                | "while_statement"
                | "statement_block"
                | "function_declaration"
                | "method_definition"
        ),
    }
}
