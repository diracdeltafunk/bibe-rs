use std::process::ExitCode;

/// biber's exit code for errors (`EXIT_ERROR`).
const EXIT_ERROR: u8 = 2;

fn main() -> ExitCode {
    eprintln!(
        "bibers {}: not yet implemented (see docs/PLAN.md)",
        env!("CARGO_PKG_VERSION")
    );
    ExitCode::from(EXIT_ERROR)
}
