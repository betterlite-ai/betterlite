mod answer;
mod support;
mod types;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::types::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| types::failure("RuntimeError", error))?;
        runtime.block_on(answer::answer(input))
    }
}
