mod cache;
mod clock;
mod enumerate;
mod input;
mod listing;

use input::Input;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Enumerate(fields) => Ok(enumerate::run(fields)),
            Input::Cache(fields) => cache::run(fields),
        }
    }
}
