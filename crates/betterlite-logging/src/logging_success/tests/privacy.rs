use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, bare_request, chunk_result, correlation, json_object, parse,
    payload, request, run_once, state,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_success::payload_state::standard_logging_object;
use crate::logging_success::types::SuccessResult;

pub(super) const SECRET_MESSAGES: &str = r#"[{"role": "system", "content": "secret prompt"}]"#;
pub(super) const SECRET_RESPONSE: &str = r#"{"id": "r1", "choices": [{"message": {"role": "assistant", "content": "secret answer", "audio": {"id": "a"}, "tool_calls": [{"function": {"name": "f", "arguments": "{\"k\": 1}"}}]}}]}"#;
pub(super) const REDACTED_MESSAGES: &str =
    r#"[{"role": "user", "content": "redacted-by-litellm"}]"#;
pub(super) const REDACTED_RESPONSE: &str = r#"{"id": "r1", "choices": [{"message": {"role": "assistant", "content": "redacted-by-litellm", "audio": null, "tool_calls": [{"function": {"name": "f", "arguments": "redacted-by-litellm"}}]}}]}"#;
const ENABLE: &str = "litellm-enable-message-redaction";
const X_ENABLE: &str = "x-litellm-enable-message-redaction";
const DISABLE: &str = "litellm-disable-message-redaction";

pub(super) fn secret_payload() -> StandardLoggingPayload {
    let mut secret = payload();
    secret.messages = Some(parse(SECRET_MESSAGES));
    secret.response = Some(parse(SECRET_RESPONSE));
    secret
}

// mode "built" is an async stream chunk without a stored payload, so the handler builds one
#[rstest]
#[case::sync_plain("sync", None, "", false, false)]
#[case::async_plain("async", None, "", false, false)]
#[case::sync_dynamic_flag("sync", Some(true), "", false, true)]
#[case::async_dynamic_flag("async", Some(true), "", false, true)]
#[case::sync_dynamic_off("sync", Some(false), "", true, false)]
#[case::async_dynamic_off("async", Some(false), "", true, false)]
#[case::sync_global("sync", None, "", true, true)]
#[case::async_global("async", None, "", true, true)]
#[case::sync_enable_header("sync", None, ENABLE, false, true)]
#[case::async_x_enable_header("async", None, X_ENABLE, false, true)]
#[case::sync_disable_header("sync", Some(true), DISABLE, true, false)]
#[case::async_disable_header("async", None, DISABLE, true, false)]
#[case::sync_dynamic_beats_enable_header("sync", Some(false), ENABLE, false, false)]
#[case::built_chunk_payload("built", Some(true), "", false, true)]
#[case::built_chunk_unredacted("built", None, "", false, false)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(
    #[case] mode: &str,
    #[case] dynamic: Option<bool>,
    #[case] header: &str,
    #[case] global: bool,
    #[case] redacted: bool,
) {
    let built = mode == "built";
    let path = if mode == "sync" { "sync" } else { "async" };
    let mut fixture = Fixture::new();
    fixture.privacy.turn_off_message_logging = global;
    fixture.builder.payload = Some(secret_payload());
    let calls = Arc::new(CallLog::default());
    fixture.add(&Arc::new(FakeLogger::new("langfuse", &calls)));
    let mut state = state(Some(built));
    if let Some(flag) = dynamic {
        state
            .callback_dynamic_params
            .insert("turn_off_message_logging".to_owned(), Json::Bool(flag));
    }
    if !header.is_empty() {
        let flags = json_object(&[(header, Json::Text("true".to_owned()))]);
        let metadata = json_object(&[("headers", flags)]);
        state.litellm_params.insert("metadata".to_owned(), metadata);
    }
    let call = if built {
        bare_request(chunk_result())
    } else {
        let mut call = request(SuccessResult::Chat(json_object(&[(
            "id",
            Json::Text("r1".to_owned()),
        )])));
        call.standard_logging_object = Some(secret_payload());
        call
    };
    let mut correlation = correlation();
    run_once(
        path,
        &fixture.handler(),
        &mut state,
        &mut correlation,
        &call,
    )
    .await;
    let (messages, response) = if redacted {
        (REDACTED_MESSAGES, REDACTED_RESPONSE)
    } else {
        (SECRET_MESSAGES, SECRET_RESPONSE)
    };
    let logged = calls.calls();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].messages, Some(parse(messages)));
    assert_eq!(logged[0].response, Some(parse(response)));
    if !built {
        let stored = standard_logging_object(&state).unwrap().unwrap();
        assert_eq!(stored.messages, Some(parse(messages)));
        assert_eq!(stored.response, Some(parse(response)));
    }
}
