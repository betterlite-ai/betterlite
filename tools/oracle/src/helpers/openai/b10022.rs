use betterlite_openai::decisions::providers::supported_decisions_providers;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::json;

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(_input: &Self::Input) -> HelperResult {
        Ok(json!({"providers": supported_decisions_providers()}))
    }
}
