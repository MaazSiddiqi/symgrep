use serde::Deserialize;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

#[derive(Debug, Clone)]
pub struct MatchOccurence {
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
    rg_binary: String,
}

impl GrepConfig {
    pub fn new(pattern: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            path: path.into(),
            rg_binary: "rg".to_string(),
        }
    }

    #[cfg(test)]
    fn with_binary(mut self, rg_binary: impl Into<String>) -> Self {
        self.rg_binary = rg_binary.into();
        self
    }
}

pub struct RipGrep {
    config: GrepConfig,
}

impl RipGrep {
    pub fn new(config: GrepConfig) -> Self {
        Self { config }
    }

    pub fn run(&mut self) -> Result<HashMap<String, Vec<MatchOccurence>>, String> {
        // NOTE: As POC, we are shelling out to subprocess. In the future, we should use a native Rust implementation
        // https://docs.rs/grep-searcher/0.1.8/grep_searcher/index.html
        let mut child = Command::new(&self.config.rg_binary)
            .arg(&self.config.pattern)
            .arg(&self.config.path)
            .arg("--json")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| err.to_string())?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("failed to capture ripgrep stdout"))?;

        let reader = BufReader::new(stdout);

        let mut matches: HashMap<String, Vec<MatchOccurence>> = HashMap::new();

        for line in reader.lines() {
            let line = line.map_err(|err| format!("failed reading ripgrep stdout: {err}"))?;

            if let Some((file_path, occurrences)) = self.parse_grep_event(&line)? {
                matches.entry(file_path).or_default().extend(occurrences);
            }
        }

        let status = child
            .wait()
            .map_err(|err| format!("failed waiting for ripgrep: {err}"))?;
        if !status.success() {
            return Err(format!("ripgrep exited with non-success status: {status}"));
        }

        Ok(matches)
    }

    fn parse_grep_event(
        &self,
        event_json: &str,
    ) -> Result<Option<(String, Vec<MatchOccurence>)>, String> {
        let event: RgEventLine = serde_json::from_str(event_json).map_err(|err| err.to_string())?;

        match event.event_type.as_str() {
            "match" => {
                let m: MatchData =
                    serde_json::from_value(event.data).map_err(|err| err.to_string())?;

                let path = m.path.text;
                let line_number = m.line_number;
                let absolute_offset = m.absolute_offset;

                let occurrences: Vec<MatchOccurence> = m
                    .submatches
                    .into_iter()
                    .map(|sm| MatchOccurence {
                        line_number,
                        start_byte: absolute_offset.saturating_add(sm.start),
                        end_byte: absolute_offset.saturating_add(sm.end),
                    })
                    .collect::<Vec<_>>();

                Ok(Some((path, occurrences)))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn rg_available() -> bool {
        Command::new("rg").arg("--version").output().is_ok()
    }

    struct TestTempDir {
        path: PathBuf,
    }

    impl TestTempDir {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be monotonic")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "project-symgrep-ripgrep-tests-{}-{}-{}",
                name,
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("should create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_file(path: &Path, body: &str) {
        fs::write(path, body).expect("should write test file");
    }

    #[test]
    fn run_happy_case_returns_expected_matches() {
        if !rg_available() {
            return;
        }

        let dir = TestTempDir::new("happy");
        let file_path = dir.path().join("sample.rs");
        write_file(
            &file_path,
            "fn main() {\n    let x = foo();\n    foo();\n}\n",
        );

        let mut ripgrep = RipGrep::new(GrepConfig::new(
            "foo",
            dir.path().to_string_lossy().to_string(),
        ));

        let result = ripgrep.run().expect("ripgrep run should succeed");
        let entry = result
            .iter()
            .find(|(path, _)| path.ends_with("sample.rs"))
            .expect("expected sample.rs entry");

        let occurrences = entry.1;
        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0].line_number, 2);
        assert_eq!(occurrences[1].line_number, 3);
    }

    #[test]
    fn run_returns_error_when_spawn_fails() {
        let dir = TestTempDir::new("spawn-fail");
        let mut ripgrep = RipGrep::new(
            GrepConfig::new("foo", dir.path().to_string_lossy().to_string())
                .with_binary("rg-does-not-exist"),
        );

        let err = ripgrep.run().expect_err("spawn should fail");
        assert!(
            err.contains("No such file")
                || err.contains("os error")
                || err.contains("cannot find")
                || err.contains("not found")
        );
    }

    #[test]
    fn run_returns_error_on_non_success_status() {
        if !rg_available() {
            return;
        }

        let dir = TestTempDir::new("non-success");
        let file_path = dir.path().join("sample.rs");
        write_file(&file_path, "fn main() {}\n");

        let mut ripgrep = RipGrep::new(GrepConfig::new(
            "definitely_no_match_token",
            dir.path().to_string_lossy().to_string(),
        ));

        let err = ripgrep
            .run()
            .expect_err("no matches should produce non-success");
        assert!(err.contains("non-success status"));
    }

    #[test]
    fn run_empty_file_returns_non_success_error() {
        if !rg_available() {
            return;
        }

        let dir = TestTempDir::new("empty-file");
        let file_path = dir.path().join("empty.rs");
        write_file(&file_path, "");

        let mut ripgrep = RipGrep::new(GrepConfig::new(
            "foo",
            dir.path().to_string_lossy().to_string(),
        ));

        let err = ripgrep
            .run()
            .expect_err("searching empty file with pattern should be non-success");
        assert!(err.contains("non-success status"));
    }

    #[test]
    fn parse_grep_event_handles_invalid_shape() {
        let ripgrep = RipGrep::new(GrepConfig::new("foo", "."));
        let bad = r#"{"type":"match","data":{"line_number":"not-number"}}"#;
        let err = ripgrep
            .parse_grep_event(bad)
            .expect_err("invalid event shape should fail");
        assert!(!err.is_empty());
    }

    #[test]
    fn parse_grep_event_handles_non_match_event() {
        let ripgrep = RipGrep::new(GrepConfig::new("foo", "."));
        let begin = r#"{"type":"begin","data":{"path":{"text":"src/main.rs"}}}"#;
        let parsed = ripgrep
            .parse_grep_event(begin)
            .expect("non-match event should be ignored");
        assert!(parsed.is_none());
    }

    #[test]
    fn parse_grep_event_parses_match_event_and_occurrences() {
        let ripgrep = RipGrep::new(GrepConfig::new("foo", "."));
        let event = r#"{
            "type":"match",
            "data":{
                "path":{"text":"src/main.rs"},
                "line_number":7,
                "absolute_offset":100,
                "submatches":[
                    {"start":2,"end":5},
                    {"start":10,"end":13}
                ]
            }
        }"#;

        let parsed = ripgrep
            .parse_grep_event(event)
            .expect("match event should parse")
            .expect("should return Some for match event");

        assert_eq!(parsed.0, "src/main.rs");
        assert_eq!(parsed.1.len(), 2);
        assert_eq!(parsed.1[0].line_number, 7);
        assert_eq!(parsed.1[0].start_byte, 102);
        assert_eq!(parsed.1[0].end_byte, 105);
        assert_eq!(parsed.1[1].start_byte, 110);
        assert_eq!(parsed.1[1].end_byte, 113);
    }
}
