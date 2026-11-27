mod families;
mod fixture;
mod input;
mod ops;
mod tests;

pub(crate) struct Op;

impl betterlite_oracle_helper_api::helper::Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> betterlite_oracle_helper_api::helper::HelperResult {
        ops::run(input)
    }
}
