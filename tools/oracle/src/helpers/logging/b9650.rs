mod execute;
mod input;
mod logs;
mod scripted;

use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let logs = logs::ErrorLines::default();
        let subscriber = registry().with(logs.clone());
        with_default(subscriber, || {
            runtime.block_on(execute::execute(input, &logs))
        })
    }
}

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "McpToolHookError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
