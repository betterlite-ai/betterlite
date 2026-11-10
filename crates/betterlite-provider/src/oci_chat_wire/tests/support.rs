use std::collections::BTreeMap;

use aws_lc_rs::digest::{SHA256, digest};
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_openai::chat::{ChatMessage, Tool};
use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

pub(super) fn body(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn messages(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn tools(text: &str) -> Vec<Tool> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn assert_json<T>(value: &T, expected_text: &str)
where
    T: Serialize,
{
    let actual = serde_json::to_value(value).unwrap();
    let expected = serde_json::to_value(body(expected_text)).unwrap();
    assert_eq!(actual, expected);
}

pub(super) fn base_response() -> ChatResponse {
    ChatResponse {
        id: String::from("chatcmpl-oci"),
        object: String::from("chat.completion"),
        created: 0,
        model: String::from("before"),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: None,
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: None,
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

pub(super) fn expected_call_id(position: usize, name: &str, sorted: &str) -> String {
    let text = format!("{position}|{name}|{sorted}");
    let mut digest = hex::encode(digest(&SHA256, text.as_bytes()));
    digest.truncate(24);
    format!("call_{digest}")
}
