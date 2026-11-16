use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::message_tracing::meta_carrier;
use rmcp::model::MetaObject;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    meta: Option<MetaObject>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let carrier = meta_carrier(input.meta.as_ref());
        Ok(serde_json::to_value(carrier)?)
    }
}
