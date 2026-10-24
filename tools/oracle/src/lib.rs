#![expect(
    clippy::disallowed_types,
    reason = "The oracle compares arbitrary provider JSON, so captured payloads require serde_json::Value."
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

pub fn run() -> Result<ExitCode, String> {
    let options = <cli::Options as clap::Parser>::try_parse().map_err(|error| error.to_string())?;
    let cases = case::load_cases(&options.paths).map_err(|error| error.to_string())?;
    runner::run_cases(&options, cases).map_err(|error| error.to_string())
}
