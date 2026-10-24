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

#[cfg(test)]
mod tests;

use std::process::ExitCode;

pub fn run() -> Result<ExitCode, String> {
    let options = cli::Options::parse().map_err(|error| error.to_string())?;
    let cases = case::load_cases(&options.paths).map_err(|error| error.to_string())?;
    runner::run_cases(&options, cases).map_err(|error| error.to_string())
}
