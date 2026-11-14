mod bearer;
mod grant;
mod mint;
mod support;
mod tests;

use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Grant { configs: Vec<support::ConfigSpec> },
    Mint(mint::MintInput),
    Bearer(bearer::BearerInput),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Grant { configs } => grant::run(configs),
            Input::Mint(mint) => {
                let executor = support::runtime()?;
                executor.block_on(mint::minted(mint))
            }
            Input::Bearer(bearer) => bearer::run(bearer),
        }
    }
}
