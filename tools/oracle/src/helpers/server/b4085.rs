mod inputs;
mod run;
mod support;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use inputs::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Pipeline(call) => run::pipeline(call),
        }
    }
}
