mod analyzer;
mod engine;
mod helpers;
mod output;
mod ripgrep;

use std::env;

use crate::engine::Engine;

const COLOR_RESET: &str = "\x1b[0m";
const COLOR_PATH_DIM: &str = "\x1b[90m";
const COLOR_LINE_NUM: &str = "\x1b[36m";
const COLOR_META_MILD: &str = "\x1b[2;37m";
const COLOR_HIGHLIGHT: &str = "\x1b[1;33m";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LanguageKind {
    Rust,
    TypeScript,
    Tsx,
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
    let (pattern, path) = match parse_cli_args() {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return;
        }
    };

    Engine::run(&path, &pattern);
}
