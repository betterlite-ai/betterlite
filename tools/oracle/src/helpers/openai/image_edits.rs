mod input;
mod run;
mod tests;

use crate::helpers::HelperResult;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum Input {
    Transform(Vec<input::Probe>),
    Reply(Vec<input::Probe>),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run::values(input)
    }
}
