use std::env::args;
use std::process::ExitCode;

use betterlite_cli::cli::run;

#[cfg_attr(test, mutants::skip)]
fn main() -> ExitCode {
    let arguments: Vec<String> = args().skip(1).collect();
    ExitCode::from(run(&arguments))
}
