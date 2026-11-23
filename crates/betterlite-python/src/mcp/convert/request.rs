use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::mcp_client_resources::types::GetPromptRequestParams;
use betterlite_mcp::tools::pagination::ToolFormat;
use betterlite_mcp::tools::tool_calls::CallRequest;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

use super::{NativeMcpErrorPayload, seconds_duration};

// The request texts each native method takes, with LiteLLM's argument names (client.py#L873-L958)
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct CallToolWire {
    pub(crate) name: String,
    pub(crate) arguments: Option<Object>,
    pub(crate) allow_input_required: bool,
    pub(crate) raise_on_error: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct GetPromptWire {
    pub(crate) name: String,
    pub(crate) arguments: Option<BTreeMap<String, String>>,
}

pub(crate) fn call_request(wire: &CallToolWire) -> CallRequest {
    CallRequest {
        name: wire.name.clone(),
        arguments: wire.arguments.clone(),
    }
}

pub(crate) fn prompt_params(wire: GetPromptWire) -> GetPromptRequestParams {
    GetPromptRequestParams {
        name: wire.name,
        arguments: wire.arguments,
    }
}

pub(crate) fn tool_format(name: &str) -> Result<ToolFormat, NativeMcpErrorPayload> {
    match name {
        "mcp" => Ok(ToolFormat::Mcp),
        "openai" => Ok(ToolFormat::Openai),
        other => Err(NativeMcpErrorPayload::value(format!(
            "format must be \"mcp\" or \"openai\", not {other:?}"
        ))),
    }
}

pub(crate) fn deadline(seconds: Option<f64>) -> Result<Option<Duration>, NativeMcpErrorPayload> {
    match seconds {
        Some(value) => seconds_duration("listing_deadline", value).map(Some),
        None => Ok(None),
    }
}
