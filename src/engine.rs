use std::time::Instant;

use tree_sitter::Node;

use crate::{
    analyzer::{Analyzer, LanguageKind},
    helpers::{
        is_root_like, kind_is_context, kind_is_tiny, language_for_path, merge_ranges,
        node_line_span, render_segment_with_highlights,
    },
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
            let Ok(parsed) = self.analyzer.get_or_load_parsed(&file_path) else {
                continue;
            };

            let mut candidates: Vec<SnippetCandidate> = matches
                .into_iter()
                .map(|m| Self::analyze_match(&file_path, parsed, &m))
                .collect();

            outputs.extend(Engine::build_output_records_for_file(
                &file_path,
                parsed,
                candidates.as_mut(),
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
            Some(Engine::select_context_node(parsed, root, node, language))
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

    fn build_output_records_for_file(
        file_path: &str,
        parsed: &ParsedFile,
        candidates: &mut Vec<SnippetCandidate>,
    ) -> Vec<OutputRecord> {
        candidates.sort_by(|a, b| {
            a.snippet_start
                .cmp(&b.snippet_start)
                .then(a.snippet_end.cmp(&b.snippet_end))
                .then(a.match_start.cmp(&b.match_start))
                .then(a.match_end.cmp(&b.match_end))
        });

        let merged = Engine::merge_candidates(candidates);
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

    fn merge_candidates(candidates: &Vec<SnippetCandidate>) -> Vec<MergedSnippet> {
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
            let need_more_context =
                selected_span < MIN_CONTEXT_LINES || kind_is_tiny(selected.kind());
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_rust(source: &str) -> ParsedFile {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("should configure rust parser");
        let tree = parser
            .parse(source, None)
            .expect("should parse rust source");
        ParsedFile::new(source.to_string(), Some(tree))
    }

    fn selected_context_node_for_needle<'a>(
        parsed: &'a ParsedFile,
        needle: &str,
    ) -> tree_sitter::Node<'a> {
        let tree = parsed.tree.as_ref().expect("tree should exist");
        let root = tree.root_node();

        let start = parsed
            .source
            .find(needle)
            .expect("needle should exist in source");
        let end = start + needle.len();

        let current = root
            .named_descendant_for_byte_range(start, end)
            .or_else(|| root.descendant_for_byte_range(start, end))
            .expect("should find AST node for needle");

        Engine::select_context_node(parsed, root, current, LanguageKind::Rust)
    }

    #[test]
    fn select_context_node_for_function_definition_name() {
        let source = r#"
fn target_fn(value: i32) -> i32 {
    let doubled = value * 2;
    doubled
}
"#;
        let expected_kind = "function_item";
        let expected_context = r#"
fn target_fn(value: i32) -> i32 {
    let doubled = value * 2;
    doubled
}
"#;

        let parsed = parse_rust(source);
        let selected = selected_context_node_for_needle(&parsed, "target_fn");
        assert_eq!(selected.kind(), expected_kind);
        let selected_text = selected
            .utf8_text(parsed.source.as_bytes())
            .expect("selected context should be valid utf8");
        assert_eq!(selected_text.trim(), expected_context.trim());
    }

    #[test]
    fn select_context_node_for_multiline_let_assignment() {
        let source = r#"
fn main() {
    let target =
        21 + 21;
    println!("{}", target);
}
"#;
        let expected_kind = "let_declaration";
        let expected_context = r#"
let target =
        21 + 21;
"#;

        let parsed = parse_rust(source);
        let selected = selected_context_node_for_needle(&parsed, "target");
        assert_eq!(selected.kind(), expected_kind);
        let selected_text = selected
            .utf8_text(parsed.source.as_bytes())
            .expect("selected context should be valid utf8");
        assert_eq!(selected_text.trim(), expected_context.trim());
    }

    #[test]
    fn select_context_node_for_multiline_call_usage() {
        let source = r#"
fn main() {
    let result = compute_target(
        10,
        20,
    );
    println!("{}", result);
}
"#;
        let expected_kind = "call_expression";
        let expected_context = r#"
let result = compute_target(
        10,
        20,
    )
"#;

        let parsed = parse_rust(source);
        let selected = selected_context_node_for_needle(&parsed, "compute_target");
        assert_eq!(selected.kind(), expected_kind);
        let selected_text = selected
            .utf8_text(parsed.source.as_bytes())
            .expect("selected context should be valid utf8");
        assert_eq!(selected_text.trim(), expected_context.trim());
    }

    #[test]
    fn select_context_node_for_non_context_usage_falls_back_to_block() {
        let source = r#"
fn main() {
    let target = 10;
    let computed = target + 5;
    println!("{}", computed);
}
"#;
        let expected_kind = "block";
        let expected_context = r#"
{
    let target = 10;
    let computed = target + 5;
    println!("{}", computed);
}
"#;

        let parsed = parse_rust(source);
        let selected = selected_context_node_for_needle(&parsed, "target");
        assert_eq!(selected.kind(), expected_kind);
        let selected_text = selected
            .utf8_text(parsed.source.as_bytes())
            .expect("selected context should be valid utf8");
        assert_eq!(selected_text.trim(), expected_context.trim());
    }
}
