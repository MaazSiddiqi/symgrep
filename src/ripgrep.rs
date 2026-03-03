use serde::Deserialize;
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader},
    process::{Command, Stdio},
};

use crate::{LanguageKind, helpers::language_for_path};

#[derive(Debug, Clone)]
pub struct MatchOccurence {
    pub file_path: String,
    pub line_number: u64,
    pub start_byte: u64,
    pub end_byte: u64,
}

#[derive(Debug, Deserialize)]
struct RgEventLine {
    #[serde(rename = "type")]
    event_type: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct MatchData {
    path: TextField,
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
    start: u64,
    end: u64,
}

#[derive(Debug, Clone)]
pub struct GrepConfig {
    pattern: String,
    path: String,
}

impl GrepConfig {
    pub fn new(pattern: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            path: path.into(),
        }
    }
}

pub struct RipGrep {
    config: GrepConfig,
}

impl RipGrep {
    pub fn new(config: GrepConfig) -> Self {
        Self { config }
    }

    pub fn run(&mut self) -> Result<HashMap<LanguageKind, Vec<MatchOccurence>>, io::Error> {
        // NOTE: As POC, we are shelling out to subprocess. In the future, we should use a native Rust implementation
        // https://docs.rs/grep-searcher/0.1.8/grep_searcher/index.html
        let mut child = Command::new("rg")
            .arg(&self.config.pattern)
            .arg(&self.config.path)
            .arg("--json")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                return Err(io::Error::other("failed to capture ripgrep stdout"));
            }
        };

        let reader = BufReader::new(stdout);

        let mut matches: HashMap<LanguageKind, Vec<MatchOccurence>> = HashMap::new();

        for line in reader.lines() {
            match self.parse_grep_event(&line?) {
                Ok(occurrences) => {
                    for occurrence in occurrences {
                        match language_for_path(&occurrence.file_path) {
                            Some(language) => {
                                matches.entry(language).or_default().push(occurrence);
                            }
                            None => {}
                        }
                    }
                }
                Err(e) => {
                    eprintln!("failed to parse rg event: {}", e);
                }
            }
        }

        let status = child.wait()?;
        if !status.success() {
            eprintln!("ripgrep exited with status: {status}");
        }

        Ok(matches)
    }

    fn parse_grep_event(&self, event_json: &str) -> Result<Vec<MatchOccurence>, serde_json::Error> {
        let event: RgEventLine = serde_json::from_str(event_json)?;

        match event.event_type.as_str() {
            "match" => {
                let m: MatchData = serde_json::from_value(event.data)?;

                let path = m.path.text;
                let line_number = m.line_number;
                let absolute_offset = m.absolute_offset;

                let occurrences: Vec<MatchOccurence> = m
                    .submatches
                    .into_iter()
                    .map(|sm| MatchOccurence {
                        file_path: path.clone(),
                        line_number,
                        start_byte: absolute_offset.saturating_add(sm.start),
                        end_byte: absolute_offset.saturating_add(sm.end),
                    })
                    .collect::<Vec<_>>();

                Ok(occurrences)
            }
            _ => Ok(vec![]),
        }
    }
}
