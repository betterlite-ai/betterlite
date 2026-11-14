use std::slice::from_ref;

use betterlite_context::agentic_loop_plan::FollowupMessage;
use betterlite_models::decisions;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::normalized::get_tool_calls_from_response;
use betterlite_prompts::response_tool_calls::projections::ToolCallResponse;
use betterlite_prompts::response_tool_calls::tools::has_tool_with_name;
use serde::{Deserialize, Serialize};

use crate::headroom_settings::HEADROOM_RETRIEVE_TOOL_NAME;

pub(crate) const TOOL_CALLS_KEY: &str = "tool_calls";
pub(crate) const HASH_FIELD: &str = "hash";
pub(crate) const QUERY_FIELD: &str = "query";
const FUNCTION_KIND: &str = "function";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RetrieveCall {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default = "default_kind", rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) arguments: Object,
}

fn default_kind() -> String {
    FUNCTION_KIND.to_owned()
}

pub(crate) fn retrieve_calls(response: &ToolCallResponse) -> Vec<RetrieveCall> {
    get_tool_calls_from_response(response, false)
        .into_iter()
        .filter(|call| call.name.as_deref() == Some(HEADROOM_RETRIEVE_TOOL_NAME))
        .map(|call| RetrieveCall {
            id: call.id,
            kind: default_kind(),
            name: call.name,
            arguments: call.arguments,
        })
        .collect()
}

pub(crate) fn hash_argument(call: &RetrieveCall) -> String {
    match call.arguments.get(HASH_FIELD) {
        Some(Json::Text(hash)) => hash.to_lowercase(),
        _ => String::new(),
    }
}

pub(crate) fn query_argument(call: &RetrieveCall) -> Option<&str> {
    call.arguments
        .get(QUERY_FIELD)
        .and_then(Json::as_str)
        .filter(|query| !query.is_empty())
}

pub(crate) fn request_holds_retrieve_tool(tools: Option<&[FollowupMessage]>) -> bool {
    let Some(entries) = tools else {
        return false;
    };
    entries.iter().any(|tool| {
        serde_json::from_str::<Json>(tool.0.get())
            .is_ok_and(|parsed| has_tool_with_name(from_ref(&parsed), HEADROOM_RETRIEVE_TOOL_NAME))
    })
}

pub(crate) fn value_holds_retrieve_tool(tools: Option<&decisions::Json>) -> bool {
    let Some(value) = tools else {
        return false;
    };
    let Ok(encoded) = serde_json::to_string(value) else {
        return false;
    };
    let Ok(Json::List(parsed)) = serde_json::from_str::<Json>(&encoded) else {
        return false;
    };
    has_tool_with_name(&parsed, HEADROOM_RETRIEVE_TOOL_NAME)
}
