mod headroom;
mod hiddenlayer;
mod scripted;

use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

// boxed so the variants keep one size; variant_size_differences and large_enum_variant deny the spread
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Input {
    ShouldRun(Box<headroom::ShouldRun>),
    Plan(Box<headroom::Plan>),
    Hiddenlayer(Box<hiddenlayer::Input>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ShouldRun(item) => headroom::should_run(item),
            Input::Plan(item) => headroom::plan(item),
            Input::Hiddenlayer(item) => hiddenlayer::run(item),
        }
    }
}
