#![cfg(test)]

use super::{
    add_guardrail_record, messages_for_call_type, sanitize_guardrail_response,
    summarize_guardrail_response, sync_log_target,
};

const CONTEXT: &str = concat!(
    r#"{"settings": {"guardrail_name": "pii", "event_hook": "pre_call"}, "#,
    r#""log_target": {"litellm_params": {"metadata": {}}, "model_call_params": null}}"#
);

#[test]
fn sanitize_drops_nulls_and_secret_fields() {
    let response = r#"{"match": null, "score": 1.5, "secret_fields": {"a": "b"}}"#;
    let cleaned = sanitize_guardrail_response(response).unwrap();
    assert_eq!(cleaned, r#"{"score":1.5}"#);
}

#[test]
fn records_land_in_the_metadata_bucket() {
    let input = r#"{"guardrail_response": "deny", "guardrail_status": "success"}"#;
    let (data, target) = add_guardrail_record(r#"{"model": "gpt-4o"}"#, input, CONTEXT).unwrap();
    assert!(
        data.contains("\"standard_logging_guardrail_information\""),
        "{data}"
    );
    assert!(data.contains("\"guardrail_name\":\"pii\""), "{data}");
    assert!(data.contains("\"guardrail_response\":\"deny\""), "{data}");
    assert!(target.contains("\"litellm_params\""), "{target}");
    assert!(target.contains("\"model_call_params\":null"), "{target}");
}

#[test]
fn syncing_fills_the_logging_target() {
    let data =
        r#"{"metadata": {"standard_logging_guardrail_information": [{"guardrail_name": "pii"}]}}"#;
    let target = r#"{"litellm_params": {"metadata": {}}, "model_call_params": null}"#;
    let synced = sync_log_target(data, target).unwrap();
    assert!(
        synced.contains("\"standard_logging_guardrail_information\""),
        "{synced}"
    );
    let untouched = sync_log_target(r#"{"model": "gpt-4o"}"#, target).unwrap();
    assert_eq!(untouched, target);
}

#[test]
fn summaries_keep_the_verdict() {
    let body = r#"{"messages": []}"#;
    let changed = r#"{"messages": [{"role": "user"}]}"#;
    assert_eq!(
        summarize_guardrail_response(changed, Some(body), Some("pre_call")).unwrap(),
        "\"mask\""
    );
    assert_eq!(
        summarize_guardrail_response(body, Some(body), Some("pre_call")).unwrap(),
        "\"allow\""
    );
    assert_eq!(
        summarize_guardrail_response(r#"{"text": "deny"}"#, None, None).unwrap(),
        r#"{"text":"deny"}"#
    );
    assert_eq!(
        summarize_guardrail_response("null", None, None).unwrap(),
        "{}"
    );
}

#[test]
fn messages_follow_the_call_type() {
    let chat = r#"{"messages": [{"role": "user", "content": "hi"}]}"#;
    let messages = messages_for_call_type("completion", Some(chat))
        .unwrap()
        .unwrap();
    assert!(messages.contains("\"content\":\"hi\""), "{messages}");
    let responses = messages_for_call_type("responses", Some(r#"{"input": "hi"}"#))
        .unwrap()
        .unwrap();
    assert!(responses.contains("\"role\":\"user\""), "{responses}");
    assert!(
        messages_for_call_type("embedding", Some(chat))
            .unwrap()
            .is_none()
    );
    assert!(
        messages_for_call_type("completion", None)
            .unwrap()
            .is_none()
    );
}
