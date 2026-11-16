use serde::Deserialize;

use self::script::DispatchCase;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod handler;
mod ops;
mod script;
mod stubs;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Dispatch(DispatchCase),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Dispatch(case) => ops::dispatch_case(case),
        }
    }
}
