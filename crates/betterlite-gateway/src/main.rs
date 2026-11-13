use betterlite_gateway::cli::Cli;
use betterlite_gateway::run;
use clap::Parser as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    run(Cli::parse())
}
