use crate::count_tokens::error::CountTokensError;
use crate::count_tokens::validate::validate_count_tokens_request;

use super::support::{MODEL, request};

fn message_error(messages: &str, expected: CountTokensError) {
    let source = format!(r#"{{"messages": {messages}, "model": "{MODEL}"}}"#);
    assert_eq!(
        validate_count_tokens_request(&request(&source)),
        Err(expected)
    );
}

fn model_error(model: &str) {
    let source =
        format!(r#"{{"messages": [{{"role": "user", "content": "hi"}}], "model": {model}}}"#);
    assert_eq!(
        validate_count_tokens_request(&request(&source)),
        Err(CountTokensError::ModelRequired)
    );
}

pub(super) fn count_model_missing() {
    model_error("null");
    model_error("false");
    model_error("\"\"");
    model_error("0");
    model_error("[]");
    model_error("{}");
    let absent = request(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    assert_eq!(
        validate_count_tokens_request(&absent),
        Err(CountTokensError::ModelRequired)
    );
}

pub(super) fn count_messages_empty() {
    message_error("[]", CountTokensError::MessagesRequired);
}

pub(super) fn count_message_not_object() {
    message_error("[42]", CountTokensError::MessageNotObject { index: 0 });
    message_error(
        r#"[{"role": "user", "content": "hi"}, 7]"#,
        CountTokensError::MessageNotObject { index: 1 },
    );
}

fn message_field_error(messages: &str, index: usize, field: &'static str) {
    let source = format!(r#"{{"messages": {messages}, "model": "{MODEL}"}}"#);
    assert_eq!(
        validate_count_tokens_request(&request(&source)),
        Err(CountTokensError::MessageFieldMissing { index, field })
    );
}

pub(super) fn count_message_role_missing() {
    message_field_error(r#"[{"content": "hi"}]"#, 0, "role");
}

pub(super) fn count_message_content_missing() {
    message_field_error(r#"[{"role": "user"}]"#, 0, "content");
    message_field_error(
        r#"[{"role": "user", "content": "hi"}, {"role": "user"}]"#,
        1,
        "content",
    );
    let null_content = request(&format!(
        r#"{{"messages": [{{"role": "user", "content": null}}], "model": "{MODEL}"}}"#
    ));
    assert_eq!(validate_count_tokens_request(&null_content), Ok(()));
}

pub(super) fn count_invoke_model_only() {
    let only_model = request(&format!(r#"{{"model": "{MODEL}"}}"#));
    assert_eq!(
        validate_count_tokens_request(&only_model),
        Err(CountTokensError::ContentRequired)
    );
    let with_extra = request(&format!(r#"{{"model": "{MODEL}", "prompt": null}}"#));
    assert_eq!(validate_count_tokens_request(&with_extra), Ok(()));
}
