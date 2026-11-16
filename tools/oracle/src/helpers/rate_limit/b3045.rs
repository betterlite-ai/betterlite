use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

mod answers;
mod cases;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    // boxed so every variant carries a pointer, well within the 3x variant_size_differences enforces
    IoTokenOps(Box<cases::IoTokenCase>),
    ScopeTargets(Box<cases::ScopeTargetsCase>),
    SuccessOps(Box<cases::SuccessCase>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::IoTokenOps(case) => answers::io_token_answer(case),
            Input::ScopeTargets(case) => Ok(answers::scope_targets_answer(case)),
            Input::SuccessOps(case) => answers::success_answer(case),
        }
    }
}
