mod answers;
mod input;
mod recorder;
mod tests;

use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(answers::failure)?;
        let answers = input
            .calls
            .iter()
            .map(|call| answers::answer(&runtime, call))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": answers }))
    }
}
