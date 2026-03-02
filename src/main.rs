use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    io::{self, BufRead, BufReader},
    path::Path,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LanguageKind {
    Rust,
    TypeScript,
    Tsx,
}

#[derive(Debug, Clone)]
struct MatchOccurrence {
    path: String,
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

#[derive(Debug)]
struct OutputRecord {
    path: String,
    line_num: u64,
    submatch_start: usize,
    node_type: String,
    node_line_from: usize,
    node_line_to: usize,
    covered_lines: String,
}

struct LanguageAnalyzer {
    parser: Parser,
    files: HashMap<String, ParsedFile>,
    stats: CacheStats,
}

impl LanguageAnalyzer {
    fn new(language: LanguageKind) -> Result<Self, String> {
        let mut parser = Parser::new();
        set_parser_language(&mut parser, language)
            .map_err(|msg| format!("Failed to initialize parser: {msg}"))?;

        Ok(Self {
            parser,
            files: HashMap::new(),
            stats: CacheStats::default(),
        })
    }

    fn analyze_match(&mut self, m: &MatchOccurrence) -> Option<OutputRecord> {
        let parsed = match self.get_or_parse_file(&m.path) {
            Some(p) => p,
            None => return None,
        };

        let global_start = m.absolute_offset.saturating_add(m.submatch_start);
        let global_end = m.absolute_offset.saturating_add(m.submatch_end);

        if global_end <= global_start {
            eprintln!(
                "warning: skipping invalid range for {}:{} line_range=[{}..{}]",
                m.path, m.line_num, m.submatch_start, m.submatch_end
            );
            return None;
        }
        if global_start >= parsed.source.len() || global_end > parsed.source.len() {
            eprintln!(
                "warning: skipping out-of-bounds range for {}:{} file_range=[{}..{}] source_len={}",
                m.path,
                m.line_num,
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
                let (from, to, covered_lines) = node_lines(parsed, current);
                Some(OutputRecord {
                    path: m.path.clone(),
                    line_num: m.line_num,
                    submatch_start: m.submatch_start,
                    node_type: current.kind().to_string(),
                    node_line_from: from,
                    node_line_to: to,
                    covered_lines,
                })
            }
            None => {
                eprintln!(
                    "warning: no syntax node found for {}:{} text={}",
                    m.path,
                    m.line_num,
                    m.line_text.trim_end()
                );
                None
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

fn set_parser_language(parser: &mut Parser, language: LanguageKind) -> Result<(), &'static str> {
    let result = match language {
        LanguageKind::Rust => parser.set_language(&tree_sitter_rust::LANGUAGE.into()),
        LanguageKind::TypeScript => {
            parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        }
        LanguageKind::Tsx => parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()),
    };

    result.map_err(|_| "unsupported tree-sitter language")
}

fn language_for_path(path: &str) -> Option<LanguageKind> {
    let extension = Path::new(path).extension()?.to_str()?;
    match extension {
        "rs" => Some(LanguageKind::Rust),
        "ts" | "js" => Some(LanguageKind::TypeScript),
        "tsx" | "jsx" => Some(LanguageKind::Tsx),
        _ => None,
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

fn parse_match_occurrences(line: &str) -> Option<Vec<MatchOccurrence>> {
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
            path: path.clone(),
            line_num,
            line_text: line_text.clone(),
            absolute_offset,
            submatch_start: sm.start,
            submatch_end: sm.end,
        })
        .collect::<Vec<_>>();

    Some(occurrences)
}

fn stream_ripgrep(
    path: &str,
    pattern: &str,
) -> io::Result<BTreeMap<LanguageKind, Vec<MatchOccurrence>>> {
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
    let mut matches_by_language: BTreeMap<LanguageKind, Vec<MatchOccurrence>> = BTreeMap::new();

    for line in reader.lines() {
        let line = line?;
        if let Some(occurrences) = parse_match_occurrences(&line) {
            for occurrence in occurrences {
                if let Some(language) = language_for_path(&occurrence.path) {
                    matches_by_language
                        .entry(language)
                        .or_default()
                        .push(occurrence);
                }
            }
        }
    }

    let status = child.wait()?;
    if !status.success() {
        eprintln!("ripgrep exited with status: {status}");
    }

    Ok(matches_by_language)
}

fn parse_cli_args() -> Result<(String, String), String> {
    let args = env::args().collect::<Vec<_>>();
    let bin = args
        .first()
        .map(String::as_str)
        .unwrap_or("project-symgrep");
    let usage = format!("Usage: {bin} <pattern> [path]");

    match args.len() {
        2 => {
            let pattern = args[1].clone();
            let path = env::current_dir()
                .map_err(|err| format!("{usage}\nFailed to determine current directory: {err}"))?
                .to_string_lossy()
                .to_string();
            Ok((pattern, path))
        }
        3 => Ok((args[1].clone(), args[2].clone())),
        _ => Err(usage),
    }
}

fn main() {
    let start = Instant::now();
    let (pattern, path) = match parse_cli_args() {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return;
        }
    };

    let matches_by_language = match stream_ripgrep(&path, &pattern) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("ripgrep stream failed: {err}");
            return;
        }
    };

    let mut analyzers: HashMap<LanguageKind, LanguageAnalyzer> = HashMap::new();
    let mut outputs: Vec<OutputRecord> = Vec::new();

    for (language, matches) in matches_by_language {
        if !analyzers.contains_key(&language) {
            match LanguageAnalyzer::new(language) {
                Ok(analyzer) => {
                    analyzers.insert(language, analyzer);
                }
                Err(err) => {
                    eprintln!("warning: skipping language analyzer init error: {err}");
                    continue;
                }
            }
        }
        let analyzer = match analyzers.get_mut(&language) {
            Some(a) => a,
            None => continue,
        };

        for m in matches {
            if let Some(record) = analyzer.analyze_match(&m) {
                outputs.push(record);
            }
        }
    }

    outputs.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line_num.cmp(&b.line_num))
            .then(a.submatch_start.cmp(&b.submatch_start))
    });

    for out in outputs {
        println!(
            "{}:{} node_type={} node_lines=[{}..{}]\n{}",
            out.path,
            out.line_num,
            out.node_type,
            out.node_line_from,
            out.node_line_to,
            out.covered_lines
        );
    }

    let total_hits: usize = analyzers.values().map(|a| a.stats.hits).sum();
    let total_misses: usize = analyzers.values().map(|a| a.stats.misses).sum();
    let elapsed = start.elapsed();
    println!(
        "Completed in {:?} (cache hits: {}, misses: {})",
        elapsed, total_hits, total_misses
    );
}
