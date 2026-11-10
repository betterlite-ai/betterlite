mod cache;
mod clock;
mod enumerate;
mod input;
mod listing;

use input::Input;

use crate::helpers::HelperResult;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Enumerate(fields) => Ok(enumerate::run(fields)),
            Input::Cache(fields) => cache::run(fields),
        }
    }
}
