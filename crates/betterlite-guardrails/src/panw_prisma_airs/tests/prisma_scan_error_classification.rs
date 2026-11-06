use std::time::Duration;

use axum::http::StatusCode;
use rstest::rstest;
use serde_json::value::RawValue;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;

use super::support::{guardrail, serve, serve_after, settings};
use crate::panw_prisma_airs::audit::Direction;
use crate::panw_prisma_airs::payload::ScanRequest;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, Failure, ScanVerdict};

fn request<'req>(
    call_id: Option<&'req str>,
    tool_event: Option<&'req RawValue>,
) -> ScanRequest<'req> {
    ScanRequest {
        content: "hello",
        direction: Direction::Request,
        metadata: None,
        call_id,
        tool_event,
    }
}

async fn scan_against(status: StatusCode, body: &[u8]) -> ScanVerdict {
    let server = serve(status, body).await;
    call_panw_api(
        &guardrail(settings(&server.base)),
        request(Some("call-1"), None),
    )
    .await
}

#[rstest]
#[case::unauthorized(StatusCode::UNAUTHORIZED, "config_error", Failure::Permanent)]
#[case::forbidden(StatusCode::FORBIDDEN, "config_error", Failure::Permanent)]
#[case::bad_request(StatusCode::BAD_REQUEST, "http_400_error", Failure::Permanent)]
#[case::not_found(StatusCode::NOT_FOUND, "http_404_error", Failure::Permanent)]
#[case::redirect(StatusCode::FOUND, "http_302_error", Failure::Permanent)]
#[case::too_many_requests(StatusCode::TOO_MANY_REQUESTS, "http_429_error", Failure::Transient)]
#[case::server_error(
    StatusCode::INTERNAL_SERVER_ERROR,
    "http_500_error",
    Failure::Transient
)]
#[case::unavailable(StatusCode::SERVICE_UNAVAILABLE, "http_503_error", Failure::Transient)]
#[tokio::test]
async fn http_status_maps_to_a_blocking_category(
    #[case] status: StatusCode,
    #[case] category: &str,
    #[case] failure: Failure,
) {
    let verdict = scan_against(status, b"{}").await;
    assert_eq!(verdict.action, Action::Block);
    assert_eq!(verdict.category.as_deref(), Some(category));
    assert_eq!(verdict.failure, Some(failure));
}

#[rstest]
#[case::profile_not_found("Profile Not Found for key")]
#[case::profile_required("profile required")]
#[case::invalid_profile("Invalid Profile name")]
#[tokio::test]
async fn profile_phrases_in_a_client_error_body_are_a_config_error(#[case] body: &str) {
    let verdict = scan_against(StatusCode::BAD_REQUEST, body.as_bytes()).await;
    assert_eq!(verdict.category.as_deref(), Some("config_error"));
    assert_eq!(verdict.failure, Some(Failure::Permanent));
}

#[tokio::test]
async fn profile_phrase_in_a_server_error_body_is_still_a_config_error() {
    let verdict = scan_against(StatusCode::INTERNAL_SERVER_ERROR, b"profile not found").await;
    assert_eq!(verdict.category.as_deref(), Some("config_error"));
    assert_eq!(verdict.failure, Some(Failure::Permanent));
}

#[tokio::test]
async fn an_error_body_that_is_not_utf8_carries_no_profile_phrase() {
    let verdict = scan_against(StatusCode::BAD_REQUEST, &[0xff, 0xfe, 0x00]).await;
    assert_eq!(verdict.category.as_deref(), Some("http_400_error"));
}

#[tokio::test]
async fn a_response_that_is_not_json_is_a_transient_api_error() {
    let verdict = scan_against(StatusCode::OK, b"<html>nope</html>").await;
    assert_eq!(verdict.action, Action::Block);
    assert_eq!(verdict.category.as_deref(), Some("api_error"));
    assert_eq!(verdict.failure, Some(Failure::Transient));
}

#[tokio::test]
async fn an_oversize_success_body_is_a_transient_api_error() {
    let body = vec![b' '; 1024 * 1024 + 1];
    let verdict = scan_against(StatusCode::OK, &body).await;
    assert_eq!(verdict.category.as_deref(), Some("api_error"));
    assert_eq!(verdict.failure, Some(Failure::Transient));
}

#[tokio::test]
async fn an_oversize_error_body_keeps_the_http_status_authoritative() {
    let body = vec![b'x'; 1024 * 1024 + 1];
    let verdict = scan_against(StatusCode::SERVICE_UNAVAILABLE, &body).await;
    assert_eq!(verdict.category.as_deref(), Some("http_503_error"));
    assert_eq!(verdict.failure, Some(Failure::Transient));
}

#[tokio::test]
async fn a_slow_scan_api_is_a_transient_timeout_error() {
    let server = serve_after(StatusCode::OK, b"{}", Duration::from_secs(5)).await;
    let mut config = settings(&server.base);
    config.timeout = 0.05_f64;
    let verdict = call_panw_api(&guardrail(config), request(Some("call-1"), None)).await;
    assert_eq!(verdict.category.as_deref(), Some("timeout_error"));
    assert_eq!(verdict.failure, Some(Failure::Transient));
}

#[tokio::test]
async fn a_refused_connection_is_a_transient_network_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let verdict = call_panw_api(&guardrail(settings(&base)), request(Some("call-1"), None)).await;
    assert_eq!(verdict.category.as_deref(), Some("network_error"));
    assert_eq!(verdict.failure, Some(Failure::Transient));
}

#[tokio::test]
async fn a_connection_closed_mid_reply_is_a_transient_network_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let accept = async {
        let (mut socket, _) = listener.accept().await.expect("accept");
        socket.shutdown().await.expect("shutdown");
    };
    let guard = guardrail(settings(&base));
    let (verdict, ()) = tokio::join!(call_panw_api(&guard, request(Some("call-1"), None)), accept);
    assert_eq!(verdict.category.as_deref(), Some("network_error"));
    assert_eq!(verdict.failure, Some(Failure::Transient));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn a_missing_call_id_blocks_without_calling_the_api(#[case] call_id: Option<&str>) {
    let server = serve(StatusCode::OK, br#"{"action":"allow"}"#).await;
    let verdict = call_panw_api(&guardrail(settings(&server.base)), request(call_id, None)).await;
    assert_eq!(verdict.action, Action::Block);
    assert_eq!(verdict.category.as_deref(), Some("missing_call_id"));
    assert_eq!(verdict.failure, Some(Failure::Permanent));
    assert!(server.captured.lock().await.is_empty());
}

#[tokio::test]
async fn an_mcp_tool_event_may_go_without_a_call_id() {
    let server = serve(StatusCode::OK, br#"{"action":"allow","category":"benign"}"#).await;
    let event =
        RawValue::from_string(r#"{"metadata":{"ecosystem":"mcp"},"input":"{}"}"#.to_owned())
            .expect("raw");
    let verdict = call_panw_api(
        &guardrail(settings(&server.base)),
        request(None, Some(&event)),
    )
    .await;
    assert_eq!(verdict.action, Action::Allow);
    assert_eq!(server.captured.lock().await.len(), 1);
}

#[tokio::test]
async fn a_tool_event_from_another_ecosystem_still_needs_a_call_id() {
    let server = serve(StatusCode::OK, br#"{"action":"allow"}"#).await;
    let event =
        RawValue::from_string(r#"{"metadata":{"ecosystem":"other"},"input":"{}"}"#.to_owned())
            .expect("raw");
    let verdict = call_panw_api(
        &guardrail(settings(&server.base)),
        request(None, Some(&event)),
    )
    .await;
    assert_eq!(verdict.category.as_deref(), Some("missing_call_id"));
}

#[tokio::test]
async fn a_bad_request_for_a_tool_event_is_still_a_permanent_http_error() {
    let server = serve(StatusCode::BAD_REQUEST, b"bad input").await;
    let event =
        RawValue::from_string(r#"{"metadata":{"ecosystem":"mcp"},"input":"{}"}"#.to_owned())
            .expect("raw");
    let verdict = call_panw_api(
        &guardrail(settings(&server.base)),
        request(None, Some(&event)),
    )
    .await;
    assert_eq!(verdict.category.as_deref(), Some("http_400_error"));
    assert_eq!(verdict.failure, Some(Failure::Permanent));
}
