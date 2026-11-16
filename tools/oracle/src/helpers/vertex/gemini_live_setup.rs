mod ops;
mod tests;
mod warnings;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = ops::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
