use std::collections::BTreeMap;

use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_prompts::arguments::json::Json;

use crate::oci_chat_wire::cohere::tool_calls::adapt_cohere_tool_calls;
use crate::oci_chat_wire::ids::normalize_oci_finish_reason;
use crate::oci_chat_wire::parse::parse_response;
use crate::oci_chat_wire::types::cohere::CohereChatResult;
use crate::oci_common::OciError;

pub fn handle_cohere_response(
    body: &Json,
    model: &str,
    created: u64,
    status: u16,
    response: &mut ChatResponse,
) -> Result<(), OciError> {
    let result: CohereChatResult = parse_response(body, "CohereChatResult", status)?;
    model.clone_into(&mut response.model);
    response.created = created;
    let text = result.chat_response.text;
    let finish_reason = normalize_oci_finish_reason(result.chat_response.finish_reason.as_deref());
    let tool_calls = result
        .chat_response
        .tool_calls
        .as_ref()
        .filter(|calls| !calls.is_empty())
        .map(|calls| adapt_cohere_tool_calls(calls));
    response.choices = vec![ChatChoice {
        index: 0,
        message: AssistantMessage {
            role: String::from("assistant"),
            content: (!text.is_empty()).then_some(text),
            tool_calls,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields::default(),
        },
        finish_reason,
        logprobs: None,
        provider_specific_fields: BTreeMap::new(),
    }];
    response.usage = match result.chat_response.usage {
        Some(usage) => ChatUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        None => ChatUsage::default(),
    };
    Ok(())
}
