#![cfg(test)]

use serde_json::{Value, json};

use super::support::{Fixture, messages};
use crate::gigachat_chat::messages::transform_messages;

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

async fn transformed(input: &str, fixture: &Fixture) -> Vec<Value> {
    let turned = transform_messages(&messages(input), &fixture.uploader)
        .await
        .unwrap();
    turned.iter().map(|message| json!(message)).collect()
}

#[tokio::test]
async fn a_name_is_dropped_from_every_message() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "user", "content": "hi", "name": "bob"}]"#,
        &fixture,
    )
    .await;
    assert_eq!(turned, vec![value(r#"{"role": "user", "content": "hi"}"#)]);
}

#[tokio::test]
async fn a_developer_message_becomes_a_system_message() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(r#"[{"role": "developer", "content": "rules"}]"#, &fixture).await;
    assert_eq!(
        turned,
        vec![value(r#"{"role": "system", "content": "rules"}"#)]
    );
}

#[tokio::test]
async fn a_later_developer_message_still_becomes_a_system_message() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "user", "content": "hi"}, {"role": "developer", "content": "rules"}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned[1],
        value(r#"{"role": "system", "content": "rules"}"#)
    );
}

#[tokio::test]
async fn the_first_system_message_stays_a_system_message() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(r#"[{"role": "system", "content": "rules"}]"#, &fixture).await;
    assert_eq!(
        turned,
        vec![value(r#"{"role": "system", "content": "rules"}"#)]
    );
}

#[tokio::test]
async fn a_system_message_after_the_first_becomes_a_user_message() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "user", "content": "hi"}, {"role": "system", "content": "rules"}]"#,
        &fixture,
    )
    .await;
    assert_eq!(turned[1], value(r#"{"role": "user", "content": "rules"}"#));
}

#[tokio::test]
async fn a_tool_message_becomes_a_function_message_with_quoted_text() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "tool", "content": "prague", "tool_call_id": "call_1"}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "function", "content": "\"prague\"", "tool_call_id": "call_1"}"#
        )]
    );
}

#[tokio::test]
async fn tool_content_that_is_already_json_is_kept() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "tool", "content": "{\"city\": \"prague\"}"}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "function", "content": "{\"city\": \"prague\"}"}"#
        )]
    );
}

#[tokio::test]
async fn a_tool_message_without_content_gets_an_empty_json_string() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(r#"[{"role": "tool"}]"#, &fixture).await;
    assert_eq!(
        turned,
        vec![value(r#"{"role": "function", "content": "\"\""}"#)]
    );
}

#[tokio::test]
async fn a_tool_message_with_a_list_content_encodes_the_whole_list() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "tool", "content": [{"type": "text", "text": "x"}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(turned[0]["role"], "function");
    let encoded = value(turned[0]["content"].as_str().unwrap());
    assert_eq!(encoded, value(r#"[{"type": "text", "text": "x"}]"#));
    assert!(fixture.uploader.uploads().is_empty());
}

#[tokio::test]
async fn a_null_content_becomes_an_empty_string() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(r#"[{"role": "user", "content": null}]"#, &fixture).await;
    assert_eq!(turned, vec![value(r#"{"role": "user", "content": ""}"#)]);
}

#[tokio::test]
async fn list_content_joins_the_text_parts_with_newlines() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "user", "content": [{"type": "text", "text": "one"}, {"type": "text", "text": "two"}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(r#"{"role": "user", "content": "one\ntwo"}"#)]
    );
}

#[tokio::test]
async fn image_parts_are_uploaded_and_become_attachments() {
    let fixture = Fixture::issuing(
        "token",
        &[
            ("https://images.example/one.png", "file-1"),
            ("data:image/png;base64,AAAA", "file-2"),
        ],
    );
    let turned = transformed(
        r#"[{"role": "user", "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": {"url": "https://images.example/one.png"}}, {"type": "image_url", "image_url": "data:image/png;base64,AAAA"}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "user", "content": "look", "attachments": ["file-1", "file-2"]}"#
        )]
    );
    assert_eq!(
        fixture.uploader.uploads(),
        vec![
            String::from("https://images.example/one.png"),
            String::from("data:image/png;base64,AAAA"),
        ]
    );
}

#[tokio::test]
async fn an_image_url_without_a_name_is_not_uploaded() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": ""}}, {"type": "text", "text": "x"}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(turned, vec![value(r#"{"role": "user", "content": "x"}"#)]);
    assert!(fixture.uploader.uploads().is_empty());
}

#[tokio::test]
async fn an_image_that_fails_to_upload_keeps_no_attachment() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://images.example/missing.png"}}, {"type": "text", "text": "x"}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(turned, vec![value(r#"{"role": "user", "content": "x"}"#)]);
    assert_eq!(
        fixture.uploader.uploads(),
        vec![String::from("https://images.example/missing.png")]
    );
}

#[tokio::test]
async fn a_text_part_of_the_wrong_shape_is_rejected() {
    let fixture = Fixture::issuing("token", &[]);
    let error = transform_messages(
        &messages(r#"[{"role": "user", "content": [{"type": "text", "text": 5}]}]"#),
        &fixture.uploader,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code, 400);
    assert!(error.source.is_some());
}

#[tokio::test]
async fn the_first_tool_call_becomes_a_function_call_with_parsed_arguments() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{\"city\": \"prague\"}"}}, {"id": "call_2", "type": "function", "function": {"name": "later", "arguments": "{}"}}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "assistant", "content": "", "function_call": {"name": "lookup", "arguments": {"city": "prague"}}}"#
        )]
    );
}

#[tokio::test]
async fn tool_arguments_that_are_not_json_become_an_empty_object() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "not json"}}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "assistant", "content": "", "function_call": {"name": "lookup", "arguments": {}}}"#
        )]
    );
}

async fn wire_text(input: &str, fixture: &Fixture) -> String {
    let turned = transform_messages(&messages(input), &fixture.uploader)
        .await
        .unwrap();
    serde_json::to_string(&turned[0]).unwrap()
}

#[tokio::test]
async fn a_generated_function_call_replaces_the_one_the_client_sent() {
    let fixture = Fixture::issuing("token", &[]);
    let text = wire_text(
        r#"[{"role": "assistant", "function_call": {"name": "old", "arguments": "{}"}, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(text.matches("\"function_call\"").count(), 1);
    assert_eq!(
        value(&text),
        value(
            r#"{"role": "assistant", "content": "", "function_call": {"name": "lookup", "arguments": {}}}"#
        )
    );
}

#[tokio::test]
async fn a_function_call_from_the_client_is_kept_without_tool_calls() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "assistant", "function_call": {"name": "old", "arguments": "{}"}}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "assistant", "content": "", "function_call": {"name": "old", "arguments": "{}"}}"#
        )]
    );
}

#[tokio::test]
async fn generated_attachments_replace_the_ones_the_client_sent() {
    let fixture = Fixture::issuing("token", &[("https://images.example/one.png", "file-1")]);
    let text = wire_text(
        r#"[{"role": "user", "attachments": ["old"], "content": [{"type": "image_url", "image_url": {"url": "https://images.example/one.png"}}]}]"#,
        &fixture,
    )
    .await;
    assert_eq!(text.matches("\"attachments\"").count(), 1);
    assert_eq!(
        value(&text),
        value(r#"{"role": "user", "content": "", "attachments": ["file-1"]}"#)
    );
}

#[tokio::test]
async fn attachments_from_the_client_are_kept_when_no_image_uploads() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(
        r#"[{"role": "user", "attachments": ["old"], "content": "hi"}]"#,
        &fixture,
    )
    .await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "user", "content": "hi", "attachments": ["old"]}"#
        )]
    );
}

#[tokio::test]
async fn an_empty_tool_calls_list_stays_as_sent() {
    let fixture = Fixture::issuing("token", &[]);
    let turned = transformed(r#"[{"role": "assistant", "tool_calls": []}]"#, &fixture).await;
    assert_eq!(
        turned,
        vec![value(
            r#"{"role": "assistant", "content": "", "tool_calls": []}"#
        )]
    );
}
