#![expect(
    clippy::disallowed_types,
    reason = "The oracle compares arbitrary provider JSON, so captured payloads require serde_json::Value."
)]

pub mod case;
mod cli;
mod compare;
mod gateway;
pub mod http;
pub mod mock;
pub mod process;
pub mod runner;

mod tests;

use std::process::ExitCode;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OracleError {
    #[error(transparent)]
    Cli(#[from] clap::Error),
    #[error(transparent)]
    Case(#[from] case::CaseError),
    #[error(transparent)]
    Harness(#[from] runner::HarnessError),
}

pub fn run() -> Result<ExitCode, OracleError> {
    let options = <cli::Options as clap::Parser>::try_parse()?;
    let cases = case::load_cases(&options.paths)?;
    Ok(runner::run_cases(&options, cases)?)
}
