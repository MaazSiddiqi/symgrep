use std::{collections::HashMap, fs};

use tree_sitter::Parser;

use crate::{helpers::language_for_path, parsed_file::ParsedFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LanguageKind {
    Rust,
    TypeScript,
    Tsx,
}

pub struct Analyzer {
    parsers: HashMap<LanguageKind, Parser>,
    files: HashMap<String, ParsedFile>,
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            parsers: HashMap::new(),
            files: HashMap::new(),
        }
    }

    pub fn get_or_load_parsed(&mut self, path: &str) -> Result<&ParsedFile, String> {
        if !self.files.contains_key(path) {
            self.load_file(path)?;
        }

        self.files
            .get(path)
            .ok_or_else(|| "Unexpected missing parsed file after load_file".to_string())
    }

    fn load_file(&mut self, path: &str) -> Result<(), String> {
        let source =
            fs::read_to_string(path).map_err(|err| format!("Failed to read {path}: {err}"))?;

        let tree = match language_for_path(path) {
            Some(language) => {
                self.load_parser(language)?;

                let parser = self.parsers.get_mut(&language).ok_or_else(|| {
                    format!(
                        "Parser for language '{:?}' has not been initialized",
                        language
                    )
                })?;

                let tree = parser
                    .parse(&source, None)
                    .ok_or_else(|| format!("Failed to parse {path} with tree-sitter"))?;

                Some(tree)
            }
            None => None,
        };

        self.files
            .insert(path.to_string(), ParsedFile::new(source, tree));
        Ok(())
    }

    fn load_parser(&mut self, language: LanguageKind) -> Result<&Parser, String> {
        if self.parsers.contains_key(&language) {
            return Ok(&self.parsers[&language]);
        }

        let mut parser = tree_sitter::Parser::new();

        let ts_language = match language {
            LanguageKind::Rust => tree_sitter_rust::LANGUAGE,
            LanguageKind::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            LanguageKind::Tsx => tree_sitter_typescript::LANGUAGE_TSX,
        };

        parser.set_language(&ts_language.into()).map_err(|err| {
            format!("Failed to set tree-sitter language for '{language:?}': {err}")
        })?;

        self.parsers.insert(language, parser);
        Ok(&self.parsers[&language])
    }
}
