pub(crate) mod input;
mod scenario;
mod server;
mod steps;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Call;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| scenario::invalid(scenario::Failure::Runtime(source)))?;
        runtime.block_on(scenario::run(input))
    }
}
