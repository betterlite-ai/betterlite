mod answer;
mod fixtures;
mod input;
mod logs;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
pub(crate) struct Op;
impl Helper for Op {
    type Input = input::Input;
    fn run(input: &Self::Input) -> HelperResult {
        answer::answer(input).or_value_error()
    }
}
