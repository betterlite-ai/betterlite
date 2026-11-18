mod input;
mod scenario;
mod scenarios;
mod server;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Call;

    fn run(input: &Self::Input) -> HelperResult {
        // the sync cache calls block their thread, so the fake Redis needs a worker of its own
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(scenario::run(input))
    }
}
