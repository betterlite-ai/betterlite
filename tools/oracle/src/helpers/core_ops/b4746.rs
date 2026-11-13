mod input;
mod ops;
mod session;

mod tests;

use crate::helpers::{Helper, HelperResult};

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
