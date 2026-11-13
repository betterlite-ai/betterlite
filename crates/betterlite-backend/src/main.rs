use betterlite_backend::cli::Cli;
use betterlite_backend::run;
use clap::Parser as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    run(Cli::parse())
}
