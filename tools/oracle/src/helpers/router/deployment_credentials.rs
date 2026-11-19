mod error;
mod input;
mod snapshot;
mod warnings;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let captured = warnings::Warnings::default();
        let subscriber = tracing_subscriber::registry().with(captured.clone());
        let answer = with_default(subscriber, || {
            snapshot::answer(input, &captured).or_value_error()
        })?;
        serde_json::to_value(answer).or_value_error()
    }
}
