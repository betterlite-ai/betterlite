use axum::http::StatusCode;
use secrecy::SecretString;

use super::support;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::singulr::SUPPORTED_EVENT_HOOKS;
use crate::singulr::error::{STATUS_CODE, SingulrError};

#[tokio::test]
async fn the_call_posts_to_the_guard_endpoint_with_the_singulr_headers() {
    let server = support::mock(StatusCode::OK, r#"{"should_block": false}"#).await;
    let guardrail = support::guardrail(&server.base);
    guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    let calls = server.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.path, "/api/v1/ai-gateway/litellm-v2");
    assert_eq!(
        call.headers
            .get("x-singulr-gateway-token")
            .map(String::as_str),
        Some(support::API_KEY)
    );
    assert_eq!(
        call.headers
            .get("x-singulr-enforcement-entity-id")
            .map(String::as_str),
        Some(support::APPLICATION_ID)
    );
    assert_eq!(
        call.headers
            .get("x-singulr-guardrail-id")
            .map(String::as_str),
        Some(support::GUARDRAIL_ID)
    );
    assert_eq!(
        call.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    support::expects(
        call,
        r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
            "guardrail_scope": "request", "messages": [{"role": "user", "content": "hello"}],
            "images": null, "tools": null, "response": null, "metadata": null}"#,
    );
}

#[tokio::test]
async fn an_empty_header_value_is_left_out() {
    let server = support::mock(StatusCode::OK, r#"{"should_block": false}"#).await;
    let mut args = support::args_for(&server.base);
    args.singulr_api_key = Some(SecretString::from(""));
    args.singulr_application_id = Some(String::new());
    args.singulr_guardrail_id = None;
    let guardrail = support::build(&args);
    guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    let calls = server.calls();
    let headers = &calls[0].headers;
    assert!(!headers.contains_key("x-singulr-gateway-token"));
    assert!(!headers.contains_key("x-singulr-enforcement-entity-id"));
    assert!(!headers.contains_key("x-singulr-guardrail-id"));
}

#[tokio::test]
async fn an_allowed_answer_returns_the_inputs_unchanged() {
    let server = support::mock(StatusCode::OK, r#"{}"#).await;
    let guardrail = support::guardrail(&server.base);
    let inputs = support::inputs(r#"{"texts": ["hello"], "model": "gpt-4o"}"#);
    let returned = guardrail
        .apply_guardrail(
            inputs.clone(),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned, inputs);
}

#[tokio::test]
async fn a_blocked_answer_raises_with_status_400_and_the_reason() {
    let server = support::mock(
        StatusCode::OK,
        r#"{"should_block": true, "blocking_due_to": "policy"}"#,
    )
    .await;
    let guardrail = support::guardrail(&server.base);
    let error = guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, SingulrError::Blocked { .. }));
    assert!(error.blocked_content());
    assert_eq!(
        error.to_string(),
        "Blocked by Singulr, Blocking due to policy"
    );
    assert_eq!(STATUS_CODE, 400);
}

#[tokio::test]
async fn a_block_without_a_reason_says_unknown() {
    let server = support::mock(StatusCode::OK, r#"{"should_block": true}"#).await;
    let guardrail = support::guardrail(&server.base);
    let error = guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Blocked by Singulr, Blocking due to unknown"
    );
}

#[tokio::test]
async fn an_http_error_keeps_the_status_and_the_body() {
    let server = support::mock(StatusCode::INTERNAL_SERVER_ERROR, r#"{"detail": "boom"}"#).await;
    let guardrail = support::guardrail(&server.base);
    let error = guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SingulrError::HttpStatus { status: 500, .. }
    ));
    assert!(!error.blocked_content());
    assert_eq!(
        error.to_string(),
        r#"Singulr API returned HTTP 500: {"detail": "boom"}"#
    );
}

#[tokio::test]
async fn an_http_error_passes_when_block_on_error_is_false() {
    let server = support::mock(StatusCode::BAD_GATEWAY, r#"{"detail": "boom"}"#).await;
    let guardrail = support::open_guardrail(&server.base);
    let inputs = support::inputs(r#"{"texts": ["hello"]}"#);
    let returned = guardrail
        .apply_guardrail(
            inputs.clone(),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned, inputs);
}

#[tokio::test]
async fn a_transport_failure_says_the_api_is_unreachable() {
    let base = support::refused_base().await;
    let guardrail = support::guardrail(&base);
    let error = guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, SingulrError::Transport { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Singulr API unreachable (block_on_error=True): ")
    );
}

#[tokio::test]
async fn a_transport_failure_passes_when_block_on_error_is_false() {
    let base = support::refused_base().await;
    let guardrail = support::open_guardrail(&base);
    let inputs = support::inputs(r#"{"texts": ["hello"]}"#);
    let returned = guardrail
        .apply_guardrail(
            inputs.clone(),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned, inputs);
}

#[tokio::test]
async fn an_answer_that_does_not_parse_is_an_invalid_response() {
    let server = support::mock(StatusCode::OK, r#"{"should_block": "maybe"}"#).await;
    let guardrail = support::guardrail(&server.base);
    let error = guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, SingulrError::InvalidResponse { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Singulr API returned an invalid response: ")
    );
}

#[tokio::test]
async fn an_answer_that_does_not_parse_passes_when_block_on_error_is_false() {
    let server = support::mock(StatusCode::OK, r#"not json"#).await;
    let guardrail = support::open_guardrail(&server.base);
    let inputs = support::inputs(r#"{"texts": ["hello"]}"#);
    let returned = guardrail
        .apply_guardrail(
            inputs.clone(),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned, inputs);
}

#[tokio::test]
async fn an_answer_with_extra_fields_is_read() {
    let server = support::mock(
        StatusCode::OK,
        r#"{"should_block": true, "blocking_due_to": "x", "extra": 1}"#,
    )
    .await;
    let guardrail = support::guardrail(&server.base);
    let error = guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Blocked by Singulr, Blocking due to x");
}

#[tokio::test]
async fn an_empty_request_sends_no_call() {
    let server = support::mock(StatusCode::OK, r#"{"should_block": true}"#).await;
    let guardrail = support::guardrail(&server.base);
    let inputs = support::inputs(r#"{"texts": []}"#);
    let returned = guardrail
        .apply_guardrail(
            inputs.clone(),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned, inputs);
    assert!(server.calls().is_empty());
}

#[tokio::test]
async fn an_answer_larger_than_a_few_kilobytes_is_still_read() {
    let padding = "x".repeat(4096);
    let reply = format!(r#"{{"should_block": false, "padding": "{padding}"}}"#);
    let server = support::mock(StatusCode::OK, &reply).await;
    let guardrail = support::guardrail(&server.base);
    let returned = guardrail
        .apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned, support::inputs(r#"{"texts": ["hello"]}"#));
}

#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_call_logs_at_error_level() {
    use tracing::subscriber::with_default;
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(move || writer.clone())
        .finish();
    let outcome = with_default(subscriber, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let server = runtime.block_on(support::mock(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"detail": "boom"}"#,
        ));
        runtime.block_on(support::open_guardrail(&server.base).apply_guardrail(
            support::inputs(r#"{"texts": ["hello"]}"#),
            &support::data("{}"),
            GuardrailInputType::Request,
            None,
        ))
    });
    assert!(outcome.is_ok());
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Singulr API returned an HTTP error status"));
}

#[test]
fn the_supported_event_hooks_are_the_singulr_list() {
    assert_eq!(
        support::encoded(&SUPPORTED_EVENT_HOOKS),
        support::json_value(
            r#"["pre_call", "post_call", "logging_only", "pre_mcp_call", "post_mcp_call"]"#
        )
    );
}
