use betterlite_logging::mcp_tool_hook::types::{McpPostCallResponse, McpToolHookCall};
use betterlite_models::decisions::Json as DecisionsJson;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_inspection::hooks::record_applied_guardrail;
use crate::cisco_mcp::blocking::{
    ReplacedResponse, TextContent, build_blocking_response, replace_tool_response,
};
use crate::cisco_mcp::redact::TextUpdate;
use crate::cisco_mcp_tool_hook::request::original_present;

const PROVIDER_NAME: &str = "cisco_ai_defense";

pub(super) fn block(
    call: &mut McpToolHookCall<'_>,
    request_data: &mut Fields,
    detail: &Json,
    guardrail_name: &str,
) -> McpPostCallResponse {
    let response_cost = call
        .response
        .hidden_params
        .response_cost
        .and_then(|cost| Decimal::try_from(cost).ok());
    let replacement =
        build_blocking_response(detail, PROVIDER_NAME, Some(guardrail_name), response_cost);
    call.response.mcp_tool_call_response = blocking_items(&replacement.mcp_tool_call_response);
    if original_present(call.kwargs) {
        replace_original(
            &mut call.original.content,
            &replacement.mcp_tool_call_response,
        );
    }
    record_applied_guardrail(request_data, guardrail_name);
    tracing::warn!(
        "Cisco AI Defense guardrail ({guardrail_name}): MCP response blocked — tool output replaced with synthesized violation message."
    );
    McpPostCallResponse {
        mcp_tool_call_response: call.response.mcp_tool_call_response.clone(),
        hidden_params: call.response.hidden_params,
    }
}

pub(super) fn rewritten_items(update: &TextUpdate) -> Vec<DecisionsJson> {
    let Ok(text) = serde_json::to_string(&update.response) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn replace_original(content: &mut Vec<DecisionsJson>, replacement: &[TextContent]) {
    let text = serde_json::to_string(content).unwrap_or_default();
    let parsed = Json::parse(&text);
    if let Some(ReplacedResponse::Replacement(items)) = replace_tool_response(&parsed, replacement)
    {
        *content = blocking_items(&items);
    }
}

fn blocking_items(replacement: &[TextContent]) -> Vec<DecisionsJson> {
    let mut items = Vec::with_capacity(replacement.len());
    for item in replacement {
        let mut fields = IndexMap::new();
        fields.insert("type".to_owned(), DecisionsJson::Text(item.kind.to_owned()));
        fields.insert("text".to_owned(), DecisionsJson::Text(item.text.clone()));
        items.push(DecisionsJson::Object(fields));
    }
    items
}
