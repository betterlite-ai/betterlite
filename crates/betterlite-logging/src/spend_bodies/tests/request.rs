use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{capture_logs, policy, redacting, value};
use crate::spend_bodies::request::{RequestKwargs, storage_request_body};
use crate::spend_sanitize::REDACTED_BY_LITELM;

fn stored(litellm_params: &Json, kwargs: Option<&RequestKwargs<'_>>, redact: bool) -> String {
    let policy = if redact {
        redacting(policy(true, 2048))
    } else {
        policy(true, 2048)
    };
    storage_request_body(litellm_params, kwargs, policy)
}

#[test]
fn stores_the_request_body_and_never_its_headers() {
    let params = value(
        r#"{"proxy_server_request": {"body": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}, "headers": {"authorization": "Bearer secret"}}}"#,
    );
    let stored = stored(&params, None, false);
    assert_eq!(
        stored,
        r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}"#
    );
    assert!(!stored.contains("authorization") && !stored.contains("Bearer"));
}

#[test]
fn disabled_storage_returns_an_empty_object() {
    let params = value(r#"{"proxy_server_request": {"body": {"model": "gpt-4o"}}}"#);
    assert_eq!(
        storage_request_body(&params, None, policy(false, 2048)),
        "{}"
    );
}

#[test]
fn a_null_or_missing_proxy_server_request_returns_an_empty_object() {
    let params = value(r#"{"proxy_server_request": null}"#);
    assert_eq!(stored(&params, None, false), "{}");
    let without_proxy = value(r#"{"model": "gpt-4o"}"#);
    assert_eq!(stored(&without_proxy, None, false), "{}");
}

#[test]
fn overlays_and_redacts_without_a_proxy_server_request() {
    let params = value(r#"{"model": "gpt-4o"}"#);
    let tools = value(r#"[{"type": "function"}]"#);
    let kwargs = RequestKwargs {
        standard_logging_object: None,
        realtime_tools: Some(&tools),
        dynamic_params: None,
    };
    assert_eq!(
        stored(&params, Some(&kwargs), false),
        r#"{"tools": [{"type": "function"}]}"#
    );
    assert_eq!(
        stored(&params, Some(&kwargs), true),
        r#"{"tools": [{"type": "function"}], "messages": [{"role": "user", "content": "redacted-by-litellm"}], "prompt": "", "input": ""}"#
    );
}

#[test]
fn overlays_classifier_audit_fields_after_the_body() {
    let params = value(r#"{"proxy_server_request": {"body": {"model": "gpt-4o"}}}"#);
    let payload = value(
        r#"{"classifier_input": {"prompt": "pick"}, "originating_request_masked": {"messages": []}}"#,
    );
    let kwargs = RequestKwargs {
        standard_logging_object: Some(&payload),
        realtime_tools: None,
        dynamic_params: None,
    };
    assert_eq!(
        stored(&params, Some(&kwargs), false),
        r#"{"model": "gpt-4o", "classifier_input": {"prompt": "pick"}, "originating_request_masked": {"messages": []}}"#
    );
}

#[test]
fn overlays_truthy_realtime_tools_only() {
    let params = value(r#"{"proxy_server_request": {"body": {"model": "gpt-4o"}}}"#);
    let tools = value(r#"[{"type": "function", "function": {"name": "f"}}]"#);
    let kwargs = RequestKwargs {
        standard_logging_object: None,
        realtime_tools: Some(&tools),
        dynamic_params: None,
    };
    assert_eq!(
        stored(&params, Some(&kwargs), false),
        r#"{"model": "gpt-4o", "tools": [{"type": "function", "function": {"name": "f"}}]}"#
    );
    let empty = value("[]");
    let empty_kwargs = RequestKwargs {
        standard_logging_object: None,
        realtime_tools: Some(&empty),
        dynamic_params: None,
    };
    assert_eq!(
        stored(&params, Some(&empty_kwargs), false),
        r#"{"model": "gpt-4o"}"#
    );
}

#[test]
fn redaction_replaces_messages_and_drops_classifier_audit_fields() {
    let params = value(
        r#"{"proxy_server_request": {"body": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}}}"#,
    );
    let payload = value(r#"{"classifier_input": {"prompt": "pick"}}"#);
    let kwargs = RequestKwargs {
        standard_logging_object: Some(&payload),
        realtime_tools: None,
        dynamic_params: None,
    };
    assert_eq!(
        stored(&params, Some(&kwargs), true),
        r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "redacted-by-litellm"}], "prompt": "", "input": ""}"#
    );
}

#[rstest::rstest]
#[case::global(true, None, None, true)]
#[case::enable_header(
    false,
    Some(r#"{"litellm-enable-message-redaction": "true"}"#),
    None,
    true
)]
#[case::dynamic_true(
    false,
    Some(r#"{"litellm-enable-message-redaction": true}"#),
    Some(r#"{"turn_off_message_logging": true}"#),
    true
)]
#[case::dynamic_false_over_enable(
    false,
    Some(r#"{"litellm-enable-message-redaction": true}"#),
    Some(r#"{"turn_off_message_logging": false}"#),
    false
)]
#[case::disable_header(
    true,
    Some(r#"{"litellm-disable-message-redaction": true}"#),
    None,
    false
)]
#[case::plain_false_header(
    true,
    Some(r#"{"litellm-disable-message-redaction": false}"#),
    None,
    true
)]
#[case::nothing_set(false, None, None, false)]
fn redaction_follows_global_header_and_dynamic_policy(
    #[case] global: bool,
    #[case] headers: Option<&str>,
    #[case] dynamic: Option<&str>,
    #[case] expected: bool,
) {
    let mut litellm = IndexMap::new();
    litellm.insert(
        "proxy_server_request".to_owned(),
        value(r#"{"body": {"messages": [{"role": "user", "content": "hi"}]}}"#),
    );
    if let Some(header_map) = headers {
        let mut entries = IndexMap::new();
        entries.insert("headers".to_owned(), value(header_map));
        litellm.insert("metadata".to_owned(), Json::Object(entries));
    }
    let params = Json::Object(litellm);
    let dynamic_param = dynamic.map(value);
    let kwargs = RequestKwargs {
        standard_logging_object: None,
        realtime_tools: None,
        dynamic_params: dynamic_param.as_ref(),
    };
    let result = stored(&params, Some(&kwargs), global);
    assert_eq!(result.contains("redacted-by-litellm"), expected);
}

#[test]
fn truncates_long_strings_and_logs_the_safeguard_note() {
    let params = value(
        r#"{"proxy_server_request": {"body": {"messages": [{"role": "user", "content": "abcdefghijklmnop"}]}}}"#,
    );
    let (logs, guard) = capture_logs();
    let stored = storage_request_body(&params, None, policy(true, 10));
    drop(guard);
    assert!(stored.contains("litellm_truncated skipped"));
    assert!(logs.text().contains("request body was truncated"));
}

#[test]
fn a_preexisting_marker_text_also_logs_the_safeguard_note() {
    let params = value(
        r#"{"proxy_server_request": {"body": {"messages": "litellm_truncated was sent by the caller"}}}"#,
    );
    let (logs, guard) = capture_logs();
    let stored = storage_request_body(&params, None, policy(true, 2048));
    drop(guard);
    assert_eq!(
        stored,
        r#"{"messages": "litellm_truncated was sent by the caller"}"#
    );
    assert!(logs.text().contains("request body was truncated"));
}

#[test]
fn masks_credentials_and_strips_secret_fields() {
    let params = value(
        r#"{"proxy_server_request": {"body": {"apikey": "sk-secret", "secret_fields": {"authorization": "Bearer x"}}}}"#,
    );
    let stored = stored(&params, None, false);
    assert!(stored.contains(REDACTED_BY_LITELM));
    assert!(!stored.contains("sk-secret") && !stored.contains("secret_fields"));
}
