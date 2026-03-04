use std::path::Path;

use crate::analyzer::LanguageKind;

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
