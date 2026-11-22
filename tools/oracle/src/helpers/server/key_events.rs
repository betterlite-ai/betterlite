mod fixtures;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

use self::fixtures::{Input, failure};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(fixtures::answer(input))
    }
}
