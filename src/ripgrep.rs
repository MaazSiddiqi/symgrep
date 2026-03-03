use serde::Deserialize;
use std::{
    collections::BTreeMap,
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

fn parse_match_occurrences(line: &str) -> Option<Vec<MatchOccurence>> {
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
    let line_number = m.line_number;
    let absolute_offset = m.absolute_offset;

    let occurrences = m
        .submatches
        .into_iter()
        .map(|sm| MatchOccurence {
            file_path: path.clone(),
            line_number,
            start_byte: absolute_offset.saturating_add(sm.start),
            end_byte: absolute_offset.saturating_add(sm.end),
        })
        .collect::<Vec<_>>();

    Some(occurrences)
}

pub fn stream_ripgrep(
    path: &str,
    pattern: &str,
) -> io::Result<BTreeMap<LanguageKind, Vec<MatchOccurence>>> {
    let mut child = Command::new("rg")
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
    let mut matches_by_language: BTreeMap<LanguageKind, Vec<MatchOccurence>> = BTreeMap::new();

    for line in reader.lines() {
        let line = line?;
        if let Some(occurrences) = parse_match_occurrences(&line) {
            for occurrence in occurrences {
                if let Some(language) = language_for_path(&occurrence.file_path) {
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
