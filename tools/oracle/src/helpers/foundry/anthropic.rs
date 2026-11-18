use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod answers;
mod fixture;
mod input;
mod tests;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answers::completion(input)
    }
}
