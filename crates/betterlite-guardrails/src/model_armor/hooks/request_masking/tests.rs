#![cfg(test)]

use rstest::rstest;

use super::super::error_handling::FieldError;
use super::super::tests::support::{data, prompt_data, value};
use super::{apply, read_messages};

#[rstest]
#[case::absent("{}", None)]
#[case::null(r#"{"messages":null}"#, None)]
#[case::empty(r#"{"messages":[]}"#, None)]
#[case::one(r#"{"messages":[{"role":"user","content":"hi"}]}"#, Some(1))]
#[case::two(
    r#"{"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":"yo"}]}"#,
    Some(2)
)]
fn request_masking_reads_the_messages(#[case] source: &str, #[case] expected: Option<usize>) {
    let messages = read_messages(&data(source)).expect("messages");
    assert_eq!(messages.map(|messages| messages.len()), expected);
}

#[test]
fn request_masking_rejects_a_messages_value_that_is_not_a_list() {
    let error = read_messages(&data(r#"{"messages":"hello"}"#)).expect_err("not a list");
    assert_eq!(
        error,
        FieldError {
            field: "messages",
            expected: "a list of messages with a string role",
        }
    );
}

#[test]
fn request_masking_rejects_a_message_shape_it_cannot_read() {
    let error = read_messages(&data(r#"{"messages":[{"role":5}]}"#)).expect_err("not a message");
    assert_eq!(
        error,
        FieldError {
            field: "messages",
            expected: "a list of messages with a string role",
        }
    );
}

#[test]
fn request_masking_applies_the_sanitized_text_to_the_last_user_message() {
    let mut request = prompt_data();
    let messages = read_messages(&request).expect("messages").expect("some");
    apply(
        &mut request,
        &messages,
        "hello",
        Some("masked text".to_owned()),
    )
    .expect("apply");
    assert_eq!(
        request.get("messages"),
        Some(&value(r#"[{"role":"user","content":"masked text"}]"#))
    );
}

#[test]
fn request_masking_keeps_the_messages_before_the_trailing_user_run() {
    let mut request = data(
        r#"{"messages":[{"role":"system","content":"rules"},{"role":"user","content":"first"},{"role":"user","content":"hello"}]}"#,
    );
    let messages = read_messages(&request).expect("messages").expect("some");
    apply(
        &mut request,
        &messages,
        "first\nhello",
        Some("masked".to_owned()),
    )
    .expect("apply");
    assert_eq!(
        request.get("messages"),
        Some(&value(
            r#"[{"role":"system","content":"rules"},{"role":"user","content":"masked"}]"#
        ))
    );
}

#[test]
fn request_masking_keeps_the_provider_fields_of_earlier_messages() {
    let mut request = data(
        r#"{"messages":[{"role":"assistant","content":"earlier","cache_control":{"type":"ephemeral"}},{"role":"user","content":"hello"}]}"#,
    );
    let messages = read_messages(&request).expect("messages").expect("some");
    apply(&mut request, &messages, "hello", Some("masked".to_owned())).expect("apply");
    assert_eq!(
        request.get("messages"),
        Some(&value(
            r#"[{"role":"assistant","content":"earlier","cache_control":{"type":"ephemeral"}},{"role":"user","content":"masked"}]"#
        ))
    );
}

#[test]
fn request_masking_skips_an_identical_text() {
    let mut request = prompt_data();
    let before = request.clone();
    let messages = read_messages(&request).expect("messages").expect("some");
    apply(
        &mut request,
        &messages,
        "scanned",
        Some("scanned".to_owned()),
    )
    .expect("apply");
    assert_eq!(request, before);
}

#[test]
fn request_masking_skips_an_absent_sanitized_text() {
    let mut request = prompt_data();
    let before = request.clone();
    let messages = read_messages(&request).expect("messages").expect("some");
    apply(&mut request, &messages, "hello", None).expect("apply");
    assert_eq!(request, before);
}

#[test]
fn request_masking_skips_an_empty_sanitized_text() {
    let mut request = prompt_data();
    let before = request.clone();
    let messages = read_messages(&request).expect("messages").expect("some");
    apply(&mut request, &messages, "hello", Some(String::new())).expect("apply");
    assert_eq!(request, before);
}
