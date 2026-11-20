mod input;
mod logs;
mod ops;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use logs::Logs;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logs = Logs::default();
        let _guard = set_default(registry().with(logs.clone()));
        let runtime = Builder::new_current_thread().enable_time().build()?;
        runtime.block_on(ops::answer(input, &logs))
    }
}
