mod fixtures;
mod input;
mod logs;
mod ops;

use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use crate::helpers::{Helper, HelperResult};

use logs::Logs;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logs = Logs::default();
        let _guard = set_default(registry().with(logs.clone()));
        let runtime = Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(ops::failure)?;
        runtime.block_on(ops::run(input, &logs))
    }
}
