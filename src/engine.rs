use std::{
    collections::{BTreeSet, HashMap},
    time::Instant,
};

use crate::{
    LanguageKind,
    analyzer::{Analyzer, ParsedFile},
    helpers::{line_bounds_for_byte_range, merge_ranges, render_segment_with_highlights},
    output::{OutputRecord, print_outputs},
    ripgrep::{MatchOccurence, stream_ripgrep},
};

#[derive(Debug, Default)]
struct CacheStats {
    hits: usize,
    misses: usize,
}

pub struct SymgrepEngine {
    analyzer: Analyzer,
    stats: CacheStats,
}

#[derive(Debug, Clone)]
struct SnippetCandidate {
    file_path: String,
    line_num: u64,
    node_type: String,
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
    node_types: BTreeSet<String>,
}

impl SymgrepEngine {
    pub fn new() -> Self {
        Self {
            analyzer: Analyzer::new(),
            stats: CacheStats::default(),
        }
    }

    pub fn run(path: &str, pattern: &str) {
        let start = Instant::now();
        let mut engine = Self::new();

        let matches_by_language = match stream_ripgrep(path, pattern) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("ripgrep stream failed: {err}");
                return;
            }
        };

        let mut candidates_by_file: HashMap<String, Vec<SnippetCandidate>> = HashMap::new();

        for (language, matches) in matches_by_language {
            for m in matches {
                if let Some(candidate) = engine.analyze_match(language, &m) {
                    candidates_by_file
                        .entry(candidate.file_path.clone())
                        .or_default()
                        .push(candidate);
                }
            }
        }

        let mut outputs: Vec<OutputRecord> = Vec::new();
        for (file_path, candidates) in candidates_by_file {
            let parsed = match engine.analyzer.get_or_load_parsed(&file_path) {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("{err}");
                    continue;
                }
            };
            outputs.extend(build_output_records_for_file(
                &file_path, parsed, candidates,
            ));
        }

        outputs.sort_by(|a, b| a.path.cmp(&b.path).then(a.line_num.cmp(&b.line_num)));
        print_outputs(&outputs);

        let elapsed = start.elapsed();
        println!(
            "Completed in {:?} (cache hits: {}, misses: {})",
            elapsed, engine.stats.hits, engine.stats.misses
        );
    }

    fn analyze_match(
        &mut self,
        language: LanguageKind,
        m: &MatchOccurence,
    ) -> Option<SnippetCandidate> {
        if self.analyzer.has_file(&m.file_path) {
            self.stats.hits += 1;
        } else {
            self.stats.misses += 1;
        }

        let parsed = match self.analyzer.get_or_load_parsed(&m.file_path) {
            Ok(p) => p,
            Err(err) => {
                eprintln!("{err}");
                return None;
            }
        };

        let global_start = m.start_byte as usize;
        let global_end = m.end_byte as usize;

        if global_end <= global_start {
            eprintln!(
                "warning: skipping invalid range for {}:{} line_range=[{}..{}]",
                m.file_path, m.line_number, m.start_byte, m.end_byte
            );
            return None;
        }
        if global_start >= parsed.source.len() || global_end > parsed.source.len() {
            eprintln!(
                "warning: skipping out-of-bounds range for {}:{} file_range=[{}..{}] source_len={}",
                m.file_path,
                m.line_number,
                global_start,
                global_end,
                parsed.source.len()
            );
            return None;
        }

        let root = parsed.tree.root_node();
        let node = root
            .named_descendant_for_byte_range(global_start, global_end)
            .or_else(|| root.descendant_for_byte_range(global_start, global_end));

        match node {
            Some(current) => {
                let bounds_node = select_context_node(parsed, root, current, language);
                Some(SnippetCandidate {
                    file_path: m.file_path.clone(),
                    line_num: m.line_number,
                    node_type: current.kind().to_string(),
                    snippet_start: bounds_node.start_byte(),
                    snippet_end: bounds_node.end_byte(),
                    match_start: global_start,
                    match_end: global_end,
                })
            }
            None => {
                eprintln!(
                    "warning: no syntax node found for {}:{}",
                    m.file_path, m.line_number
                );
                None
            }
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
            let (node_line_from, node_line_to) = line_bounds_for_byte_range(
                &parsed.line_starts,
                &parsed.line_ends,
                m.snippet_start,
                m.snippet_end,
            );
            let rendered_lines = render_segment_with_highlights(
                &parsed.source,
                m.snippet_start,
                m.snippet_end,
                &m.highlights,
            );
            let node_type = if m.node_types.len() == 1 {
                m.node_types
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "unknown".to_string())
            } else {
                let joined = m.node_types.into_iter().collect::<Vec<_>>().join(",");
                format!("multi[{joined}]")
            };

            OutputRecord {
                path: file_path.to_string(),
                line_num: m.line_num,
                node_type,
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
                current.node_types.insert(c.node_type);
            }
            _ => {
                let mut node_types = BTreeSet::new();
                node_types.insert(c.node_type);
                out.push(MergedSnippet {
                    line_num: c.line_num,
                    snippet_start: c.snippet_start,
                    snippet_end: c.snippet_end,
                    highlights: vec![(c.match_start, c.match_end)],
                    node_types,
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

    if parsed.lines.is_empty() {
        return 0;
    }

    let last = parsed.lines.len() - 1;
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
