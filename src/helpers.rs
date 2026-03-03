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

pub fn line_bounds_for_byte_range(
    line_starts: &[usize],
    line_ends: &[usize],
    start_byte: usize,
    end_byte: usize,
) -> (usize, usize) {
    if line_starts.is_empty() || line_ends.is_empty() {
        return (0, 0);
    }

    let from = find_line_for_byte(line_starts, start_byte);
    let end_inclusive = end_byte.saturating_sub(1);
    let to = find_line_for_byte(line_starts, end_inclusive);
    (from + 1, to + 1)
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

fn find_line_for_byte(line_starts: &[usize], byte: usize) -> usize {
    let idx = line_starts.partition_point(|&s| s <= byte);
    idx.saturating_sub(1)
        .min(line_starts.len().saturating_sub(1))
}
