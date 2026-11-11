use betterlite_openai::chat::{ContentPart, MessageContent, Role};
use rstest::rstest;
use serde_json::{Value, json};

use super::prelude::{text_of, translate};

#[test]
fn contents_without_a_role_translate_as_user_text() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{"text": "hi"}]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::User);
    assert_eq!(
        request.messages[0].content,
        Some(MessageContent::Text(String::from("hi")))
    );
}

#[test]
fn a_single_content_object_translates_like_a_one_item_list() {
    let request = translate(json!({
        "model": "m",
        "contents": {"parts": [{"text": "hi"}]},
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(text_of(&request.messages[0]), "hi");
}

#[rstest]
#[case::system("system")]
#[case::function("function")]
#[case::tool("tool")]
fn contents_with_another_role_are_dropped(#[case] role: &str) {
    let request = translate(json!({
        "model": "m",
        "contents": [{"role": role, "parts": [{"text": "gone"}]}],
    }));
    assert!(request.messages.is_empty());
}

#[test]
fn model_text_parts_join_into_one_assistant_message() {
    let request = translate(json!({
        "model": "m",
        "contents": [
            {"role": "model", "parts": [{"text": "one "}, "two", {"text": " three"}]},
        ],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::Assistant);
    assert_eq!(
        request.messages[0].content,
        Some(MessageContent::Text(String::from("one two three")))
    );
}

#[test]
fn a_model_content_without_text_or_calls_keeps_a_null_content() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"role": "model", "parts": []}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::Assistant);
    assert_eq!(request.messages[0].content, None);
    assert_eq!(request.messages[0].tool_calls, None);
}

#[test]
fn bare_string_parts_translate_as_text() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": ["plain"]}],
    }));
    assert_eq!(text_of(&request.messages[0]), "plain");
}

#[test]
fn a_null_parts_list_is_an_empty_list() {
    let request = translate(json!({
        "model": "m",
        "contents": [
            {"parts": null},
            {"role": "model", "parts": null},
        ],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::Assistant);
    assert_eq!(request.messages[0].content, None);
}

#[test]
fn multipart_user_content_keeps_every_part() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{"text": "look"}, {"inline_data": {"mime_type": "image/png", "data": "QUJD"}}]}],
    }));
    let Some(MessageContent::Parts(parts)) = &request.messages[0].content else {
        panic!("expected content parts");
    };
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].kind, "text");
    assert_eq!(parts[1].kind, "image_url");
    assert_eq!(
        parts[1].fields.get("image_url"),
        Some(&json!({"url": "data:image/png;base64,QUJD"}))
    );
}

#[rstest]
#[case::complete(json!({"mime_type": "image/webp", "data": "AA"}), "data:image/webp;base64,AA")]
#[case::default_mime(json!({"data": "AA"}), "data:image/jpeg;base64,AA")]
#[case::missing_data(json!({"mime_type": "image/gif"}), "data:image/gif;base64,")]
#[case::empty_data(json!({"mime_type": "image/gif", "data": ""}), "data:image/gif;base64,")]
fn inline_data_becomes_an_image_url(#[case] inline: Value, #[case] expected: &str) {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{"inline_data": inline}]}],
    }));
    assert_eq!(
        request.messages[0].content,
        Some(MessageContent::Parts(vec![ContentPart {
            kind: String::from("image_url"),
            fields: indexmap::IndexMap::from([(
                String::from("image_url"),
                json!({"url": expected}),
            )]),
        }]))
    );
}

#[test]
fn function_responses_become_tool_messages_after_the_user_message() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [
            {"text": "checking"},
            {"functionResponse": {"name": "lookup", "response": {"answer": 7_i32}}},
        ]}],
    }));
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.messages[0].role, Role::User);
    assert_eq!(request.messages[1].role, Role::Tool);
    assert_eq!(
        request.messages[1].tool_call_id.as_deref(),
        Some("call_lookup")
    );
    let Some(MessageContent::Text(content)) = &request.messages[1].content else {
        panic!("expected tool content");
    };
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("encoded response"),
        json!({"answer": 7_i32})
    );
}

#[test]
fn a_function_response_alone_produces_only_the_tool_message() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{"functionResponse": {"name": "lookup", "response": {}}}]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::Tool);
}

#[test]
fn a_function_response_without_name_or_response_uses_the_fallbacks() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{"functionResponse": {}}]}],
    }));
    assert_eq!(
        request.messages[0].tool_call_id.as_deref(),
        Some("call_unknown")
    );
    let Some(MessageContent::Text(content)) = &request.messages[0].content else {
        panic!("expected tool content");
    };
    assert_eq!(content, "{}");
}

#[test]
fn function_calls_become_assistant_tool_calls_with_content() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"role": "model", "parts": [
            {"text": "calling"},
            {"functionCall": {"name": "lookup", "args": {"id": 3_i32}}},
        ]}],
    }));
    assert_eq!(request.messages.len(), 1);
    let message = &request.messages[0];
    assert_eq!(message.role, Role::Assistant);
    assert_eq!(
        message.content,
        Some(MessageContent::Text(String::from("calling")))
    );
    let calls = message.tool_calls.as_ref().expect("tool calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_lookup");
    assert_eq!(calls[0].kind, "function");
    assert_eq!(calls[0].function.name, "lookup");
    assert_eq!(
        serde_json::from_str::<Value>(&calls[0].function.arguments).expect("encoded args"),
        json!({"id": 3_i32})
    );
}

#[test]
fn a_function_call_without_name_or_args_uses_the_fallbacks() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"role": "model", "parts": [{"functionCall": {}}]}],
    }));
    let calls = request.messages[0].tool_calls.as_ref().expect("tool calls");
    assert_eq!(calls[0].id, "call_unknown");
    assert_eq!(calls[0].function.name, "");
    assert_eq!(calls[0].function.arguments, "{}");
}

#[test]
fn a_user_part_with_an_unknown_shape_is_ignored() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{"video": {"uri": "x"}}, {"text": "kept"}]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(text_of(&request.messages[0]), "kept");
}

#[test]
fn a_user_part_with_text_and_a_function_response_keeps_only_the_text() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{
            "text": "text wins",
            "inline_data": {"mime_type": "image/png", "data": "QUJD"},
            "functionResponse": {"name": "lookup", "response": {}},
        }]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(
        request.messages[0].content,
        Some(MessageContent::Text(String::from("text wins")))
    );
}

#[test]
fn a_user_part_with_inline_data_and_a_function_response_keeps_only_the_image() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{
            "inline_data": {"mime_type": "image/png", "data": "QUJD"},
            "functionResponse": {"name": "lookup", "response": {}},
        }]}],
    }));
    assert_eq!(request.messages.len(), 1);
    let Some(MessageContent::Parts(parts)) = &request.messages[0].content else {
        panic!("expected content parts");
    };
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].kind, "image_url");
}

#[test]
fn a_user_part_with_a_function_call_and_response_uses_the_response() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{
            "functionCall": {"name": "ignored", "args": {}},
            "functionResponse": {"name": "lookup", "response": {"answer": 7_i32}},
        }]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::Tool);
    assert_eq!(
        request.messages[0].tool_call_id.as_deref(),
        Some("call_lookup")
    );
}

#[test]
fn non_object_contents_parts_are_ignored() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [7_i32, null, [1_i32], {"text": "kept"}]}],
    }));
    assert_eq!(text_of(&request.messages[0]), "kept");
}

#[test]
fn a_model_part_with_text_and_a_function_call_keeps_only_the_text() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"role": "model", "parts": [{
            "text": "text wins",
            "inline_data": {"mime_type": "image/png", "data": "QUJD"},
            "functionResponse": {"name": "other", "response": {}},
            "functionCall": {"name": "lookup", "args": {}},
        }]}],
    }));
    assert_eq!(
        request.messages[0].content,
        Some(MessageContent::Text(String::from("text wins")))
    );
    assert_eq!(request.messages[0].tool_calls, None);
}

#[test]
fn a_model_part_with_a_function_call_and_other_keys_keeps_the_tool_call() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"role": "model", "parts": [{
            "inline_data": {"mime_type": "image/png", "data": "QUJD"},
            "functionResponse": {"name": "other", "response": {}},
            "functionCall": {"name": "lookup", "args": {"id": 3_i32}},
        }]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::Assistant);
    assert_eq!(request.messages[0].content, None);
    let calls = request.messages[0].tool_calls.as_ref().expect("tool calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_lookup");
    assert_eq!(calls[0].function.name, "lookup");
}

#[test]
fn a_null_function_call_args_encodes_as_an_empty_object() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"role": "model", "parts": [{"functionCall": {"name": "lookup", "args": null}}]}],
    }));
    let calls = request.messages[0].tool_calls.as_ref().expect("tool calls");
    assert_eq!(calls[0].function.arguments, "{}");
}

#[test]
fn a_null_function_response_encodes_as_an_empty_object() {
    let request = translate(json!({
        "model": "m",
        "contents": [{"parts": [{"functionResponse": {"name": "lookup", "response": null}}]}],
    }));
    let Some(MessageContent::Text(content)) = &request.messages[0].content else {
        panic!("expected tool content");
    };
    assert_eq!(content, "{}");
}
