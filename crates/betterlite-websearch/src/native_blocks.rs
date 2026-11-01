use std::collections::BTreeMap;

use uuid::Uuid;

use crate::call_transformation::TransformationError;
use crate::call_transformation::native::{self, ResultBlock};
use crate::call_transformation::wire::{Call, Input};
use crate::records::{
    AnthropicSearchQuery, AnthropicServerToolUseBlock, SearchOutcome, ServerToolUseType,
    WebSearchName,
};

#[expect(
    clippy::disallowed_types,
    reason = "provider-defined response and citation-block JSON; only the `content` field is ever read or written"
)]
pub type ResponseJson = serde_json::Value;

pub const NATIVE_BLOCKS_METADATA_KEY: &str = "websearch_native_blocks";

#[must_use]
pub fn tool_call_query(call: &Call) -> String {
    let Input::Object(fields) = &call.input else {
        return String::new();
    };
    fields
        .get("query")
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        .unwrap_or_default()
}

#[must_use]
pub fn native_result_pair(
    query: &str,
    outcome: &SearchOutcome,
) -> (AnthropicServerToolUseBlock, ResultBlock) {
    let tool_use_id = format!("srvtoolu_{}", Uuid::new_v4().simple());
    let server_tool_use = AnthropicServerToolUseBlock {
        block_type: ServerToolUseType::ServerToolUse,
        id: tool_use_id.clone(),
        name: WebSearchName::WebSearch,
        input: AnthropicSearchQuery {
            query: query.to_owned(),
        },
    };
    (
        server_tool_use,
        native::outcome_block(&tool_use_id, outcome),
    )
}

pub fn build_native_result_blocks(
    tool_calls: &[Call],
    search_outcomes: &[SearchOutcome],
) -> Result<Vec<(AnthropicServerToolUseBlock, ResultBlock)>, TransformationError> {
    if tool_calls.len() != search_outcomes.len() {
        return Err(TransformationError::MismatchedOutcomes {
            calls: tool_calls.len(),
            outcomes: search_outcomes.len(),
        });
    }
    Ok(tool_calls
        .iter()
        .zip(search_outcomes)
        .map(|(call, outcome)| native_result_pair(&tool_call_query(call), outcome))
        .collect())
}

// A non-object response is the Rust analog of Python's object-form response refusing the attribute write.
#[must_use]
pub fn inject_native_blocks(
    response: ResponseJson,
    native_blocks: &[ResponseJson],
) -> ResponseJson {
    if native_blocks.is_empty() {
        return response;
    }
    let ResponseJson::Object(mut fields) = response else {
        return response;
    };
    let mut content = native_blocks.to_vec();
    if let Some(ResponseJson::Array(existing)) = fields.get("content") {
        content.extend(existing.iter().cloned());
    }
    fields.insert("content".to_owned(), ResponseJson::Array(content));
    ResponseJson::Object(fields)
}

#[must_use]
pub fn post_agentic_response_hook(
    response: ResponseJson,
    metadata: &BTreeMap<String, ResponseJson>,
) -> ResponseJson {
    let Some(ResponseJson::Array(blocks)) = metadata.get(NATIVE_BLOCKS_METADATA_KEY) else {
        return response;
    };
    if blocks.is_empty() {
        return response;
    }
    inject_native_blocks(response, blocks)
}
