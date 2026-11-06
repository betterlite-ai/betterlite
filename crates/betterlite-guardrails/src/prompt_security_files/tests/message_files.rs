#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::shared::{
    DONE_ALLOW, UPLOADED, data_url, done, image_item, object, sanitizer, text, vendor,
};
use crate::prompt_security_files::error::FileSanitizationError;
use crate::prompt_security_files::messages::process_message_files;

const REDACTED_BASE64: &str = "UkVEQUNURUQ=";

fn message(content: GuardrailValue) -> GuardrailValue {
    GuardrailValue::Object(object(&[("role", text("user")), ("content", content)]))
}

fn parts(items: Vec<GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Array(items)
}

fn part(entries: BTreeMap<String, GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Object(entries)
}

fn text_part(value: &str) -> GuardrailValue {
    part(object(&[("type", text("text")), ("text", text(value))]))
}

#[tokio::test]
async fn message_files_leave_text_only_conversations_unchanged() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let messages = vec![
        message(text("hello")),
        message(parts(vec![text_part("hi"), GuardrailValue::Null])),
        text("not an object"),
    ];
    let processed = process_message_files(&sanitizer(&server.base), messages.clone(), None)
        .await
        .expect("unchanged");
    assert_eq!(processed, messages);
    assert!(server.requests("POST").is_empty());
}

#[tokio::test]
async fn message_files_sanitize_image_and_document_items_in_order() {
    let server = vendor(UPLOADED, &[done("modify", "REDACTED")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let document = part(object(&[
        ("type", text("file")),
        ("file", part(object(&[("data", text("aGVsbG8="))]))),
    ]));
    let messages = vec![message(parts(vec![
        text_part("look"),
        part(image_item(&data_url("image/png", "pixels"))),
        document,
    ]))];
    let processed = process_message_files(&guardrail, messages, None)
        .await
        .expect("modified");
    let expected = vec![message(parts(vec![
        text_part("look"),
        part(image_item(&format!(
            "data:image/png;base64,{REDACTED_BASE64}"
        ))),
        part(object(&[
            ("type", text("file")),
            ("file", part(object(&[("data", text(REDACTED_BASE64))]))),
        ])),
    ]))];
    assert_eq!(processed, expected);
    assert_eq!(server.requests("POST").len(), 2);
}

#[tokio::test]
async fn message_files_stop_at_the_first_blocked_item() {
    let server = vendor(UPLOADED, &[done("block", "x")]).await;
    let messages = vec![message(parts(vec![
        part(image_item(&data_url("image/png", "one"))),
        part(image_item(&data_url("image/png", "two"))),
    ]))];
    let error = process_message_files(&sanitizer(&server.base), messages, None)
        .await
        .expect_err("blocked");
    assert!(matches!(
        error,
        FileSanitizationError::Blocked {
            resource: "File",
            ..
        }
    ));
    assert_eq!(server.requests("POST").len(), 1);
}

#[tokio::test]
async fn message_files_ignore_items_of_other_types() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let audio = part(object(&[
        ("type", text("input_audio")),
        ("data", text("aGVsbG8=")),
    ]));
    let untyped = part(object(&[("data", text("aGVsbG8="))]));
    let messages = vec![message(parts(vec![audio, untyped]))];
    let processed = process_message_files(&sanitizer(&server.base), messages.clone(), None)
        .await
        .expect("unchanged");
    assert_eq!(processed, messages);
    assert!(server.requests("POST").is_empty());
}
