#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Value, json};
use uuid::Uuid;

use super::MistralChatError;
use super::reply::{
    decode_mistral_chat, handle_content_list_to_str_conversion, handle_empty_content_response,
};
use super::request::{
    MessageTransform, RequestTransform, add_reasoning_system_prompt_if_needed,
    handle_message_with_file, handle_name_in_message, handle_tool_call_message,
    is_empty_assistant_message, strip_output_only_fields, transform_messages, transform_request,
};
use super::stream::chunk_parser;
use crate::chat::ChatMessage;
use crate::chat_choices::stream::StreamError;
use crate::chat_content::{Mode, RefuseDownloads};
use crate::mistral_options::REASONING_SYSTEM_PROMPT;

fn message(value: Value) -> ChatMessage {
    serde_json::from_value(value).expect("a chat message")
}

fn prompt(value: Value) -> betterlite_prompts::message_content::Message {
    serde_json::from_value(value).expect("a prompt message")
}

fn request_body(model: &str, messages: Value, optional: Value) -> Value {
    let messages: Vec<ChatMessage> = serde_json::from_value(messages).expect("messages");
    let optional: Object = serde_json::from_value(optional).expect("optional params");
    let body = transform_request(&RequestTransform {
        model,
        messages: &messages,
        optional_params: &optional,
        downloads: &RefuseDownloads,
    })
    .expect("a transformed request");
    serde_json::to_value(body).expect("a body")
}

#[test]
fn text_messages_drop_names_and_project_tool_calls() {
    let body = request_body(
        "mistral-small-latest",
        json!([
            {"role": "user", "content": [{"type": "text", "text": "hi"}], "name": "ada"},
            {"role": "tool", "content": "ok", "name": "lookup", "tool_call_id": "call-1"},
            {"role": "tool", "content": "ok", "name": "   ", "tool_call_id": "call-2"},
            {"role": "assistant", "content": "", "tool_calls": [
                {"id": "call-3", "type": "custom", "function": {"name": "f", "arguments": "{}"}, "extra": 1}
            ]}
        ]),
        json!({}),
    );
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages[0]["content"], json!("hi"));
    assert!(messages[0].get("name").is_none());
    assert_eq!(messages[1]["name"], json!("lookup"));
    assert!(messages[2].get("name").is_none());
    assert_eq!(
        messages[3]["tool_calls"],
        json!([{"id": "call-3", "type": "function", "function": {"name": "f", "arguments": "{}"}}])
    );
}

#[test]
fn empty_messages_are_dropped_without_a_role_check() {
    let body = request_body(
        "mistral-small-latest",
        json!([
            {"role": "user", "content": ""},
            {"role": "assistant", "content": "", "tool_calls": []},
            {"role": "assistant", "content": ""},
            {"role": "user", "content": "kept"}
        ]),
        json!({}),
    );
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], json!("assistant"));
    assert_eq!(messages[0]["tool_calls"], json!([]));
    assert_eq!(messages[1]["content"], json!("kept"));
}

#[test]
fn output_only_fields_leave_assistant_messages_only() {
    let stripped = strip_output_only_fields(&message(json!({
        "role": "assistant",
        "content": "hi",
        "reasoning_content": "why",
        "thinking_blocks": [{"type": "thinking"}]
    })));
    assert!(stripped.passthrough.get("reasoning_content").is_none());
    assert!(stripped.passthrough.get("thinking_blocks").is_none());
    let user = strip_output_only_fields(&message(json!({
        "role": "user",
        "content": "hi",
        "reasoning_content": "why"
    })));
    assert_eq!(
        user.passthrough.get("reasoning_content"),
        Some(&json!("why"))
    );
}

#[test]
fn empty_predicate_reads_assistant_field_names_on_every_role() {
    assert!(is_empty_assistant_message(&prompt(
        json!({"role": "user", "content": ""})
    )));
    assert!(!is_empty_assistant_message(&prompt(
        json!({"role": "assistant", "content": [], "content_": null})
    )));
    assert!(!is_empty_assistant_message(&prompt(
        json!({"role": "assistant", "content": "", "function_call": {"name": "f"}})
    )));
    assert!(!is_empty_assistant_message(&prompt(
        json!({"role": "assistant", "content": "", "cache_control": {"type": "ephemeral"}})
    )));
}

#[test]
fn names_survive_only_on_nonblank_tool_messages() {
    let kept = handle_name_in_message(prompt(json!({"role": "tool", "name": "f", "content": "x"})));
    assert_eq!(kept.name.as_deref(), Some("f"));
    let dropped =
        handle_name_in_message(prompt(json!({"role": "user", "name": "f", "content": "x"})));
    assert!(dropped.name.is_none());
    let blank =
        handle_name_in_message(prompt(json!({"role": "tool", "name": " ", "content": "x"})));
    assert!(blank.name.is_none());
}

#[test]
fn media_messages_keep_the_conversation_as_sent() {
    let body = request_body(
        "mistral-small-latest",
        json!([
            {"role": "user", "name": "ada", "content": [
                {"type": "text", "text": "look"},
                {"type": "image_url", "image_url": "https://example.test/a.png"},
                {"type": "file", "file": {"file_id": "file-1", "file_data": "ZGF0YQ=="}}
            ]},
            {"role": "assistant", "content": ""}
        ]),
        json!({}),
    );
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages[0]["name"], json!("ada"));
    assert_eq!(
        messages[0]["content"][1]["image_url"],
        json!({"url": "https://example.test/a.png"})
    );
    assert_eq!(messages[0]["content"][2]["file_id"], json!("file-1"));
    assert!(messages[0]["content"][2].get("file").is_none());
    assert_eq!(
        messages[1]["content"],
        json!(""),
        "the empty message is not filtered on the media path"
    );
}

#[test]
fn empty_file_ids_stay_on_the_file_object() {
    let mut messages = vec![message(json!({
        "role": "user",
        "content": [{"type": "file", "file": {"file_id": ""}}]
    }))];
    handle_message_with_file(&mut messages).expect("a lifted conversation");
    let part = &messages[0].content.as_ref().expect("content");
    let crate::chat::MessageContent::Parts(parts) = part else {
        panic!("parts");
    };
    assert!(parts[0].fields.get("file").is_some());
    assert!(parts[0].fields.get("file_id").is_none());
}

#[test]
fn a_non_object_file_field_fails() {
    let mut messages = vec![message(json!({
        "role": "assistant",
        "content": [{"type": "file", "file": "nope"}]
    }))];
    assert!(matches!(
        handle_message_with_file(&mut messages),
        Err(MistralChatError::FileBlockShape)
    ));
}

#[test]
fn the_reasoning_prompt_prepends_to_a_string_or_list_system_message() {
    let mut params = Object::from_iter([(String::from("_add_reasoning_prompt"), Json::Bool(true))]);
    let updated = add_reasoning_system_prompt_if_needed(
        "Magistral-Small-2506",
        &[message(json!({"role": "system", "content": "be brief"}))],
        &mut params,
    );
    assert_eq!(
        updated[0].content,
        Some(crate::chat::MessageContent::Text(format!(
            "{REASONING_SYSTEM_PROMPT}\n\nbe brief"
        )))
    );
    assert!(params.get("_add_reasoning_prompt").is_none());

    let mut params = Object::from_iter([(String::from("_add_reasoning_prompt"), Json::Bool(true))]);
    let updated = add_reasoning_system_prompt_if_needed(
        "magistral-medium",
        &[message(
            json!({"role": "system", "content": [{"type": "text", "text": "be brief"}]}),
        )],
        &mut params,
    );
    let crate::chat::MessageContent::Parts(parts) = updated[0].content.clone().expect("content")
    else {
        panic!("parts");
    };
    assert_eq!(parts[0].kind, "text");
    assert_eq!(parts.len(), 2);
}

#[test]
fn the_reasoning_prompt_inserts_a_system_message_and_consumes_the_flag() {
    let mut params = Object::from_iter([(String::from("_add_reasoning_prompt"), Json::Bool(true))]);
    let updated = add_reasoning_system_prompt_if_needed(
        "magistral-small",
        &[message(json!({"role": "user", "content": "hi"}))],
        &mut params,
    );
    assert_eq!(updated.len(), 2);
    assert_eq!(updated[0].role, crate::chat::Role::System);
    assert!(params.get("_add_reasoning_prompt").is_none());

    let mut params = Object::from_iter([(String::from("_add_reasoning_prompt"), Json::Bool(true))]);
    let updated = add_reasoning_system_prompt_if_needed(
        "mistral-small-latest",
        &[message(json!({"role": "user", "content": "hi"}))],
        &mut params,
    );
    assert_eq!(updated.len(), 1);
    assert_eq!(params.get("_add_reasoning_prompt"), Some(&Json::Bool(true)));
}

#[test]
fn client_metadata_never_reaches_the_body() {
    let body = request_body(
        "mistral-small-latest",
        json!([{"role": "user", "content": "hi"}]),
        json!({"client_metadata": {"trace": 1}, "temperature": 0.2, "cache_control": {"type": "ephemeral"}}),
    );
    assert!(body.get("client_metadata").is_none());
    assert_eq!(body["temperature"], json!(0.2));
    // only messages and tools lose cache_control, so a top-level key reaches the provider
    assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn a_request_without_tools_drops_tool_choice_and_max_retries() {
    let body = request_body(
        "mistral-small-latest",
        json!([{"role": "user", "content": "hi"}]),
        json!({"tool_choice": "any", "max_retries": 3}),
    );
    assert!(body.get("tool_choice").is_none());
    assert!(body.get("max_retries").is_none());
}

#[test]
fn cache_control_leaves_messages_and_tools() {
    let body = request_body(
        "mistral-small-latest",
        json!([{"role": "user", "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}]}]),
        json!({"tools": [{"type": "function", "function": {"name": "f"}, "cache_control": {"type": "ephemeral"}}]}),
    );
    assert!(
        body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none()
    );
    assert!(body["tools"][0].get("cache_control").is_none());
}

#[test]
fn transform_messages_returns_the_conversation_only() {
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let transformed = transform_messages(&MessageTransform {
        model: "mistral-small-latest",
        messages: &messages,
        mode: Mode::Sync,
        downloads: &RefuseDownloads,
    })
    .expect("messages");
    assert_eq!(transformed.len(), 1);
}

#[test]
fn empty_reply_content_becomes_null() {
    let mut reply = json!({"choices": [{"message": {"role": "assistant", "content": ""}}]});
    handle_empty_content_response(&mut reply);
    assert_eq!(reply["choices"][0]["message"]["content"], Value::Null);
}

#[test]
fn the_last_text_and_thinking_block_win() {
    let decoded = decode_mistral_chat(
        json!({"choices": [{"message": {"role": "assistant", "content": [
            {"type": "text", "text": "first"},
            {"type": "thinking", "thinking": [{"type": "text", "text": "one"}, {"type": "text", "text": "two"}]},
            {"type": "text", "text": "second"},
            {"type": "thinking", "thinking": [{"type": "text", "text": "three"}]}
        ]}}]} )
        .to_string()
        .as_bytes(),
    )
    .expect("a reply");
    let message = &decoded.value["choices"][0]["message"];
    assert_eq!(message["content"], json!("second"));
    assert_eq!(message["reasoning_content"], json!("three"));
}

#[test]
fn a_reply_without_thinking_keeps_only_its_text() {
    let decoded = decode_mistral_chat(
        json!({"choices": [{"message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": [{"type": "text", "text": ""}]},
            {"type": "text", "text": "only"}
        ]}}]} )
        .to_string()
        .as_bytes(),
    )
    .expect("a reply");
    assert!(
        decoded.value["choices"][0]["message"]
            .get("reasoning_content")
            .is_none()
    );
    assert_eq!(
        decoded.value["choices"][0]["message"]["content"],
        json!("only")
    );
}

#[test]
fn a_string_reply_content_is_left_alone() {
    let mut reply = json!({"choices": [{"message": {"role": "assistant", "content": "plain"}}]});
    handle_content_list_to_str_conversion(&mut reply).expect("a conversion");
    assert_eq!(reply["choices"][0]["message"]["content"], json!("plain"));
}

#[test]
fn a_malformed_reply_is_not_json() {
    assert!(matches!(
        decode_mistral_chat(b"{"),
        Err(MistralChatError::ReplyJson { .. })
    ));
}

#[test]
fn stream_text_blocks_concatenate_and_thinking_blocks_carry_signatures() {
    let mut chunk = json!({"choices": [{"delta": {"content": [
        {"type": "text", "text": "a"},
        {"type": "thinking", "thinking": [{"type": "text", "text": "one"}, {"type": "text", "text": "two"}]},
        {"type": "text", "text": "b"},
        {"type": "thinking", "thinking": [{"type": "text", "text": "three"}]}
    ]}}]});
    let reply = chunk_parser(&mut chunk, Uuid::nil, i64::default).expect("a chunk");
    let reply = serde_json::to_value(reply).expect("a chunk value");
    let delta = &reply["choices"][0]["delta"];
    assert_eq!(delta["content"], json!("ab"));
    assert_eq!(delta["reasoning_content"], json!("one\nthree"));
    assert_eq!(
        delta["thinking_blocks"],
        json!([{"type": "thinking", "thinking": "one", "signature": "mistral"},
               {"type": "thinking", "thinking": "three", "signature": "mistral"}])
    );
}

#[test]
fn stream_text_without_thinking_removes_stale_fields() {
    let mut chunk = json!({"choices": [{"delta": {
        "content": [{"type": "text", "text": "only"}],
        "thinking_blocks": [{"type": "thinking"}],
        "reasoning_content": "stale"
    }}]});
    let reply = chunk_parser(&mut chunk, Uuid::nil, i64::default).expect("a chunk");
    let reply = serde_json::to_value(reply).expect("a chunk value");
    let delta = &reply["choices"][0]["delta"];
    assert_eq!(delta["content"], json!("only"));
    assert!(delta.get("thinking_blocks").is_none());
    assert!(delta.get("reasoning_content").is_none());
}

#[test]
fn stream_thinking_only_sends_null_content() {
    let mut chunk = json!({"choices": [{"delta": {"content": [
        {"type": "thinking", "thinking": [{"type": "text", "text": "why"}]}
    ]}}]});
    let reply = chunk_parser(&mut chunk, Uuid::nil, i64::default).expect("a chunk");
    let reply = serde_json::to_value(reply).expect("a chunk value");
    assert_eq!(reply["choices"][0]["delta"]["content"], Value::Null);
    assert_eq!(
        reply["choices"][0]["delta"]["reasoning_content"],
        json!("why")
    );
}

#[test]
fn a_malformed_stream_block_falls_back_to_the_parent_parser() {
    let mut chunk = json!({"choices": [{"delta": {"content": [
        {"type": "thinking", "thinking": "not-a-list"}
    ]}}]});
    assert!(matches!(
        chunk_parser(&mut chunk, Uuid::nil, i64::default),
        Err(StreamError::Chunk(_))
    ));
}

#[test]
fn a_stream_error_chunk_reaches_the_parent_after_a_malformed_block() {
    let mut chunk = json!({
        "error": {"message": "upstream failed", "code": 400},
        "choices": [{"delta": {"content": [{"type": "thinking", "thinking": 5}]}}]
    });
    let error = chunk_parser(&mut chunk, Uuid::nil, i64::default).expect_err("an error chunk");
    assert!(matches!(error, StreamError::Provider { status: 400, .. }));
}

#[test]
fn replays_mistral_chat_corpus() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mistral_chat");
    let mut seeds = 0_usize;
    for entry in std::fs::read_dir(&corpus)
        .expect("the corpus folder")
        .flatten()
    {
        let bytes = std::fs::read(entry.path()).expect("a seed");
        let _ = decode_mistral_chat(&bytes);
        if let Ok(mut chunk) = serde_json::from_slice::<Value>(&bytes) {
            let _ = chunk_parser(&mut chunk, Uuid::nil, i64::default);
        }
        seeds += 1;
    }
    assert!(seeds > 0);
}
