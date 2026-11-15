mod fakes;
mod input;
mod scenario;

use input::Input;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        scenario::run(input)
    }
}
