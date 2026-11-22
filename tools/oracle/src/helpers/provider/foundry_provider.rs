mod tests;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::foundry_dispatch::fixtures::oracle::{Case, execute};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Case;

    fn run(input: &Self::Input) -> HelperResult {
        serde_json::to_value(execute(input)).or_value_error()
    }
}
