use crate::helpers::{Helper, HelperResult};

mod steps;

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
