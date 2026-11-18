use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::Input;
use super::output;

pub(crate) fn run(input: &Input) -> HelperResult {
    output::probe(input.case()).map_err(Into::into)
}
