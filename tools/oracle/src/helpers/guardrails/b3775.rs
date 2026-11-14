mod headroom;
mod hiddenlayer;
mod scripted;

use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Input {
    ShouldRun(headroom::ShouldRun),
    Plan(headroom::Plan),
    Hiddenlayer(hiddenlayer::Input),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ShouldRun(input) => headroom::should_run(input),
            Input::Plan(input) => headroom::plan(input),
            Input::Hiddenlayer(input) => hiddenlayer::run(input),
        }
    }
}
