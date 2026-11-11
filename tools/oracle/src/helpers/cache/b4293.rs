mod input;
mod scenario;
mod server;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Call;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(scenario::run(input))
    }
}
