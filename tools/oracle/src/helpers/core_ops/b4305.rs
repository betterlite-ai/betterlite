mod fixtures;
mod scenario;
use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

use fixtures::Input;
use scenario::scenarios;
pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        let reports = runtime.block_on(scenarios(input))?;
        Ok(serde_json::Value::Array(reports))
    }
}
