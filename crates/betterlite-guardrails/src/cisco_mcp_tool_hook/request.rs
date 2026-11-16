use betterlite_core::passthrough_guardrail::settings::RequestData;
use betterlite_core::passthrough_guardrail::wire::parse_request_data;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::mcp_tool_hook::types::McpPostCallResponse;
use betterlite_models::decisions::Json as DecisionsJson;
use indexmap::IndexMap;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::blocking::extract_tool_call_response;
use crate::cisco_verdict_enforcement::support::insert_serialized;

const COPIED_KEYS: [&str; 14] = [
    "name",
    "litellm_call_id",
    "id",
    "user",
    "mcp_tool_name",
    "tool_name",
    "mcp_arguments",
    "arguments",
    "mcp_server_name",
    "server_name",
    "metadata",
    "litellm_metadata",
    "mcp_tool_call_metadata",
    "guardrails",
];

pub(super) fn request_fields(kwargs: &IndexMap<String, DecisionsJson>) -> Fields {
    let mut data = Fields::new();
    for key in COPIED_KEYS {
        if let Some(value) = kwargs.get(key)
            && !matches!(value, DecisionsJson::Null)
        {
            insert_serialized(&mut data, key, value);
        }
    }
    data
}

pub(super) fn guardrail_data(
    request_data: &Fields,
    callback: &str,
) -> Result<RequestData, LoggerError> {
    let text = serde_json::to_string(request_data).map_err(|error| LoggerError::Delivery {
        callback: callback.to_owned(),
        message: error.to_string(),
    })?;
    parse_request_data(&text).map_err(|error| LoggerError::Delivery {
        callback: callback.to_owned(),
        message: error.to_string(),
    })
}

pub(super) fn scanned_response(response: &McpPostCallResponse) -> Option<Json> {
    let mut wrapper = Fields::new();
    insert_serialized(
        &mut wrapper,
        "mcp_tool_call_response",
        &response.mcp_tool_call_response,
    );
    let extracted = extract_tool_call_response(&Json::Object(wrapper));
    if matches!(extracted, Json::Null) {
        return None;
    }
    Some(extracted)
}

pub(super) fn original_present(kwargs: &IndexMap<String, DecisionsJson>) -> bool {
    kwargs
        .get("original_response")
        .is_some_and(|value| !matches!(value, DecisionsJson::Null))
}
