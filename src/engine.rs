use std::time::Instant;

use tree_sitter::Node;

use crate::{
    analyzer::{Analyzer, LanguageKind},
    helpers::{language_for_path, merge_ranges, render_segment_with_highlights},
    output::{OutputRecord, print_outputs},
    parsed_file::ParsedFile,
    ripgrep::{GrepConfig, MatchOccurence, RipGrep},
};

pub struct Engine {
    analyzer: Analyzer,
}

#[derive(Debug, Clone)]
struct SnippetCandidate {
    line_num: u64,
    snippet_start: usize,
    snippet_end: usize,
    match_start: usize,
    match_end: usize,
}

#[derive(Debug, Default)]
struct MergedSnippet {
    line_num: u64,
    snippet_start: usize,
    snippet_end: usize,
    highlights: Vec<(usize, usize)>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            analyzer: Analyzer::new(),
        }
    }

    pub fn run(&mut self, path: &str, pattern: &str) {
        let start = Instant::now();

        let mut ripgrep = RipGrep::new(GrepConfig::new(pattern, path));
        let matches_by_file = match ripgrep.run() {
            Ok(v) => v,
            Err(err) => {
                eprintln!("ripgrep run failed: {err}");
                return;
            }
        };

        let mut outputs: Vec<OutputRecord> = Vec::new();
        for (file_path, matches) in matches_by_file {
            let parsed = match self.analyzer.get_or_load_parsed(&file_path) {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("{err}");
                    continue;
                }
            };

            let candidates: Vec<SnippetCandidate> = matches
                .into_iter()
                .map(|m| Self::analyze_match(&file_path, parsed, &m))
                .collect();

            outputs.extend(build_output_records_for_file(
                &file_path, parsed, candidates,
            ));
        }

        outputs.sort_by(|a, b| a.path.cmp(&b.path).then(a.line_num.cmp(&b.line_num)));
        print_outputs(&outputs);

        let elapsed = start.elapsed();
        println!("Completed in {:?}", elapsed);
    }

    fn analyze_match(file_path: &str, parsed: &ParsedFile, m: &MatchOccurence) -> SnippetCandidate {
        let global_start = m.start_byte as usize;
        let global_end = m.end_byte as usize;

        let context_node: Option<Node> = parsed.tree.as_ref().and_then(|tree| {
            let root = tree.root_node();
            let node = root
                .named_descendant_for_byte_range(global_start, global_end)
                .or_else(|| root.descendant_for_byte_range(global_start, global_end))?;
            let language = language_for_path(file_path)?;
            Some(select_context_node(parsed, root, node, language))
        });

        let (snippet_start, snippet_end): (usize, usize) = match context_node {
            Some(node) => (node.start_byte(), node.end_byte()),
            None => parsed.line_bounds_for_byte_range(global_start, global_end),
        };

        SnippetCandidate {
            line_num: m.line_number,
            snippet_start,
            snippet_end,
            match_start: m.start_byte as usize,
            match_end: m.end_byte as usize,
        }
    }
}

fn build_output_records_for_file(
    file_path: &str,
    parsed: &ParsedFile,
    mut candidates: Vec<SnippetCandidate>,
) -> Vec<OutputRecord> {
    candidates.sort_by(|a, b| {
        a.snippet_start
            .cmp(&b.snippet_start)
            .then(a.snippet_end.cmp(&b.snippet_end))
            .then(a.match_start.cmp(&b.match_start))
            .then(a.match_end.cmp(&b.match_end))
    });

    let merged = merge_candidates(candidates);
    merged
        .into_iter()
        .map(|m| {
            let (node_line_from, node_line_to) =
                parsed.line_bounds_for_byte_range(m.snippet_start, m.snippet_end);
            let rendered_lines = render_segment_with_highlights(
                &parsed.source,
                m.snippet_start,
                m.snippet_end,
                &m.highlights,
            );

            OutputRecord {
                path: file_path.to_string(),
                line_num: m.line_num,
                node_line_from,
                node_line_to,
                rendered_lines,
            }
        })
        .collect()
}

fn merge_candidates(candidates: Vec<SnippetCandidate>) -> Vec<MergedSnippet> {
    let mut out: Vec<MergedSnippet> = Vec::new();

    for c in candidates {
        match out.last_mut() {
            Some(current) if c.snippet_start <= current.snippet_end => {
                current.snippet_end = current.snippet_end.max(c.snippet_end);
                current.line_num = current.line_num.min(c.line_num);
                current.highlights.push((c.match_start, c.match_end));
            }
            _ => {
                out.push(MergedSnippet {
                    line_num: c.line_num,
                    snippet_start: c.snippet_start,
                    snippet_end: c.snippet_end,
                    highlights: vec![(c.match_start, c.match_end)],
                });
            }
        }
    }

    for merged in &mut out {
        merged.highlights.sort_unstable_by_key(|&(s, e)| (s, e));
        merged.highlights = merge_ranges(&merged.highlights);
    }

    out
}

fn select_context_node<'a>(
    parsed: &ParsedFile,
    root: tree_sitter::Node<'a>,
    current: tree_sitter::Node<'a>,
    language: LanguageKind,
) -> tree_sitter::Node<'a> {
    const MIN_CONTEXT_LINES: usize = 2;
    const MAX_CONTEXT_LINES: usize = 200;
    const MAX_ANCESTOR_STEPS: usize = 8;

    let mut selected = current;
    let mut cursor = current;

    for _ in 0..MAX_ANCESTOR_STEPS {
        let parent = match cursor.parent() {
            Some(p) => p,
            None => break,
        };
        if is_root_like(parent, root) {
            break;
        }

        let parent_span = node_line_span(parsed, parent);
        if parent_span > MAX_CONTEXT_LINES {
            break;
        }

        let selected_span = node_line_span(parsed, selected);
        let need_more_context = selected_span < MIN_CONTEXT_LINES || kind_is_tiny(selected.kind());
        let parent_is_context = kind_is_context(language, parent.kind());

        if need_more_context || parent_is_context {
            selected = parent;
        }

        if parent_is_context && node_line_span(parsed, selected) >= MIN_CONTEXT_LINES {
            break;
        }

        cursor = parent;
    }

    selected
}

fn is_root_like(node: tree_sitter::Node<'_>, root: tree_sitter::Node<'_>) -> bool {
    let same_as_root_range =
        node.start_byte() == root.start_byte() && node.end_byte() == root.end_byte();
    let root_like_kind = matches!(node.kind(), "source_file" | "program");
    same_as_root_range || root_like_kind
}

fn node_line_span(parsed: &ParsedFile, node: tree_sitter::Node<'_>) -> usize {
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

fn kind_is_tiny(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "string_content"
    )
}

fn kind_is_context(language: LanguageKind, kind: &str) -> bool {
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
