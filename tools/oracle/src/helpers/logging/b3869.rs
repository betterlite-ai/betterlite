mod input;
mod record;
mod tests;

use input::Input;
use record::record_call;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        record_call(input)
    }
}
