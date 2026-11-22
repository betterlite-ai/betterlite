mod fixtures;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = fixtures::case::Case;

    fn run(input: &Self::Input) -> HelperResult {
        let observations = fixtures::execute::run(input)
            .map_err(|error| fixtures::case::failure("ValueError", error))?;
        serde_json::to_value(observations)
            .map_err(|error| fixtures::case::failure("ValueError", error))
    }
}
