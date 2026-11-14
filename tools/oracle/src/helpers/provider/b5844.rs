mod input;
mod ops;
mod project;
mod support;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        ops::output(input)
    }
}
