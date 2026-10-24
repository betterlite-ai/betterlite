use clap::Parser;
use std::process::ExitCode;

#[derive(Parser)]
#[command(version)]
struct Cli;

fn main() -> ExitCode {
    let _cli = Cli::parse();
    ExitCode::SUCCESS
}
