use std::env::args;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = args().skip(1).collect();
    ExitCode::from(betterlite_cli::cli::run(&arguments))
}
