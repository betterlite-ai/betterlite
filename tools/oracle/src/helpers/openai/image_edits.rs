mod input;
mod run;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum Input {
    Transform(Vec<input::Probe>),
    Reply(Vec<input::Probe>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run::values(input)
    }
}
