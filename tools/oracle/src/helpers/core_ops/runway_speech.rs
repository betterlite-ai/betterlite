mod fakes;
pub(crate) mod inputs;
mod ops;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::inputs::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
