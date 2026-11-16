use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;
use tracing::Level;
use tracing::subscriber::set_default;

use super::support::{
    ANONYMIZING_ASSISTANT_REPLY, ANONYMIZING_USER_REPLY, BLOCKING_REPLY, CapturedLog,
    MODEL_RESPONSE, SAFE_REPLY, USER_MESSAGES, build, data, field, hooked, mock_noma, moderation,
    params, post_call, pre_call, records, request_count, scanned_text, value,
};
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::noma_lifecycle::error::NomaLifecycleError;
use crate::noma_scan_policy::error::{NomaBlockedMessage, NomaError};

fn text(found: &str) -> GuardrailValue {
    GuardrailValue::Text(found.to_owned())
}

#[tokio::test]
async fn pre_call_scans_the_user_message_and_lets_a_safe_one_through() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let original = request_data.clone();
    pre_call(&guardrail, &mut request_data, "completion")
        .await
        .unwrap();
    assert_eq!(request_count(&mock), 1);
    assert!(scanned_text(&mock, 0).contains("hello"));
    assert_eq!(request_data.get("messages"), original.get("messages"));
    let written = records(&request_data);
    assert_eq!(written.len(), 1);
    assert_eq!(
        field(&written[0], "guardrail_provider").cloned(),
        Some(text("noma"))
    );
    assert_eq!(
        field(&written[0], "guardrail_mode").cloned(),
        Some(text("pre_call"))
    );
    assert_eq!(
        field(&written[0], "guardrail_status").cloned(),
        Some(text("success"))
    );
}

#[tokio::test]
async fn pre_call_without_messages_scans_nothing() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(r#"{"messages": []}"#);
    pre_call(&guardrail, &mut request_data, "completion")
        .await
        .unwrap();
    assert_eq!(request_count(&mock), 0);
}

#[tokio::test]
async fn pre_call_re_raises_a_block_as_a_400() {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let error = pre_call(&guardrail, &mut request_data, "completion")
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert!(matches!(
        error,
        NomaLifecycleError::Noma(NomaError::Blocked(_))
    ));
    let written = records(&request_data);
    assert_eq!(written.len(), 1);
    assert_eq!(
        field(&written[0], "guardrail_status").cloned(),
        Some(text("guardrail_intervened"))
    );
}

#[tokio::test]
async fn pre_call_keeps_raising_a_block_when_failures_are_allowed() {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(false);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let error = pre_call(&guardrail, &mut request_data, "completion")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        NomaLifecycleError::Noma(NomaError::Blocked(_))
    ));
}

#[tokio::test]
async fn pre_call_anonymizes_the_last_user_message() {
    let mock = mock_noma(StatusCode::OK, ANONYMIZING_USER_REPLY).await;
    let mut configured = params(&mock.base);
    configured.anonymize_input = Some(true);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    pre_call(&guardrail, &mut request_data, "completion")
        .await
        .unwrap();
    let expected = data(r#"{"messages": [{"role": "user", "content": "clean"}]}"#);
    assert_eq!(request_data.get("messages"), expected.get("messages"));
}

#[rstest]
#[case::named_hook("pre_call", "completion", 1)]
#[case::other_hook("during_call", "completion", 0)]
#[case::post_call_hook("post_call", "completion", 0)]
#[case::mcp_call_needs_the_mcp_hook("pre_call", "call_mcp_tool", 0)]
#[case::mcp_call_with_the_mcp_hook("pre_mcp_call", "call_mcp_tool", 1)]
#[case::mcp_hook_ignores_other_calls("pre_mcp_call", "completion", 0)]
#[tokio::test]
async fn pre_call_runs_only_for_its_event_hook(
    #[case] hook: &str,
    #[case] call_type: &str,
    #[case] scans: usize,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&hooked(&mock.base, hook));
    let mut request_data = data(USER_MESSAGES);
    pre_call(&guardrail, &mut request_data, call_type)
        .await
        .unwrap();
    assert_eq!(request_count(&mock), scans);
}

#[rstest]
#[case::named_hook("during_call", "completion", 1)]
#[case::other_hook("pre_call", "completion", 0)]
#[case::mcp_call_needs_the_mcp_hook("during_call", "call_mcp_tool", 0)]
#[case::mcp_call_with_the_mcp_hook("pre_mcp_call", "call_mcp_tool", 1)]
#[case::mcp_hook_ignores_other_calls("pre_mcp_call", "completion", 0)]
#[tokio::test]
async fn moderation_runs_only_for_its_event_hook(
    #[case] hook: &str,
    #[case] call_type: &str,
    #[case] scans: usize,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&hooked(&mock.base, hook));
    let mut request_data = data(USER_MESSAGES);
    moderation(&guardrail, &mut request_data, call_type)
        .await
        .unwrap();
    assert_eq!(request_count(&mock), scans);
}

#[tokio::test]
async fn a_scan_event_stays_the_hooks_own_even_for_an_mcp_call() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&hooked(&mock.base, "pre_mcp_call"));
    let mut pre_data = data(USER_MESSAGES);
    pre_call(&guardrail, &mut pre_data, "call_mcp_tool")
        .await
        .unwrap();
    let mut during_data = data(USER_MESSAGES);
    moderation(&guardrail, &mut during_data, "call_mcp_tool")
        .await
        .unwrap();
    assert_eq!(
        field(&records(&pre_data)[0], "guardrail_mode").cloned(),
        Some(text("pre_call"))
    );
    assert_eq!(
        field(&records(&during_data)[0], "guardrail_mode").cloned(),
        Some(text("during_call"))
    );
}

#[tokio::test]
async fn moderation_blocks_and_scans_in_the_during_call_event() {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let error = moderation(&guardrail, &mut request_data, "completion")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        NomaLifecycleError::Noma(NomaError::Blocked(_))
    ));
    assert_eq!(
        field(&records(&request_data)[0], "guardrail_mode").cloned(),
        Some(text("during_call"))
    );
}

#[rstest]
#[case::pre_call(true, "pre_call")]
#[case::moderation(false, "during_call")]
#[tokio::test]
async fn a_failed_scan_is_recorded_and_raised_when_failures_block(
    #[case] pre: bool,
    #[case] mode: &str,
) {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let failed = if pre {
        pre_call(&guardrail, &mut request_data, "completion").await
    } else {
        moderation(&guardrail, &mut request_data, "completion").await
    };
    let error = failed.unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    let written = records(&request_data);
    assert_eq!(written.len(), 1);
    assert_eq!(
        field(&written[0], "guardrail_mode").cloned(),
        Some(text(mode))
    );
    assert_eq!(
        field(&written[0], "guardrail_status").cloned(),
        Some(text("guardrail_failed_to_respond"))
    );
    assert_eq!(
        field(&written[0], "guardrail_provider").cloned(),
        Some(text("noma"))
    );
    assert_eq!(
        field(&written[0], "guardrail_response").cloned(),
        Some(text(&error.to_string()))
    );
    let stamp = Some(GuardrailValue::Decimal(1_700_000_000.0));
    assert_eq!(field(&written[0], "start_time").cloned(), stamp);
    assert_eq!(field(&written[0], "end_time").cloned(), stamp);
    assert_eq!(
        field(&written[0], "duration").cloned(),
        Some(GuardrailValue::Decimal(0.0))
    );
}

#[rstest]
#[case::pre_call(true)]
#[case::moderation(false)]
#[tokio::test]
async fn a_failed_scan_is_recorded_and_allowed_when_failures_do_not_block(#[case] pre: bool) {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(false);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    if pre {
        pre_call(&guardrail, &mut request_data, "completion")
            .await
            .unwrap();
    } else {
        moderation(&guardrail, &mut request_data, "completion")
            .await
            .unwrap();
    }
    let written = records(&request_data);
    assert_eq!(written.len(), 1);
    assert_eq!(
        field(&written[0], "guardrail_status").cloned(),
        Some(text("guardrail_failed_to_respond"))
    );
}

#[tokio::test]
async fn post_call_scans_the_response_content_and_keeps_a_safe_response() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let mut response = value(MODEL_RESPONSE);
    post_call(&guardrail, &mut request_data, &mut response)
        .await
        .unwrap();
    assert_eq!(request_count(&mock), 1);
    assert_eq!(scanned_text(&mock, 0), "hello");
    assert_eq!(response, value(MODEL_RESPONSE));
    let written = records(&request_data);
    assert_eq!(
        field(&written[0], "guardrail_mode").cloned(),
        Some(text("post_call"))
    );
}

#[tokio::test]
async fn post_call_returns_the_anonymized_response() {
    let mock = mock_noma(StatusCode::OK, ANONYMIZING_ASSISTANT_REPLY).await;
    let mut configured = params(&mock.base);
    configured.anonymize_input = Some(true);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let mut response = value(MODEL_RESPONSE);
    post_call(&guardrail, &mut request_data, &mut response)
        .await
        .unwrap();
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["choices"][0]["message"]["content"], "clean");
}

#[tokio::test]
async fn post_call_re_raises_a_block_and_keeps_the_response() {
    let mock = mock_noma(
        StatusCode::OK,
        r#"{"aggregatedScanResult": true, "scanResult": [{"role": "assistant", "results": {"harmfulContent": {"result": true}}}]}"#,
    )
    .await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(false);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let mut response = value(MODEL_RESPONSE);
    let error = post_call(&guardrail, &mut request_data, &mut response)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        NomaLifecycleError::Noma(NomaError::Blocked(_))
    ));
    assert_eq!(response, value(MODEL_RESPONSE));
}

#[rstest]
#[case::blocking(true)]
#[case::allowing(false)]
#[tokio::test]
async fn post_call_failures_follow_block_failures(#[case] block_failures: bool) {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(block_failures);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let mut response = value(MODEL_RESPONSE);
    let outcome = post_call(&guardrail, &mut request_data, &mut response).await;
    assert_eq!(outcome.is_err(), block_failures);
    assert_eq!(response, value(MODEL_RESPONSE));
    let written = records(&request_data);
    assert_eq!(written.len(), 1);
    assert_eq!(
        field(&written[0], "guardrail_mode").cloned(),
        Some(text("post_call"))
    );
    assert_eq!(
        field(&written[0], "guardrail_status").cloned(),
        Some(text("guardrail_failed_to_respond"))
    );
}

#[tokio::test]
async fn a_failed_hook_logs_the_hook_it_failed_in() {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(false);
    let guardrail = build(&configured);
    let log = CapturedLog::default();
    let _guard = set_default(log.subscriber(Level::ERROR));
    let mut request_data = data(USER_MESSAGES);
    pre_call(&guardrail, &mut request_data, "completion")
        .await
        .unwrap();
    assert!(log.text().contains("Noma pre-call hook failed"));
    moderation(&guardrail, &mut request_data, "completion")
        .await
        .unwrap();
    assert!(log.text().contains("Noma moderation hook failed"));
    let mut response = value(MODEL_RESPONSE);
    post_call(&guardrail, &mut request_data, &mut response)
        .await
        .unwrap();
    assert!(log.text().contains("Noma post-call hook failed"));
}

#[rstest]
#[case::named_hook("post_call", 1)]
#[case::other_hook("pre_call", 0)]
#[tokio::test]
async fn post_call_runs_only_for_its_event_hook(#[case] hook: &str, #[case] scans: usize) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&hooked(&mock.base, hook));
    let mut request_data = data(USER_MESSAGES);
    let mut response = value(MODEL_RESPONSE);
    post_call(&guardrail, &mut request_data, &mut response)
        .await
        .unwrap();
    assert_eq!(request_count(&mock), scans);
}

#[rstest]
#[case::block(
    NomaLifecycleError::Noma(NomaError::Blocked(NomaBlockedMessage {
        details: GuardrailValue::Null,
    })),
    StatusCode::BAD_REQUEST
)]
#[case::other_noma_failure(
    NomaLifecycleError::Noma(NomaError::Field { field: "messages" }),
    StatusCode::INTERNAL_SERVER_ERROR
)]
#[case::bad_request_selection(
    NomaLifecycleError::Selection(GuardrailSelectionError::RequestedNotList),
    StatusCode::BAD_REQUEST
)]
#[case::tag_mode_selection(
    NomaLifecycleError::Selection(GuardrailSelectionError::TagMode { guardrail_name: None }),
    StatusCode::INTERNAL_SERVER_ERROR
)]
fn status_code_follows_litellm(#[case] error: NomaLifecycleError, #[case] expected: StatusCode) {
    assert_eq!(error.status_code(), expected);
}
