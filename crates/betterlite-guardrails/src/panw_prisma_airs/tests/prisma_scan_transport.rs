use axum::http::StatusCode;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::support::{guardrail, metadata, serve, settings};
use crate::panw_prisma_airs::audit::Direction;
use crate::panw_prisma_airs::construct::{ConstructError, PanwPrismaAirs};
use crate::panw_prisma_airs::metadata::ScanMetadata;
use crate::panw_prisma_airs::payload::{ScanRequest, build_payload};
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, Failure};

const ALLOW: &[u8] = br#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;

fn request<'req>(content: &'req str, metadata: Option<&'req ScanMetadata>) -> ScanRequest<'req> {
    ScanRequest {
        content,
        direction: Direction::Request,
        metadata,
        call_id: Some("call-1"),
        tool_event: None,
    }
}

fn payload_json(metadata: Option<&ScanMetadata>, direction: Direction) -> String {
    let settings = settings("http://unused.test");
    let scan = ScanRequest {
        direction,
        ..request("hello", metadata)
    };
    serde_json::to_string(&build_payload(&settings, &scan)).expect("payload serializes")
}

#[test]
fn payload_carries_profile_app_name_user_ip_trace_id_and_app_user() {
    let metadata = ScanMetadata {
        user: "u1".to_owned(),
        model: "gpt-4o".to_owned(),
        profile_name: Some("meta-profile".to_owned()),
        profile_id: Some("pid-1".to_owned()),
        user_ip: Some("10.0.0.1".to_owned()),
        requester_ip_address: None,
        app_name: Some("shop".to_owned()),
        app_user: Some("alice".to_owned()),
        litellm_trace_id: Some("trace-1".to_owned()),
    };
    assert_eq!(
        payload_json(Some(&metadata), Direction::Request),
        r#"{"metadata":{"app_user":"alice","ai_model":"gpt-4o","app_name":"LiteLLM-shop","source":"litellm_builtin_guardrail","user_ip":"10.0.0.1","litellm_trace_id":"trace-1"},"contents":[{"prompt":"hello"}],"tr_id":"call-1","ai_profile":{"profile_id":"pid-1","profile_name":"meta-profile"}}"#
    );
}

#[test]
fn payload_defaults_without_metadata_and_uses_the_config_profile() {
    assert_eq!(
        payload_json(None, Direction::Request),
        r#"{"metadata":{"app_user":"litellm_user","ai_model":"unknown","app_name":"LiteLLM","source":"litellm_builtin_guardrail"},"contents":[{"prompt":"hello"}],"tr_id":"call-1","ai_profile":{"profile_name":"config-profile"}}"#
    );
}

#[test]
fn response_scan_sends_response_text_and_the_is_response_flag() {
    let json = payload_json(None, Direction::Response);
    assert!(json.contains(r#""contents":[{"response":"hello"}]"#));
    assert!(json.contains(r#""is_response":true"#));
}

#[test]
fn tool_event_replaces_the_text_and_skips_the_is_response_flag() {
    let settings = settings("http://unused.test");
    let event =
        RawValue::from_string(r#"{"metadata":{"ecosystem":"mcp"},"input":"{}"}"#.to_owned())
            .expect("raw");
    let scan = ScanRequest {
        direction: Direction::Response,
        tool_event: Some(&event),
        ..request("", None)
    };
    let json = serde_json::to_string(&build_payload(&settings, &scan)).expect("payload");
    assert!(
        json.contains(
            r#""contents":[{"tool_event":{"metadata":{"ecosystem":"mcp"},"input":"{}"}}]"#
        )
    );
    assert!(!json.contains("is_response"));
}

#[rstest]
#[case::user_ip_wins(Some("1.1.1.1"), Some("2.2.2.2"), Some("1.1.1.1"))]
#[case::requester_ip_address_fallback(None, Some("2.2.2.2"), Some("2.2.2.2"))]
#[case::none(None, None, None)]
fn user_ip_falls_back_to_requester_ip_address(
    #[case] user_ip: Option<&str>,
    #[case] requester: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let metadata = ScanMetadata {
        user_ip: user_ip.map(str::to_owned),
        requester_ip_address: requester.map(str::to_owned),
        ..metadata()
    };
    let json = payload_json(Some(&metadata), Direction::Request);
    let sent = expected.map(|ip| format!(r#""user_ip":"{ip}""#));
    match sent {
        Some(field) => assert!(json.contains(&field)),
        None => assert!(!json.contains("user_ip")),
    }
}

#[rstest]
#[case::id_and_name(
    Some("pid"),
    Some("meta"),
    r#"{"profile_id":"pid","profile_name":"meta"}"#
)]
#[case::name_only(None, Some("meta"), r#"{"profile_name":"meta"}"#)]
#[case::id_keeps_the_config_name(
    Some("pid"),
    None,
    r#"{"profile_id":"pid","profile_name":"config-profile"}"#
)]
fn profile_id_and_name_follow_metadata_before_config(
    #[case] profile_id: Option<&str>,
    #[case] profile_name: Option<&str>,
    #[case] expected: &str,
) {
    let metadata = ScanMetadata {
        profile_id: profile_id.map(str::to_owned),
        profile_name: profile_name.map(str::to_owned),
        ..metadata()
    };
    let json = payload_json(Some(&metadata), Direction::Request);
    assert!(
        json.contains(&format!(r#""ai_profile":{expected}"#)),
        "{json}"
    );
}

#[test]
fn no_profile_anywhere_omits_ai_profile() {
    let mut settings = settings("http://unused.test");
    settings.profile_name = None;
    let scan = request("hello", None);
    let json = serde_json::to_string(&build_payload(&settings, &scan)).expect("payload");
    assert!(!json.contains("ai_profile"));
}

#[test]
fn app_name_without_metadata_name_is_not_prefixed_twice() {
    let mut settings = settings("http://unused.test");
    settings.app_name = "LiteLLM-configured".to_owned();
    let json =
        serde_json::to_string(&build_payload(&settings, &request("hello", None))).expect("payload");
    assert!(json.contains(r#""app_name":"LiteLLM-configured""#));
}

#[tokio::test]
async fn scan_posts_json_with_the_api_token_and_parses_the_verdict() {
    let server = serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base));
    let verdict = call_panw_api(&guardrail, request("hello", None)).await;
    assert_eq!(verdict.action, Action::Allow);
    assert_eq!(verdict.category.as_deref(), Some("benign"));
    assert_eq!(verdict.scan_id.as_deref(), Some("scan-1"));
    assert_eq!(verdict.failure, None);
    let captured = server.captured.lock().await.clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].token.as_deref(), Some("airs-key"));
    assert_eq!(
        captured[0].content_type.as_deref(),
        Some("application/json")
    );
    assert!(
        captured[0]
            .body
            .contains(r#""contents":[{"prompt":"hello"}]"#)
    );
}

#[tokio::test]
async fn blank_content_is_allowed_without_a_request() {
    let server = serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base));
    let verdict = call_panw_api(&guardrail, request("  \n", None)).await;
    assert_eq!(verdict.action, Action::Allow);
    assert_eq!(verdict.category.as_deref(), Some("empty"));
    assert!(server.captured.lock().await.is_empty());
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = serve(StatusCode::FOUND, b"").await;
    let guardrail = guardrail(settings(&server.base));
    let verdict = call_panw_api(&guardrail, request("hello", None)).await;
    assert_eq!(verdict.category.as_deref(), Some("http_302_error"));
    assert_eq!(verdict.failure, Some(Failure::Permanent));
    assert!(server.redirect_hits.lock().await.is_empty());
}

#[tokio::test]
async fn a_response_without_an_action_is_a_plain_api_error_block() {
    let server = serve(StatusCode::OK, br#"{"category":"benign"}"#).await;
    let guardrail = guardrail(settings(&server.base));
    let verdict = call_panw_api(&guardrail, request("hello", None)).await;
    assert_eq!(verdict.action, Action::Block);
    assert_eq!(verdict.category.as_deref(), Some("api_error"));
    assert_eq!(verdict.failure, None);
}

#[test]
fn construct_rejects_an_empty_api_key() {
    let mut settings = settings("http://unused.test");
    settings.api_key = SecretString::from(String::new());
    assert!(matches!(
        PanwPrismaAirs::new(settings),
        Err(ConstructError::MissingApiKey)
    ));
}

#[test]
fn construct_rejects_an_api_key_that_is_not_a_header_value() {
    let mut settings = settings("http://unused.test");
    settings.api_key = SecretString::from("bad\nkey".to_owned());
    assert!(matches!(
        PanwPrismaAirs::new(settings),
        Err(ConstructError::InvalidApiKey { .. })
    ));
}
