use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let env = bibers_core::env::Env::from_process();
    ExitCode::from(bibers::run::main_then_exit(&args, &env))
}
