mod input;
mod ops;
mod signing;
mod tests;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
