pub(crate) mod input;
mod scenario;
mod server;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Call;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| scenario::invalid(source.to_string()))?;
        runtime.block_on(scenario::run(input))
    }
}
