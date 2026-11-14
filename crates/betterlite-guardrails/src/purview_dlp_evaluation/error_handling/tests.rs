#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;

use crate::purview_dlp_evaluation::error_handling::{
    POLICY_BLOCK_STATUS, activity_name, evaluation_http_error, message_error, policy_block,
};
use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::wire::Activity;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn status_error(status: StatusCode, retry_after: Option<&str>) -> PurviewGraphError {
    PurviewGraphError::Status {
        status,
        retry_after: retry_after.map(str::to_owned),
    }
}

#[test]
fn upstream_status_is_preserved_for_rate_limits() {
    let error = evaluation_http_error(
        &status_error(StatusCode::TOO_MANY_REQUESTS, Some("17")),
        Activity::UploadText,
    );
    assert_eq!(error.status, 429);
    assert_eq!(error.retry_after.as_deref(), Some("17"));
    assert_eq!(
        error.detail.get("upstream_status"),
        Some(&GuardrailValue::UnsignedInteger(429))
    );
}

#[test]
fn upstream_status_is_preserved_for_service_unavailable() {
    let error = evaluation_http_error(
        &status_error(StatusCode::SERVICE_UNAVAILABLE, None),
        Activity::DownloadText,
    );
    assert_eq!(error.status, 503);
    assert_eq!(error.detail.get("activity"), Some(&text("downloadText")));
}

#[rstest]
#[case::unauthorized(StatusCode::UNAUTHORIZED)]
#[case::forbidden(StatusCode::FORBIDDEN)]
fn credential_problems_map_to_bad_gateway(#[case] status: StatusCode) {
    let error = evaluation_http_error(&status_error(status, None), Activity::UploadText);
    assert_eq!(error.status, 502);
    assert_eq!(
        error.detail.get("upstream_status"),
        Some(&GuardrailValue::UnsignedInteger(u64::from(status.as_u16())))
    );
}

#[test]
fn other_upstream_statuses_stay_as_sent() {
    assert_eq!(
        evaluation_http_error(
            &status_error(StatusCode::BAD_REQUEST, None),
            Activity::UploadText
        )
        .status,
        400
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn retry_after_is_kept_only_when_present_and_non_empty(#[case] retry_after: Option<&str>) {
    let error = evaluation_http_error(
        &status_error(StatusCode::TOO_MANY_REQUESTS, retry_after),
        Activity::UploadText,
    );
    assert_eq!(error.retry_after, None);
}

#[test]
fn upstream_failure_detail_names_the_error_the_activity_and_the_upstream_status() {
    let source = status_error(StatusCode::TOO_MANY_REQUESTS, None);
    let error = evaluation_http_error(&source, Activity::UploadText);
    assert_eq!(
        error.detail.keys().cloned().collect::<Vec<String>>(),
        vec![
            "error".to_owned(),
            "activity".to_owned(),
            "upstream_status".to_owned(),
            "exception".to_owned(),
        ]
    );
    assert_eq!(
        error.detail.get("error"),
        Some(&text(
            "Microsoft Purview DLP: upstream policy evaluation failed"
        ))
    );
    assert_eq!(
        error.detail.get("exception"),
        Some(&text(&source.to_string()))
    );
}

// Deviation: B3738's TokenStatus carries no Retry-After, so a token endpoint 429 reaches the caller without it.
#[rstest]
#[case::unauthorized(StatusCode::UNAUTHORIZED, 502)]
#[case::rate_limited(StatusCode::TOO_MANY_REQUESTS, 429)]
#[case::unavailable(StatusCode::SERVICE_UNAVAILABLE, 503)]
fn a_token_endpoint_status_maps_like_graph_without_retry_after(
    #[case] status: StatusCode,
    #[case] client_status: u16,
) {
    let error = evaluation_http_error(
        &PurviewGraphError::TokenStatus { status },
        Activity::UploadText,
    );
    assert_eq!(error.status, client_status);
    assert_eq!(error.retry_after, None);
    assert_eq!(
        error.detail.get("upstream_status"),
        Some(&GuardrailValue::UnsignedInteger(u64::from(status.as_u16())))
    );
}

#[test]
fn other_failures_become_a_bad_request_without_an_upstream_status() {
    let error = evaluation_http_error(&PurviewGraphError::TokenExpiry, Activity::DownloadText);
    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert!(error.detail.get("upstream_status").is_none());
    assert_eq!(error.detail.get("activity"), Some(&text("downloadText")));
    assert!(error.detail.contains_key("exception"));
}

#[test]
fn an_unreadable_graph_answer_becomes_a_bad_request() {
    let source = serde_json::from_str::<BTreeMap<String, i32>>("not json").unwrap_err();
    let error = evaluation_http_error(
        &PurviewGraphError::ResponseJson { source },
        Activity::UploadText,
    );
    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert_eq!(
        error.detail.get("error"),
        Some(&text(
            "Microsoft Purview DLP: upstream policy evaluation failed"
        ))
    );
}

#[rstest]
#[case::upload(Activity::UploadText, "uploadText")]
#[case::download(Activity::DownloadText, "downloadText")]
fn policy_block_names_the_activity(#[case] activity: Activity, #[case] name: &str) {
    let error = policy_block(activity);
    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert_eq!(error.retry_after, None);
    assert_eq!(
        error.detail.get("error"),
        Some(&text("Microsoft Purview DLP: Content blocked by policy"))
    );
    assert_eq!(error.detail.get("activity"), Some(&text(name)));
}

#[test]
fn message_error_carries_only_the_error_text() {
    let error = message_error("Microsoft Purview DLP: test message");
    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert_eq!(error.detail.len(), 1);
    assert_eq!(
        error.detail.get("error"),
        Some(&text("Microsoft Purview DLP: test message"))
    );
}

#[rstest]
#[case::upload(Activity::UploadText, "uploadText")]
#[case::download(Activity::DownloadText, "downloadText")]
fn activity_name_is_the_litellm_wire_value(#[case] activity: Activity, #[case] name: &str) {
    assert_eq!(activity_name(activity), name);
}
