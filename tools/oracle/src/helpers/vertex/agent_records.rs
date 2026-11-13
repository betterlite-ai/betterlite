mod ops;
mod tests;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = ops::Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        ops::run(input)
    }
}
