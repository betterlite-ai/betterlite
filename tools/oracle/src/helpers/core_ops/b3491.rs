mod config;
mod fixtures;
mod runner;

use betterlite_core::api::interactions::contract::records::Params;

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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        runner::run(input)
    }
}
