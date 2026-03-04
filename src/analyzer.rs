use std::{collections::HashMap, fs};

use tree_sitter::{Parser, Tree};

use crate::{
    LanguageKind,
    helpers::{compute_line_offsets, language_for_path},
};

pub(crate) struct ParsedFile {
    pub source: String,
    pub lines: Vec<String>,
    pub line_starts: Vec<usize>,
    pub line_ends: Vec<usize>,
    pub tree: Tree,
}

pub struct AnalyzerStats {
    hits: usize,
    misses: usize,
}

pub struct Analyzer {
    parsers: HashMap<LanguageKind, Parser>,
    files: HashMap<String, ParsedFile>,
    stats: AnalyzerStats,
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            parsers: HashMap::new(),
            files: HashMap::new(),
            stats: AnalyzerStats { hits: 0, misses: 0 },
        }
    }

    pub fn get_or_load_parsed(&mut self, path: &str) -> Result<&ParsedFile, String> {
        if !self.files.contains_key(path) {
            self.load_file(path)?;
            self.stats.misses += 1;
        } else {
            self.stats.hits += 1;
        }

        self.files
            .get(path)
            .ok_or_else(|| "Unexpected missing parsed file after load_file".to_string())
    }

    fn load_file(&mut self, path: &str) -> Result<(), String> {
        let language = match language_for_path(path) {
            Some(l) => l,
            None => return Err(format!("Unknown file extension for file: {path}")),
        };

        self.load_parser(language)?;

        let parser = match self.parsers.get_mut(&language) {
            Some(p) => p,
            None => {
                return Err(format!(
                    "Parser for language '{:?}' has not been initialized",
                    language
                ));
            }
        };

        let source =
            fs::read_to_string(path).map_err(|err| format!("Failed to read {path}: {err}"))?;

        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| format!("Failed to parse {path} with tree-sitter"))?;

        let lines = source.lines().map(ToString::to_string).collect::<Vec<_>>();
        let (line_starts, line_ends) = compute_line_offsets(&source);

        self.files.insert(
            path.to_string(),
            ParsedFile {
                source,
                lines,
                line_starts,
                line_ends,
                tree,
            },
        );
        Ok(())
    }

    fn load_parser(&mut self, language: LanguageKind) -> Result<(), String> {
        if self.parsers.contains_key(&language) {
            return Ok(());
        }

        let mut parser = tree_sitter::Parser::new();

        let result = match language {
            LanguageKind::Rust => parser.set_language(&tree_sitter_rust::LANGUAGE.into()),
            LanguageKind::TypeScript => {
                parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            }
            LanguageKind::Tsx => parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()),
        };

        if result.is_err() {
            return Err(format!(
                "Unsupported tree-sitter language '{:?}' requested",
                language
            ));
        }

        self.parsers.insert(language, parser);
        Ok(())
    }
}
