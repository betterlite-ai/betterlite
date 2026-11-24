mod events;
mod fixture;
mod script;
mod view;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

pub(crate) struct Op;

impl Helper for Op {
    type Input = script::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let output = runtime.block_on(script::drive(input))?;
        serde_json::to_value(output).or_value_error()
    }
}
