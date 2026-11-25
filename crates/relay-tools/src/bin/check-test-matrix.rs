//! Recounts `spec/tests.md` (or the file named by the first argument).

use std::process::ExitCode;

fn main() -> ExitCode {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "spec/tests.md".into());
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("check-test-matrix: cannot read {path}: {err}");
            return ExitCode::FAILURE;
        }
    };
    match relay_tools::matrix::recount(&text) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            println!("{}", problems.join("\n"));
            ExitCode::FAILURE
        }
    }
}
