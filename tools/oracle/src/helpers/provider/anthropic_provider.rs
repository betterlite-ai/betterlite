mod tests;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::anthropic_dispatch::fixtures::{Case, execute};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Case;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let observations = runtime.block_on(execute(input)).or_value_error()?;
        serde_json::to_value(observations).or_value_error()
    }
}
