#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{
    build_guard_input_for_request, build_guard_input_for_response, extract_transformed_texts,
    messages_since_last_assistant,
};
use crate::crowdstrike_aidr::models::{
    ContentPart, GuardInput, Message, MessageContent, TextContentPart, TextPartType,
};
use crate::generic_guardrail_contracts::{GuardrailMessage, GuardrailToolParam};
use crate::guardrail_base::inputs::GuardrailApiInputs;

fn message(role: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        )]),
    }
}

fn inputs(messages: Vec<GuardrailMessage>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        structured_messages: Some(messages),
        ..empty_inputs()
    }
}

fn empty_inputs() -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn roles(messages: &[GuardrailMessage]) -> Vec<String> {
    messages
        .iter()
        .map(|message| message.role.clone())
        .collect()
}

#[test]
fn _messages_since_last_assistant_no_messages() {
    let filtered = messages_since_last_assistant(&[]);
    assert!(filtered.messages.is_empty());
    assert!(filtered.indices.is_empty());
}

#[test]
fn _messages_since_last_assistant_last_is_assistant() {
    let messages = [
        message("system", "s"),
        message("user", "u"),
        message("assistant", "a"),
    ];
    let filtered = messages_since_last_assistant(&messages);
    assert_eq!(filtered.indices, vec![0, 2]);
    assert_eq!(roles(&filtered.messages), vec!["system", "assistant"]);
}

#[test]
fn _messages_since_last_assistant_last_is_not_assistant() {
    let messages = [
        message("system", "s0"),
        message("user", "u1"),
        message("assistant", "a2"),
        message("system", "s3"),
        message("user", "u4"),
    ];
    let filtered = messages_since_last_assistant(&messages);
    assert_eq!(filtered.indices, vec![0, 3, 4]);
    assert_eq!(roles(&filtered.messages), vec!["system", "system", "user"]);
}

#[test]
fn _messages_since_last_assistant_without_an_assistant_keeps_every_message() {
    let messages = [message("user", "u0"), message("system", "s1")];
    let filtered = messages_since_last_assistant(&messages);
    assert_eq!(filtered.indices, vec![0, 1]);
}

#[test]
fn _build_guard_input_for_request_structured_messages() {
    let built = build_guard_input_for_request(&inputs(vec![
        message("system", "sys"),
        message("user", "hello"),
    ]))
    .unwrap()
    .expect("a structured request builds an input");
    assert_eq!(built.sent_indices, vec![0, 1]);
    assert_eq!(built.guard_input.messages.len(), 2);
    assert_eq!(
        built.guard_input.messages[0].content,
        Some(MessageContent::Text("sys".to_owned()))
    );
    assert_eq!(
        built.guard_input.messages[1].content,
        Some(MessageContent::Text("hello".to_owned()))
    );
    assert_eq!(built.guard_input.tools, Some(Vec::new()));
}

#[test]
fn _build_guard_input_for_request_texts_fallback() {
    let request = GuardrailApiInputs {
        texts: Some(vec!["alpha".to_owned(), "beta".to_owned()]),
        ..empty_inputs()
    };
    let built = build_guard_input_for_request(&request).unwrap().unwrap();
    assert_eq!(built.sent_indices, vec![0, 1]);
    assert_eq!(built.guard_input.messages.len(), 2);
    assert_eq!(built.guard_input.messages[0].role, "user");
    assert_eq!(
        built.guard_input.messages[0].content,
        Some(MessageContent::Text("alpha".to_owned()))
    );
}

#[test]
fn _build_guard_input_for_request_no_input() {
    assert!(
        build_guard_input_for_request(&empty_inputs())
            .unwrap()
            .is_none()
    );
    let empty_structured = GuardrailApiInputs {
        structured_messages: Some(Vec::new()),
        texts: Some(Vec::new()),
        ..empty_inputs()
    };
    assert!(
        build_guard_input_for_request(&empty_structured)
            .unwrap()
            .is_none()
    );
}

#[test]
fn _build_guard_input_for_request_empty_content() {
    let mut null_content = message("user", "placeholder");
    null_content
        .extra
        .insert("content".to_owned(), GuardrailValue::Null);
    let mut empty_list = message("user", "placeholder");
    empty_list
        .extra
        .insert("content".to_owned(), GuardrailValue::Array(Vec::new()));
    let built = build_guard_input_for_request(&inputs(vec![null_content, empty_list]))
        .unwrap()
        .unwrap();
    assert_eq!(
        built.guard_input.messages[0].content,
        Some(MessageContent::Text(String::new()))
    );
    assert_eq!(
        built.guard_input.messages[1].content,
        Some(MessageContent::Text(String::new()))
    );
}

#[test]
fn _build_guard_input_for_request_keeps_non_empty_content_parts() {
    let mut listed = message("user", "placeholder");
    listed.extra.insert(
        "content".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
            ("type".to_owned(), GuardrailValue::Text("text".to_owned())),
            ("text".to_owned(), GuardrailValue::Text("part".to_owned())),
        ]))]),
    );
    let built = build_guard_input_for_request(&inputs(vec![listed]))
        .unwrap()
        .unwrap();
    assert_eq!(
        built.guard_input.messages[0].content,
        Some(MessageContent::Parts(vec![ContentPart::Text(
            TextContentPart {
                part_type: TextPartType::Text,
                text: "part".to_owned(),
            }
        )]))
    );
}

#[test]
fn _build_guard_input_for_request_copies_non_empty_tools() {
    let tools = vec![GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    }];
    let request = GuardrailApiInputs {
        texts: Some(vec!["hi".to_owned()]),
        tools: Some(tools.clone()),
        ..empty_inputs()
    };
    let built = build_guard_input_for_request(&request).unwrap().unwrap();
    assert_eq!(built.guard_input.tools, Some(tools));
}

#[test]
fn _build_guard_input_for_response() {
    let request = GuardrailApiInputs {
        texts: Some(vec!["answer one".to_owned()]),
        ..empty_inputs()
    };
    let built = build_guard_input_for_response(&request);
    assert_eq!(built.messages.len(), 1);
    assert_eq!(built.messages[0].role, "assistant");
    assert_eq!(
        built.messages[0].content,
        Some(MessageContent::Text("answer one".to_owned()))
    );
    assert_eq!(built.tools, Some(Vec::new()));
    assert!(
        build_guard_input_for_response(&empty_inputs())
            .messages
            .is_empty()
    );
}

#[test]
fn _extract_transformed_texts() {
    let output = GuardInput {
        messages: vec![
            Message {
                role: "user".to_owned(),
                content: Some(MessageContent::Text("u".to_owned())),
            },
            Message {
                role: "assistant".to_owned(),
                content: Some(MessageContent::Text("a".to_owned())),
            },
        ],
        tools: None,
    };
    assert_eq!(extract_transformed_texts(&output, 1), vec!["a".to_owned()]);
    assert_eq!(
        extract_transformed_texts(&output, 2),
        vec!["u".to_owned(), "a".to_owned()]
    );
    assert!(extract_transformed_texts(&output, 0).is_empty());
    assert_eq!(
        extract_transformed_texts(&output, 9),
        vec!["u".to_owned(), "a".to_owned()]
    );
}
