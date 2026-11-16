mod config;
mod fixtures;
mod runner;

use betterlite_core::api::interactions::contract::records::Params;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Async,
    Sync,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mode: Mode,
    lines: Vec<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    litellm_params: Option<Params>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        runner::run(input)
    }
}
