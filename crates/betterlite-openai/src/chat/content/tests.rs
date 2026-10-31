#![cfg(test)]

use super::*;
use crate::chat::response::{AssistantMessage, ChatChoice, ChatUsage, MessageProviderFields};
use crate::chat::{FunctionCall, ToolCall};
use std::collections::BTreeMap;

fn response(choices: Vec<ChatChoice>) -> ChatResponse {
    ChatResponse {
        id: "resp".to_owned(),
        object: "chat.completion".to_owned(),
        created: 0,
        model: "gpt".to_owned(),
        choices,
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn choice(message: AssistantMessage) -> ChatChoice {
    ChatChoice {
        index: 0,
        message,
        finish_reason: None,
        logprobs: None,
        provider_specific_fields: BTreeMap::new(),
    }
}

fn message(content: Option<&str>, tool_calls: Option<Vec<ToolCall>>) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".to_owned(),
        content: content.map(str::to_owned),
        tool_calls,
        annotations: None,
        audio: None,
        refusal: None,
        provider_specific_fields: MessageProviderFields::default(),
    }
}

#[test]
fn message_content_behavior_tests_behavior() {
    let tool_call = ToolCall {
        id: "call-1".to_owned(),
        kind: "function".to_owned(),
        function: FunctionCall {
            name: "lookup".to_owned(),
            arguments: "{}".to_owned(),
        },
    };
    let input = response(vec![
        choice(message(Some("hello "), Some(vec![tool_call.clone()]))),
        choice(message(None, None)),
    ]);
    let expected_tool_call = serde_json::to_string(&tool_call).expect("tool call serializes");
    assert_eq!(
        from_model_response(&input),
        format!("hello {expected_tool_call}")
    );
}
