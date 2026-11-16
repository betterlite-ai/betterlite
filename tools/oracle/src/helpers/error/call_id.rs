use betterlite_error::envelope;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub enabled: bool,
    pub call_id: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let call_id = envelope::error_body_call_id(input.enabled, input.call_id.as_deref());
        Ok(serde_json::json!({"result": call_id}))
    }
}
