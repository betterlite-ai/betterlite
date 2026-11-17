use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

mod fixtures;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = fixtures::Case;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| fixtures::failure("RuntimeError", error))?;
        runtime.block_on(fixtures::dispatch_case(input))
    }
}
