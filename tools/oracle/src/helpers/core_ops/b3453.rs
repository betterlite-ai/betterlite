mod fakes;
mod inputs;
mod operations;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use inputs::StoreInput;
use operations::store;

pub(crate) struct Op;

impl Helper for Op {
    type Input = StoreInput;

    fn run(input: &Self::Input) -> HelperResult {
        store(input)
    }
}
