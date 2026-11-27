mod fixtures;
mod input;
mod run;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run::run(input)
    }
}
