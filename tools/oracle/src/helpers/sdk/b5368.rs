use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::input::Input;

mod fixture;
mod input;
mod ports;
mod target;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        fixture::run(input)
    }
}
