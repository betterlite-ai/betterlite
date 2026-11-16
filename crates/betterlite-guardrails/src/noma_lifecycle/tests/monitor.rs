use std::collections::BTreeMap;

use reqwest::StatusCode;
use rstest::rstest;
use tracing::Level;
use tracing::subscriber::{set_default, with_default};

use super::support::{
    BLOCKING_REPLY, CapturedLog, MODEL_RESPONSE, SAFE_REPLY, USER_MESSAGES, build, data, hooked,
    mock_noma, moderation, params, post_call, pre_call, records, request_count, scanned_text,
    value,
};
use crate::noma_lifecycle::NomaGuardrail;
use crate::noma_lifecycle::background::{check_background, create_background_noma_check};
use crate::noma_lifecycle::hooks::ScanSubject;

fn monitoring(api_base: &str) -> NomaGuardrail {
    let mut configured = params(api_base);
    configured.monitor_mode = Some(true);
    build(&configured)
}

#[rstest]
#[case::pre_call(true)]
#[case::moderation(false)]
#[tokio::test]
async fn monitor_mode_returns_at_once_and_scans_in_the_background(#[case] pre: bool) {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let guardrail = monitoring(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let original = request_data.clone();
    if pre {
        pre_call(&guardrail, &mut request_data, "completion")
            .await
            .unwrap();
    } else {
        moderation(&guardrail, &mut request_data, "completion")
            .await
            .unwrap();
    }
    // nothing has run yet: the hook never waited for the scan
    assert_eq!(request_count(&mock), 0);
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    assert_eq!(request_count(&mock), 1);
    assert!(scanned_text(&mock, 0).contains("hello"));
    assert_eq!(request_data, original);
}

#[tokio::test]
async fn monitor_mode_checks_the_response_in_the_background() {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let guardrail = monitoring(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let mut response = value(MODEL_RESPONSE);
    post_call(&guardrail, &mut request_data, &mut response)
        .await
        .unwrap();
    assert_eq!(request_count(&mock), 0);
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    assert_eq!(request_count(&mock), 1);
    assert_eq!(scanned_text(&mock, 0), "hello");
    assert_eq!(response, value(MODEL_RESPONSE));
}

#[tokio::test]
async fn monitor_mode_still_honours_the_event_hook() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let mut configured = hooked(&mock.base, "post_call");
    configured.monitor_mode = Some(true);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    pre_call(&guardrail, &mut request_data, "completion")
        .await
        .unwrap();
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    assert_eq!(request_count(&mock), 0);
}

#[rstest]
#[case::user_message(true, "Noma background user message check failed")]
#[case::response(false, "Noma background response check failed")]
#[tokio::test]
async fn a_failing_background_scan_is_logged_and_never_raised(
    #[case] pre: bool,
    #[case] logged: &str,
) {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let guardrail = monitoring(&mock.base);
    let log = CapturedLog::default();
    let _guard = set_default(log.subscriber(Level::ERROR));
    let mut request_data = data(USER_MESSAGES);
    if pre {
        pre_call(&guardrail, &mut request_data, "completion")
            .await
            .unwrap();
    } else {
        let mut response = value(MODEL_RESPONSE);
        post_call(&guardrail, &mut request_data, &mut response)
            .await
            .unwrap();
    }
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    assert_eq!(request_count(&mock), 1);
    assert!(log.text().contains(logged));
}

#[tokio::test]
async fn the_user_message_background_check_logs_a_block_without_raising() {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let log = CapturedLog::default();
    let _guard = set_default(log.subscriber(Level::ERROR));
    let mut request_data = data(USER_MESSAGES);
    check_background(
        &guardrail,
        &mut request_data,
        &BTreeMap::new(),
        ScanSubject::UserMessage,
    )
    .await;
    assert_eq!(request_count(&mock), 1);
    assert!(
        log.text()
            .contains("Noma background user message check failed")
    );
}

#[tokio::test]
async fn the_response_background_check_scans_the_response_and_logs_a_failure() {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let guardrail = build(&params(&mock.base));
    let log = CapturedLog::default();
    let _guard = set_default(log.subscriber(Level::ERROR));
    let mut request_data = data(USER_MESSAGES);
    let mut response = value(MODEL_RESPONSE);
    check_background(
        &guardrail,
        &mut request_data,
        &BTreeMap::new(),
        ScanSubject::LlmResponse(&mut response),
    )
    .await;
    assert_eq!(request_count(&mock), 1);
    assert_eq!(scanned_text(&mock, 0), "hello");
    assert!(log.text().contains("Noma background response check failed"));
}

#[tokio::test]
async fn a_background_check_that_succeeds_logs_no_error() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let log = CapturedLog::default();
    let _guard = set_default(log.subscriber(Level::ERROR));
    let mut request_data = data(USER_MESSAGES);
    check_background(
        &guardrail,
        &mut request_data,
        &BTreeMap::new(),
        ScanSubject::UserMessage,
    )
    .await;
    assert_eq!(request_count(&mock), 1);
    assert!(log.text().is_empty());
    assert_eq!(records(&request_data).len(), 1);
}

#[test]
fn a_missing_runtime_is_logged_when_the_background_task_is_created() {
    let guardrail = build(&params("http://noma"));
    let log = CapturedLog::default();
    with_default(log.subscriber(Level::ERROR), || {
        create_background_noma_check(&guardrail.tasks, async {});
    });
    assert!(log.text().contains("Failed to create background Noma task"));
}
