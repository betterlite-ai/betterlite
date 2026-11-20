#![cfg(test)]

use betterlite_guardrails::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::images::image_sources;
use super::{
    extract_input_text_and_images, extract_top_level_system_text, get_structured_messages,
};
use crate::guardrails::anthropic_translation::scan_targets::{
    ExtractedInput, InputWriteBackTarget, MessageTextTarget, ScannedText, ToolUseInputTarget,
};

const ALL: ScopeFlags = ScopeFlags {
    skip_system_message: false,
    skip_tool_message: false,
    scan: ScanScope::AllMessages,
};

const SCAN_ONLY: ScopeFlags = ScopeFlags {
    skip_system_message: false,
    skip_tool_message: false,
    scan: ScanScope::ToolResultsOnly,
};

fn texts(found: &ExtractedInput) -> Vec<&str> {
    found
        .scanned
        .iter()
        .map(|item| item.text.as_str())
        .collect()
}

fn message_target(text: &str, target: MessageTextTarget) -> ScannedText {
    ScannedText {
        text: text.to_owned(),
        target: InputWriteBackTarget::Message(target),
    }
}

fn object(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().expect("an object fixture")
}

#[test]
fn extract_string_content_scans_one_message_content() {
    let found = extract_input_text_and_images(&json!({"role": "user", "content": "hello"}), 2, ALL);
    assert_eq!(
        found,
        ExtractedInput {
            scanned: vec![message_target(
                "hello",
                MessageTextTarget::MessageContent { msg_idx: 2 }
            )],
            images: Vec::new(),
            tool_calls: Vec::new(),
        }
    );
}

#[test]
fn extract_empty_string_content_is_still_scanned() {
    let found = extract_input_text_and_images(&json!({"role": "user", "content": ""}), 0, ALL);
    assert_eq!(texts(&found), vec![""]);
}

#[test]
fn extract_text_blocks_keep_content_positions() {
    let message = json!({"role": "user", "content": [
        {"type": "text", "text": "a"},
        {"type": "image", "source": {"type": "url", "url": "http://x/y.png"}},
        {"type": "custom", "text": "b"},
        {"type": "text", "text": ""},
        "junk",
        7
    ]});
    let found = extract_input_text_and_images(&message, 1, ALL);
    assert_eq!(texts(&found), vec!["a", "b", ""]);
    assert_eq!(
        found.scanned[1].target,
        InputWriteBackTarget::Message(MessageTextTarget::ContentBlockText {
            msg_idx: 1,
            content_idx: 2
        })
    );
    assert_eq!(
        found.scanned[2].target,
        InputWriteBackTarget::Message(MessageTextTarget::ContentBlockText {
            msg_idx: 1,
            content_idx: 3
        })
    );
    assert_eq!(found.images, vec!["http://x/y.png"]);
}

#[test]
fn extract_scan_only_keeps_only_tool_result_texts() {
    let message = json!({"role": "user", "content": [
        {"type": "text", "text": "prompt"},
        {"type": "tool_result", "tool_use_id": "t", "content": "res"},
        {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {}}
    ]});
    let found = extract_input_text_and_images(&message, 0, SCAN_ONLY);
    assert_eq!(texts(&found), vec!["res"]);
    assert!(found.tool_calls.is_empty());
    let plain =
        extract_input_text_and_images(&json!({"role": "user", "content": "hi"}), 0, SCAN_ONLY);
    assert!(plain.scanned.is_empty());
}

#[test]
fn extract_tool_result_strings_and_items() {
    let message = json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "t", "content": "res"},
        {"type": "tool_result", "tool_use_id": "t", "content": [
            {"type": "text", "text": "r1"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAA"}},
            {"type": "image", "source": {"type": "file", "file_id": "f"}},
            "junk"
        ]}
    ]});
    let found = extract_input_text_and_images(&message, 4, ALL);
    assert_eq!(texts(&found), vec!["res", "r1"]);
    assert_eq!(
        found.scanned[0].target,
        InputWriteBackTarget::Message(MessageTextTarget::ToolResultString {
            msg_idx: 4,
            content_idx: 0
        })
    );
    assert_eq!(
        found.scanned[1].target,
        InputWriteBackTarget::Message(MessageTextTarget::ToolResultBlockText {
            msg_idx: 4,
            content_idx: 1,
            block_idx: 0
        })
    );
    assert_eq!(found.images, vec!["data:image/png;base64,AAA"]);
}

#[test]
fn extract_tool_result_other_content_is_empty() {
    let message = json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "t"},
        {"type": "tool_result", "tool_use_id": "t", "content": 3},
        {"type": "tool_result", "tool_use_id": "t", "content": {"text": "x"}}
    ]});
    assert_eq!(
        extract_input_text_and_images(&message, 0, ALL),
        ExtractedInput::empty()
    );
}

#[test]
fn extract_system_role_scans_midturn_even_when_skip_system_is_set() {
    let skip_system = ScopeFlags {
        skip_system_message: true,
        ..ALL
    };
    let found =
        extract_input_text_and_images(&json!({"role": "system", "content": "mid"}), 3, skip_system);
    assert_eq!(texts(&found), vec!["mid"]);
    let scan_only =
        extract_input_text_and_images(&json!({"role": "system", "content": "mid"}), 3, SCAN_ONLY);
    assert!(scan_only.scanned.is_empty());
}

#[test]
fn extract_tool_role_is_skipped_only_with_the_flag() {
    let skip_tool = ScopeFlags {
        skip_tool_message: true,
        ..ALL
    };
    let skipped =
        extract_input_text_and_images(&json!({"role": "Tool", "content": "t"}), 0, skip_tool);
    assert!(skipped.scanned.is_empty());
    let kept = extract_input_text_and_images(&json!({"role": "Tool", "content": "t"}), 0, ALL);
    assert_eq!(texts(&kept), vec!["t"]);
}

#[test]
fn extract_skipped_tool_role_keeps_tool_result_texts() {
    let skip_tool = ScopeFlags {
        skip_tool_message: true,
        ..ALL
    };
    let message = json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "t", "content": "res"},
        {"type": "text", "text": "kept"}
    ]});
    let found = extract_input_text_and_images(&message, 0, skip_tool);
    assert_eq!(texts(&found), vec!["kept"]);
}

#[rstest]
#[case(json!({"role": "user"}))]
#[case(json!({"role": "user", "content": 5}))]
#[case(json!({"role": "user", "content": {"text": "x"}}))]
#[case(json!({"role": "user", "content": null}))]
#[case(json!("plain text"))]
fn extract_missing_or_non_string_content_is_empty(#[case] message: Value) {
    assert_eq!(
        extract_input_text_and_images(&message, 0, ALL),
        ExtractedInput::empty()
    );
}

#[test]
fn extract_tool_use_running_index_counts_only_client_tool_use() {
    let message = json!({"role": "user", "content": [
        {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}},
        {"type": "tool_use", "id": "toolu_2", "name": "no_input"},
        {"type": "text", "text": "between"},
        {"type": "tool_use", "id": "toolu_3", "name": "other", "input": {}}
    ]});
    let found = extract_input_text_and_images(&message, 7, ALL);
    assert_eq!(found.tool_calls.len(), 2);
    assert_eq!(
        found.tool_calls[0].target,
        ToolUseInputTarget {
            msg_idx: 7,
            content_idx: 0
        }
    );
    assert_eq!(
        found.tool_calls[1].target,
        ToolUseInputTarget {
            msg_idx: 7,
            content_idx: 3
        }
    );
    assert_eq!(
        found.tool_calls[0].tool_call["function"]["arguments"].as_str(),
        Some("{\"a\": 1}")
    );
    assert_eq!(found.tool_calls[1].tool_call["index"].as_u64(), Some(1));
    assert_eq!(
        found.tool_calls[1].tool_call["id"].as_str(),
        Some("toolu_3")
    );
}

#[test]
fn extract_tool_use_is_not_scanned_in_scan_only() {
    let message = json!({"role": "user", "content": [
        {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}}
    ]});
    let found = extract_input_text_and_images(&message, 0, SCAN_ONLY);
    assert!(found.tool_calls.is_empty());
}

#[rstest]
#[case(json!({"type": "image", "source": {"type": "url", "url": "http://u"}}), vec!["http://u".to_owned()])]
#[case(json!({"type": "image", "source": {"type": "url", "url": ""}}), vec![])]
#[case(json!({"type": "image", "source": {"type": "url"}}), vec![])]
#[case(json!({"type": "image", "source": {"type": "file", "file_id": "f"}}), vec![])]
#[case(json!({"type": "image", "source": "text"}), vec![])]
#[case(json!({"type": "image"}), vec![])]
fn image_sources_url_and_missing_sources(#[case] block: Value, #[case] expected: Vec<String>) {
    assert_eq!(image_sources(&object(&block)), expected);
}

#[rstest]
#[case(
    json!({"source": {"type": "base64", "media_type": "image/png", "data": "D"}}),
    vec!["data:image/png;base64,D".to_owned()]
)]
#[case(json!({"source": {"type": "base64", "media_type": "", "data": "D"}}), vec!["D".to_owned()])]
#[case(json!({"source": {"type": "base64", "data": "D"}}), vec!["D".to_owned()])]
#[case(json!({"source": {"type": "base64", "media_type": "image/png", "data": ""}}), vec![])]
fn image_sources_base64_forms(#[case] block: Value, #[case] expected: Vec<String>) {
    assert_eq!(image_sources(&object(&block)), expected);
}

#[test]
fn midturn_system_array_scans_text_blocks_with_the_real_position() {
    let message = json!({"role": "system", "content": [
        {"type": "text", "text": "s1"},
        {"type": "text", "text": ""},
        {"type": "text", "text": 5},
        {"type": "image", "source": {}},
        {"type": "text", "text": "s3"}
    ]});
    let found = extract_input_text_and_images(&message, 2, ALL);
    assert_eq!(texts(&found), vec!["s1", "s3"]);
    assert_eq!(
        found.scanned[1].target,
        InputWriteBackTarget::Message(MessageTextTarget::ContentBlockText {
            msg_idx: 2,
            content_idx: 4
        })
    );
    assert!(found.images.is_empty());
}

#[rstest]
#[case(json!(""))]
#[case(json!(5))]
#[case(json!({"text": "x"}))]
fn midturn_system_other_content_is_empty(#[case] content: Value) {
    let message = json!({"role": "system", "content": content});
    assert_eq!(
        extract_input_text_and_images(&message, 0, ALL),
        ExtractedInput::empty()
    );
}

#[test]
fn top_level_system_string_is_scanned_even_when_empty() {
    let hoisted = json!({"role": "system", "content": ""});
    assert_eq!(
        extract_top_level_system_text(&hoisted),
        vec![ScannedText {
            text: String::new(),
            target: InputWriteBackTarget::SystemString,
        }]
    );
}

#[test]
fn top_level_system_blocks_number_over_all_items() {
    let hoisted = json!({"role": "system", "content": [
        {"type": "text", "text": "a"},
        {"type": "image", "source": {}},
        {"type": "text", "text": ""},
        {"type": "text", "text": 5}
    ]});
    assert_eq!(
        extract_top_level_system_text(&hoisted),
        vec![
            ScannedText {
                text: "a".to_owned(),
                target: InputWriteBackTarget::SystemBlockText { block_idx: 0 },
            },
            ScannedText {
                text: String::new(),
                target: InputWriteBackTarget::SystemBlockText { block_idx: 2 },
            },
        ]
    );
}

#[rstest]
#[case(json!(5))]
#[case(json!({"text": "x"}))]
#[case(json!(null))]
fn top_level_system_non_string_content_scans_nothing(#[case] content: Value) {
    assert!(
        extract_top_level_system_text(&json!({"role": "system", "content": content})).is_empty()
    );
}

#[test]
fn top_level_system_missing_content_scans_nothing() {
    assert!(extract_top_level_system_text(&json!({"role": "system"})).is_empty());
}

#[tokio::test]
async fn structured_messages_missing_or_null_is_none() {
    let missing = object(&json!({"model": "claude-sonnet-4-5"}));
    assert_eq!(get_structured_messages(&missing).await.unwrap(), None);
    let null = object(&json!({"model": "claude-sonnet-4-5", "messages": null}));
    assert_eq!(get_structured_messages(&null).await.unwrap(), None);
}

#[tokio::test]
async fn structured_messages_empty_translation_is_none() {
    let data = object(&json!({"model": "claude-sonnet-4-5", "messages": []}));
    assert_eq!(get_structured_messages(&data).await.unwrap(), None);
}

#[tokio::test]
async fn structured_messages_include_the_top_level_system_row() {
    let data = object(&json!({
        "model": "claude-sonnet-4-5",
        "system": "trusted",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let rows = get_structured_messages(&data).await.unwrap().unwrap();
    assert_eq!(
        rows,
        vec![
            json!({"role": "system", "content": "trusted"}),
            json!({"role": "user", "content": "hi"}),
        ]
    );
}
