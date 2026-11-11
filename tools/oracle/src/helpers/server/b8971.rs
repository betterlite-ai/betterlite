use betterlite_server::mcp::message_tracing::meta_carrier;
use rmcp::model::MetaObject;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    meta: Option<MetaObject>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let carrier = meta_carrier(input.meta.as_ref());
        Ok(serde_json::to_value(carrier)?)
    }
}
