use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

use crate::openai_media_dispatch::failure;

mod harness;
mod model;
mod rows;
mod tests;

use model::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(rows::probe_rows(input))
    }
}
