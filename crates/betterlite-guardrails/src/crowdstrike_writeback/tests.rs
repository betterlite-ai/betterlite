#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{
    SkipFlags, apply_message_redaction, merge_request_transforms, redacted_messages,
    writeback_messages,
};
use crate::crowdstrike_aidr::handler::{ConstructorArgs, CrowdStrikeAidr, Dependencies};
use crate::crowdstrike_aidr::models::{
    ContentPart, GuardInput, Message, MessageContent, TextContentPart, TextPartType,
};
use crate::crowdstrike_streaming::StreamingParams;
use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::record::{GuardrailInvocationRecord, GuardrailSpanSink};

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn handler() -> CrowdStrikeAidr {
    let args = ConstructorArgs {
        guardrail_name: Some("cs-aidr".to_owned()),
        api_key: Some("key".to_owned()),
        api_base: Some("https://cs.test".to_owned()),
        fail_on_error: None,
        streaming: StreamingParams::default(),
        timeout: None,
    };
    CrowdStrikeAidr::new(
        &args,
        &BTreeMap::<String, String>::new(),
        Dependencies {
            client: reqwest::Client::default(),
            sink: Box::new(DiscardSink),
            now: std::time::SystemTime::now,
        },
    )
    .expect("the constructor gets both secrets")
}

fn message(role: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        )]),
    }
}

fn list_message(role: &str, parts: Vec<GuardrailValue>) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([("content".to_owned(), GuardrailValue::Array(parts))]),
    }
}

fn text_block(value: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        ("type".to_owned(), GuardrailValue::Text("text".to_owned())),
        ("text".to_owned(), GuardrailValue::Text(value.to_owned())),
    ]))
}

fn output(content: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(MessageContent::Text(content.to_owned())),
    }
}

fn guard_input(messages: Vec<Message>) -> GuardInput {
    GuardInput {
        messages,
        tools: None,
    }
}

fn content_of(message: &GuardrailMessage) -> &GuardrailValue {
    message
        .extra
        .get("content")
        .unwrap_or(&GuardrailValue::Null)
}

#[test]
fn _merge_request_transforms() {
    let structured = [
        message("system", "s0"),
        message("user", "u1"),
        message("assistant", "a2"),
        message("user", "u3"),
    ];
    let output = guard_input(vec![
        Message {
            role: "system".to_owned(),
            content: Some(MessageContent::Text("R0".to_owned())),
        },
        Message {
            role: "user".to_owned(),
            content: Some(MessageContent::Text("R3".to_owned())),
        },
    ]);
    let texts = merge_request_transforms(&output, Some(&structured), &[], &[0, 3]);
    assert_eq!(
        texts,
        vec![
            "R0".to_owned(),
            "u1".to_owned(),
            "a2".to_owned(),
            "R3".to_owned()
        ]
    );
}

#[test]
fn _merge_request_transforms_uses_texts_when_no_structured_messages() {
    let output = guard_input(vec![output("masked")]);
    let texts = merge_request_transforms(&output, None, &["one".to_owned()], &[0]);
    assert_eq!(texts, vec!["masked".to_owned()]);
    let untouched = merge_request_transforms(&output, None, &["one".to_owned()], &[]);
    assert_eq!(untouched, vec!["one".to_owned()]);
}

#[test]
fn _merge_request_transforms_ignores_indices_past_either_list() {
    let output = guard_input(vec![output("masked")]);
    let out_of_range = merge_request_transforms(&output, None, &["one".to_owned()], &[5]);
    assert_eq!(out_of_range, vec!["one".to_owned()]);
    let too_many_positions = merge_request_transforms(
        &output,
        None,
        &["one".to_owned(), "two".to_owned()],
        &[0, 1],
    );
    assert_eq!(
        too_many_positions,
        vec!["masked".to_owned(), "two".to_owned()]
    );
}

#[test]
fn _apply_message_redaction_string_content() {
    let original = message("user", "secret");
    let redacted = apply_message_redaction(&original, &output("masked"));
    assert_eq!(
        content_of(&redacted),
        &GuardrailValue::Text("masked".to_owned())
    );
    assert_eq!(original.extra, message("user", "secret").extra);
}

#[test]
fn _apply_message_redaction_list_content() {
    let original = list_message("user", vec![text_block("secret")]);
    let redacted_message = Message {
        role: "user".to_owned(),
        content: Some(MessageContent::Parts(vec![ContentPart::Text(
            TextContentPart {
                part_type: TextPartType::Text,
                text: "masked".to_owned(),
            },
        )])),
    };
    let redacted = apply_message_redaction(&original, &redacted_message);
    assert_eq!(
        content_of(&redacted),
        &GuardrailValue::Array(vec![text_block("masked")])
    );
}

#[test]
fn _apply_message_redaction_list_content_keeps_a_text_content_string_reply() {
    let original = list_message("user", vec![text_block("secret")]);
    let redacted = apply_message_redaction(&original, &output("masked"));
    assert_eq!(
        content_of(&redacted),
        &GuardrailValue::Text("masked".to_owned())
    );
}

#[test]
fn _apply_message_redaction_no_text_content() {
    let empty = list_message("user", Vec::new());
    let unchanged = apply_message_redaction(&empty, &output("masked"));
    assert_eq!(unchanged, empty);
    let image_only = list_message(
        "user",
        vec![GuardrailValue::Object(BTreeMap::from([(
            "type".to_owned(),
            GuardrailValue::Text("image_url".to_owned()),
        )]))],
    );
    assert_eq!(
        apply_message_redaction(&image_only, &output("masked")),
        image_only
    );
    let no_content = GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::new(),
    };
    assert_eq!(
        apply_message_redaction(&no_content, &output("masked")),
        no_content
    );
}

#[test]
fn _redacted_messages() {
    let structured = [message("user", "u"), message("assistant", "a")];
    let output = guard_input(vec![Message {
        role: "assistant".to_owned(),
        content: Some(MessageContent::Text("A2".to_owned())),
    }]);
    let rebuilt = redacted_messages(&structured, &output, &[1], &structured).expect("aligned");
    assert_eq!(rebuilt[0], structured[0]);
    assert_eq!(
        content_of(&rebuilt[1]),
        &GuardrailValue::Text("A2".to_owned())
    );
    assert_eq!(rebuilt.len(), 2);
}

#[test]
fn _redacted_messages_returns_none_when_an_index_leaves_the_message_list() {
    let structured = [message("user", "u")];
    let output = guard_input(vec![output("masked")]);
    assert_eq!(
        redacted_messages(&structured, &output, &[4], &structured),
        None
    );
}

#[test]
fn _redacted_messages_returns_none_when_the_full_list_holds_no_match() {
    let structured = [message("user", "u")];
    let other_full = [message("assistant", "a")];
    let output = guard_input(vec![output("masked")]);
    assert_eq!(
        redacted_messages(&structured, &output, &[0], &other_full),
        None
    );
}

#[test]
fn structured_messages_cover_full_request() {
    let mut guardrail = handler();
    assert!(!guardrail.messages_cover_full_request());
    guardrail.settings.skip = SkipFlags {
        system: true,
        tool: false,
    };
    assert!(guardrail.messages_cover_full_request());
    guardrail.settings.skip = SkipFlags {
        system: false,
        tool: true,
    };
    assert!(guardrail.messages_cover_full_request());
}

#[test]
fn _writeback_messages_skipping_enabled() {
    let structured = [message("user", "u")];
    let request_data = BTreeMap::from([(
        "messages".to_owned(),
        GuardrailValue::Array(vec![
            GuardrailValue::Object(BTreeMap::from([
                ("role".to_owned(), GuardrailValue::Text("system".to_owned())),
                ("content".to_owned(), GuardrailValue::Text("sys".to_owned())),
            ])),
            GuardrailValue::Object(BTreeMap::from([
                ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
                ("content".to_owned(), GuardrailValue::Text("u".to_owned())),
            ])),
        ]),
    )]);
    let output = guard_input(vec![output("masked")]);
    let rebuilt = writeback_messages(
        &structured,
        &output,
        &[0],
        &request_data,
        SkipFlags {
            system: true,
            tool: false,
        },
    )
    .expect("the request message list aligns");
    assert_eq!(rebuilt.len(), 2);
    assert_eq!(
        content_of(&rebuilt[0]),
        &GuardrailValue::Text("sys".to_owned())
    );
    assert_eq!(
        content_of(&rebuilt[1]),
        &GuardrailValue::Text("masked".to_owned())
    );
}

#[test]
fn _writeback_messages_skipping_enabled_ignores_a_non_list_request_messages() {
    let structured = [message("user", "u")];
    let request_data = BTreeMap::from([(
        "messages".to_owned(),
        GuardrailValue::Text("nope".to_owned()),
    )]);
    let output = guard_input(vec![output("masked")]);
    let rebuilt = writeback_messages(
        &structured,
        &output,
        &[0],
        &request_data,
        SkipFlags {
            system: true,
            tool: false,
        },
    )
    .expect("the structured list is the fallback");
    assert_eq!(
        content_of(&rebuilt[0]),
        &GuardrailValue::Text("masked".to_owned())
    );
}

#[test]
fn _writeback_messages_no_skipping() {
    let structured = [message("user", "u"), message("assistant", "a")];
    let output = guard_input(vec![Message {
        role: "user".to_owned(),
        content: Some(MessageContent::Text("masked".to_owned())),
    }]);
    let rebuilt = writeback_messages(
        &structured,
        &output,
        &[0],
        &BTreeMap::new(),
        SkipFlags::default(),
    )
    .expect("the structured list aligns");
    assert_eq!(
        content_of(&rebuilt[0]),
        &GuardrailValue::Text("masked".to_owned())
    );
    assert_eq!(rebuilt[1], structured[1]);
}

#[test]
fn _writeback_messages_applies_redactions() {
    let structured = [message("user", "u")];
    let output = guard_input(vec![output("masked")]);
    let rebuilt = writeback_messages(
        &structured,
        &output,
        &[0],
        &BTreeMap::new(),
        SkipFlags::default(),
    )
    .expect("the structured list aligns");
    assert_eq!(
        content_of(&rebuilt[0]),
        &GuardrailValue::Text("masked".to_owned())
    );
}
