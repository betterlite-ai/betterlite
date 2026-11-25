use betterlite_mcp::server_payload::names::validate_tool_name;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    separator: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let probe = format!("litellm{}probe", input.separator);
        Ok(serde_json::json!(validate_tool_name(&probe).is_valid))
    }
}
