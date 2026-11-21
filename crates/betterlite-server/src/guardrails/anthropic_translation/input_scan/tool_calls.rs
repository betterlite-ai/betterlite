use betterlite_anthropic::response_content::project_response;
use betterlite_anthropic::response_content::wire::{AnthropicCompletionResponse, JsonFields};
use betterlite_models::decisions::Json as ModelJson;
use serde_json::{Map, Value};

use crate::guardrails::anthropic_translation::scan_targets::is_client_tool_use;

use super::serde_convert;

// LiteLLM's convert_tool_use_to_openai_format: the chat tool call a guardrail sees for a client tool_use block
pub(crate) fn client_tool_use_to_openai(block: &Map<String, Value>, index: usize) -> Option<Value> {
    if !is_client_tool_use(block) {
        return None;
    }
    output_tool_use_to_openai(block, index)
}

// The output side accepts any tool_use block, so a missing id, name or input turns into None here
pub(crate) fn output_tool_use_to_openai(block: &Map<String, Value>, index: usize) -> Option<Value> {
    let content: ModelJson = serde_convert(block, "tool_use block").ok()?;
    let response = AnthropicCompletionResponse {
        content: vec![content],
        fields: JsonFields::new(),
    };
    let reply = project_response(&response, None).ok()?;
    let mut call = reply.tool_calls.into_iter().next()?;
    call.index = index;
    serde_json::to_value(call).ok()
}
