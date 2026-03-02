use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead, BufReader},
    process::{Command, Stdio},
    time::Instant,
};
use tree_sitter::{Parser, Tree};

#[derive(Debug, Deserialize)]
struct RgEventLine {
    #[serde(rename = "type")]
    event_type: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct MatchData {
    path: TextField,
    lines: TextField,
    line_number: u64,
    absolute_offset: u64,
    submatches: Vec<SubmatchData>,
}

#[derive(Debug, Deserialize)]
struct TextField {
    text: String,
}

#[derive(Debug, Deserialize)]
struct SubmatchData {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct MatchOccurrence {
    line_num: u64,
    line_text: String,
    absolute_offset: usize,
    submatch_start: usize,
    submatch_end: usize,
}

#[derive(Debug)]
struct ParsedFile {
    source: String,
    lines: Vec<String>,
    tree: Tree,
}

#[derive(Debug, Default)]
struct CacheStats {
    hits: usize,
    misses: usize,
}

struct Analyzer {
    parser: Parser,
    files: HashMap<String, ParsedFile>,
    stats: CacheStats,
}

impl Analyzer {
    fn new() -> Result<Self, String> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|_| "Failed to initialize tree-sitter Rust grammar".to_string())?;

        Ok(Self {
            parser,
            files: HashMap::new(),
            stats: CacheStats::default(),
        })
    }

    fn handle_match(&mut self, path: &str, m: MatchOccurrence) {
        if !path.ends_with(".rs") {
            return;
        }

        let parsed = match self.get_or_parse_file(path) {
            Some(p) => p,
            None => return,
        };

        let global_start = m.absolute_offset.saturating_add(m.submatch_start);
        let global_end = m.absolute_offset.saturating_add(m.submatch_end);

        if global_end <= global_start {
            eprintln!(
                "warning: skipping invalid range for {path}:{} line_range=[{}..{}]",
                m.line_num, m.submatch_start, m.submatch_end
            );
            return;
        }
        if global_start >= parsed.source.len() || global_end > parsed.source.len() {
            eprintln!(
                "warning: skipping out-of-bounds range for {path}:{} file_range=[{}..{}] source_len={}",
                m.line_num,
                global_start,
                global_end,
                parsed.source.len()
            );
            return;
        }

        let root = parsed.tree.root_node();
        let node = root
            .named_descendant_for_byte_range(global_start, global_end)
            .or_else(|| root.descendant_for_byte_range(global_start, global_end));

        match node {
            Some(current) => {
                let (from, to, covered_lines) = node_lines(parsed, current);
                println!(
                    "{path}:{} node_type={} node_lines=[{}..{}]\n{}",
                    m.line_num,
                    current.kind(),
                    from,
                    to,
                    covered_lines
                );
            }
            None => {
                eprintln!(
                    "warning: no syntax node found for {path}:{} text={}",
                    m.line_num,
                    m.line_text.trim_end()
                );
            }
        }
    }

    fn get_or_parse_file(&mut self, path: &str) -> Option<&ParsedFile> {
        if self.files.contains_key(path) {
            self.stats.hits += 1;
            return self.files.get(path);
        }

        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Failed to read {path}: {err}");
                return None;
            }
        };

        let tree = match self.parser.parse(&source, None) {
            Some(t) => t,
            None => {
                eprintln!("Failed to parse {path} with tree-sitter");
                return None;
            }
        };

        let lines = source.lines().map(ToString::to_string).collect::<Vec<_>>();
        self.files.insert(
            path.to_string(),
            ParsedFile {
                source,
                lines,
                tree,
            },
        );
        self.stats.misses += 1;
        self.files.get(path)
    }
}

fn node_lines(parsed: &ParsedFile, node: tree_sitter::Node<'_>) -> (usize, usize, String) {
    if parsed.lines.is_empty() {
        return (0, 0, String::new());
    }

    let start_row = node.start_position().row;
    let mut end_row = node.end_position().row;
    if node.end_position().column == 0 && end_row > start_row {
        end_row = end_row.saturating_sub(1);
    }

    let last = parsed.lines.len() - 1;
    let from = start_row.min(last);
    let to = end_row.min(last);
    let covered = if from <= to {
        parsed.lines[from..=to].join("\n")
    } else {
        String::new()
    };

    (from + 1, to + 1, covered)
}

fn parse_match_occurrences(line: &str) -> Option<(String, Vec<MatchOccurrence>)> {
    let event: RgEventLine = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("Failed to parse rg JSON line: {err}\nline: {line}");
            return None;
        }
    };

    if event.event_type != "match" {
        return None;
    }

    let m: MatchData = match serde_json::from_value(event.data) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("Failed to parse rg match payload: {err}");
            return None;
        }
    };

    let path = m.path.text;
    let line_num = m.line_number;
    let line_text = m.lines.text;
    let absolute_offset = m.absolute_offset as usize;

    let occurrences = m
        .submatches
        .into_iter()
        .map(|sm| MatchOccurrence {
            line_num,
            line_text: line_text.clone(),
            absolute_offset,
            submatch_start: sm.start,
            submatch_end: sm.end,
        })
        .collect::<Vec<_>>();

    Some((path, occurrences))
}

fn stream_ripgrep(path: &str, pattern: &str, analyzer: &mut Analyzer) -> io::Result<()> {
    let mut child = Command::new("rg")
        .arg("-i")
        .arg(pattern)
        .arg(path)
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            return Err(io::Error::other("Failed to capture ripgrep stdout"));
        }
    };

    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = line?;
        if let Some((matched_path, occurrences)) = parse_match_occurrences(&line) {
            for occurrence in occurrences {
                analyzer.handle_match(&matched_path, occurrence);
            }
        }
    }

    let status = child.wait()?;
    if !status.success() {
        eprintln!("ripgrep exited with status: {status}");
    }

    Ok(())
}

fn main() {
    let start = Instant::now();

    let mut analyzer = match Analyzer::new() {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err}");
            return;
        }
    };

    if let Err(err) = stream_ripgrep("./", "arg", &mut analyzer) {
        eprintln!("ripgrep stream failed: {err}");
        return;
    }

    let elapsed = start.elapsed();
    println!(
        "Completed in {:?} (cache hits: {}, misses: {})",
        elapsed, analyzer.stats.hits, analyzer.stats.misses
    );
}
