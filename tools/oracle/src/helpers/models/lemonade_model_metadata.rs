mod input;
mod scenario;
mod tests;

use crate::helpers::HelperResult;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        scenario::run(input)
    }
}
