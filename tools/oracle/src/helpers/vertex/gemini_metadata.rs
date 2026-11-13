use crate::helpers::{Helper, HelperResult};

mod ops;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = ops::Input;
    fn run(input: &Self::Input) -> HelperResult {
        Ok(ops::run(input))
    }
}
