#![expect(
    clippy::restriction,
    clippy::complexity,
    clippy::pedantic,
    clippy::nursery,
    clippy::style,
    reason = "The oracle is a bounded CLI harness that records wire JSON and owns subprocess/thread lifetimes."
)]

mod case;
mod cli;
mod compare;
mod gateway;
mod http;
mod mock;
mod process;
mod runner;

mod tests;

use std::process::ExitCode;

pub fn exit_status(code: ExitCode) -> i32 {
    let status = if code == ExitCode::SUCCESS { 0 } else { 1 };
    status
}

#[cfg(not(test))]
pub fn run() -> Result<ExitCode, String> {
    let options = <cli::Options as clap::Parser>::try_parse().map_err(|error| error.to_string())?;
    let cases = case::load_cases(&options.paths).map_err(|error| error.to_string())?;
    runner::run_cases(&options, cases).map_err(|error| error.to_string())
}
