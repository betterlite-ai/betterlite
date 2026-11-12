use crate::helpers::{Helper, HelperResult};

mod input;
mod ops;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
