use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[path = "anthropic/tests/mod.rs"]
mod adapter;

use adapter::input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        adapter::answers::completion(input)
    }
}
