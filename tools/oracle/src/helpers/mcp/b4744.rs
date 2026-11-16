mod fixtures;
mod input;
mod ops;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Transport(value) => ops::transport(value),
            Input::StdioEnv(value) => Ok(ops::stdio_env(value)),
            Input::Initialize(value) => ops::initialize(value),
            Input::Factory(value) => ops::factory(value),
            Input::Execute(value) => ops::execute(value),
        }
    }
}
