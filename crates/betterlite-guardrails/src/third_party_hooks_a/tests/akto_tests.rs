use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::akto_mirroring::{
    Configured, UnreachableFallback as AktoFallback,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;

use super::support::{Call, list, message, object, serve, text};
use crate::akto::AktoMirroring;
use crate::akto::error::AktoError;
use crate::akto::payload::AktoInputs;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::third_party_hooks_a::akto;
use crate::third_party_hooks_a::error::ThirdPartyHookError;

fn akto_guardrail(base: &str, fallback: AktoFallback) -> AktoMirroring {
    AktoMirroring::new(
        Configured {
            guardrail_name: "akto".to_owned(),
            api_base: base.to_owned(),
            api_key: SecretString::from("akto-key"),
            account_id: "1000000".to_owned(),
            vxlan_id: "0".to_owned(),
            unreachable_fallback: fallback,
            timeout: Duration::from_secs(5),
        },
        reqwest::Client::default(),
    )
    .expect("guardrail")
}

fn akto_inputs() -> AktoInputs {
    AktoInputs {
        texts: vec!["hello".to_owned()],
        model: Some("gpt-4o".to_owned()),
        tools: None,
        tool_calls: None,
        structured_messages: None,
    }
}

fn akto_request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("model".to_owned(), text("gpt-4o")),
        ("messages".to_owned(), list(&[message("user", "hello")])),
    ])
}

fn akto_response_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(
        "response".to_owned(),
        object(&[(
            "choices",
            list(&[object(&[("message", message("assistant", "hello"))])]),
        )]),
    )])
}

fn allowed_answer() -> String {
    r#"{"data":{"guardrailsResult":{"Allowed":true}}}"#.to_owned()
}

fn blocked_answer() -> String {
    r#"{"data":{"guardrailsResult":{"Allowed":false,"Reason":"PII detected"}}}"#.to_owned()
}

#[derive(Deserialize)]
struct CapturedPayload {
    #[serde(rename = "responsePayload")]
    response_payload: String,
    #[serde(rename = "responseHeaders")]
    response_headers: String,
    #[serde(rename = "statusCode")]
    status_code: String,
}

async fn unreachable_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("addr");
    drop(listener);
    format!("http://{address}")
}

// runs one hook mode over one input type and returns how many mirror calls it made
async fn run_mode(mode: &LitellmParamsMode, input_type: GuardrailInputType) -> usize {
    let answers = vec![
        (StatusCode::OK, allowed_answer()),
        (StatusCode::OK, allowed_answer()),
    ];
    let (base, _server, captured) = serve("/api/http-proxy", answers).await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailClosed);
    let returned = akto::apply_guardrail(
        &guardrail,
        akto_inputs(),
        &akto_request_data(),
        input_type,
        mode,
    )
    .await
    .expect("mirrored");
    assert_eq!(returned, akto_inputs());
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    captured.lock().expect("captured").len()
}

#[tokio::test]
async fn akto_request_allowed_mirrors_with_guardrails_only() {
    let (base, _server, captured) =
        serve("/api/http-proxy", vec![(StatusCode::OK, allowed_answer())]).await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailClosed);
    let inputs = akto_inputs();
    let returned = akto::apply_guardrail(
        &guardrail,
        inputs.clone(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &LitellmParamsMode::One("pre_call".to_owned()),
    )
    .await
    .expect("allowed");
    assert_eq!(returned, inputs);
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].path.contains("guardrails=true"));
    assert!(!calls[0].path.contains("ingest_data"));
}

#[tokio::test]
async fn akto_request_blocked_answers_403_and_mirrors_the_marker() {
    let (base, _server, captured) =
        serve("/api/http-proxy", vec![(StatusCode::OK, blocked_answer())]).await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailClosed);
    let error = akto::apply_guardrail(
        &guardrail,
        akto_inputs(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &LitellmParamsMode::One("pre_call".to_owned()),
    )
    .await
    .expect_err("blocked");
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert!(matches!(
        &error,
        ThirdPartyHookError::Rejection { detail, .. } if *detail == text("PII detected")
    ));
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 2);
    assert_blocked_ingest(&calls[1]);
}

fn assert_blocked_ingest(call: &Call) {
    assert!(call.path.contains("ingest_data=true"));
    assert!(!call.path.contains("guardrails=true"));
    let payload: CapturedPayload = serde_json::from_str(&call.body).expect("payload");
    assert_eq!(payload.status_code, "403");
    let wrapper =
        serde_json::from_str::<GuardrailValue>(&payload.response_payload).expect("wrapper");
    let GuardrailValue::Object(fields) = wrapper else {
        panic!("responsePayload is not an object");
    };
    let Some(GuardrailValue::Text(inner)) = fields.get("body") else {
        panic!("responsePayload has no body text");
    };
    let marker = serde_json::from_str::<GuardrailValue>(inner).expect("marker");
    assert_eq!(
        marker,
        object(&[
            ("x-blocked-by", text("Akto Proxy")),
            ("reason", text("PII detected"))
        ])
    );
    let headers =
        serde_json::from_str::<GuardrailValue>(&payload.response_headers).expect("responseHeaders");
    assert_eq!(
        headers,
        object(&[("content-type", text("application/json"))])
    );
}

#[tokio::test]
async fn akto_request_blocked_without_a_reason_uses_the_default_message() {
    let (base, _server, _captured) = serve(
        "/api/http-proxy",
        vec![(
            StatusCode::OK,
            r#"{"data":{"guardrailsResult":{"Allowed":false}}}"#.to_owned(),
        )],
    )
    .await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailClosed);
    let error = akto::apply_guardrail(
        &guardrail,
        akto_inputs(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &LitellmParamsMode::One("pre_call".to_owned()),
    )
    .await
    .expect_err("blocked");
    assert!(matches!(
        &error,
        ThirdPartyHookError::Rejection { detail, .. }
            if *detail == text("Blocked by Akto Guardrails")
    ));
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
}

#[tokio::test]
async fn akto_response_side_only_mirrors_in_the_background() {
    let (base, _server, captured) = serve("/api/http-proxy", Vec::new()).await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailClosed);
    let inputs = akto_inputs();
    let returned = akto::apply_guardrail(
        &guardrail,
        inputs.clone(),
        &akto_response_data(),
        GuardrailInputType::Response,
        &LitellmParamsMode::One("post_call".to_owned()),
    )
    .await
    .expect("response");
    assert_eq!(returned, inputs);
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].path.contains("guardrails=true"));
    assert!(calls[0].path.contains("ingest_data=true"));
    let payload: CapturedPayload = serde_json::from_str(&calls[0].body).expect("payload");
    let wrapper =
        serde_json::from_str::<GuardrailValue>(&payload.response_payload).expect("wrapper");
    assert!(matches!(wrapper, GuardrailValue::Object(fields) if fields.contains_key("body")));
}

#[tokio::test]
async fn akto_skips_an_input_type_its_single_hook_mode_does_not_handle() {
    let (base, _server, captured) =
        serve("/api/http-proxy", vec![(StatusCode::OK, allowed_answer())]).await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailClosed);
    let inputs = akto_inputs();
    let skipped = akto::apply_guardrail(
        &guardrail,
        inputs.clone(),
        &akto_request_data(),
        GuardrailInputType::Response,
        &LitellmParamsMode::One("pre_call".to_owned()),
    )
    .await
    .expect("skipped");
    assert_eq!(skipped, inputs);
    let many = LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()]);
    let allowed = akto::apply_guardrail(
        &guardrail,
        inputs.clone(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &many,
    )
    .await
    .expect("allowed");
    assert_eq!(allowed, inputs);
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].path.contains("guardrails=true"));
}

#[tokio::test]
async fn akto_mode_maps_only_the_two_hook_names() {
    let pre = LitellmParamsMode::One("pre_call".to_owned());
    let post = LitellmParamsMode::One("post_call".to_owned());
    let during = LitellmParamsMode::One("during_call".to_owned());
    // post_call handles the response side only, so a request under it is skipped
    assert_eq!(run_mode(&post, GuardrailInputType::Response).await, 1);
    assert_eq!(run_mode(&post, GuardrailInputType::Request).await, 0);
    assert_eq!(run_mode(&pre, GuardrailInputType::Response).await, 0);
    // any other single name maps to neither input type, so both sides run
    assert_eq!(run_mode(&during, GuardrailInputType::Request).await, 1);
    assert_eq!(run_mode(&during, GuardrailInputType::Response).await, 1);
}

#[tokio::test]
async fn akto_unreachable_follows_the_configured_fallback() {
    let base = unreachable_base().await;
    let mode = LitellmParamsMode::One("pre_call".to_owned());
    let closed = akto_guardrail(&base, AktoFallback::FailClosed);
    let inputs = akto_inputs();
    let error = akto::apply_guardrail(
        &closed,
        inputs.clone(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &mode,
    )
    .await
    .expect_err("fail-closed");
    assert_eq!(error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(matches!(
        &error,
        ThirdPartyHookError::Akto(AktoError::Unreachable)
    ));
    let open = akto_guardrail(&base, AktoFallback::FailOpen);
    let returned = akto::apply_guardrail(
        &open,
        inputs.clone(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &mode,
    )
    .await
    .expect("fail-open");
    assert_eq!(returned, inputs);
}

#[tokio::test]
async fn akto_error_status_is_unreachable() {
    let (base, _server, captured) = serve(
        "/api/http-proxy",
        vec![(StatusCode::BAD_GATEWAY, "bad gateway".to_owned())],
    )
    .await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailOpen);
    let inputs = akto_inputs();
    let returned = akto::apply_guardrail(
        &guardrail,
        inputs.clone(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &LitellmParamsMode::One("pre_call".to_owned()),
    )
    .await
    .expect("fail-open");
    assert_eq!(returned, inputs);
    assert_eq!(captured.lock().expect("captured").len(), 1);
}

#[tokio::test]
async fn akto_malformed_answer_is_not_unreachable() {
    let (base, _server, _captured) = serve(
        "/api/http-proxy",
        vec![(StatusCode::OK, "not json".to_owned())],
    )
    .await;
    let guardrail = akto_guardrail(&base, AktoFallback::FailOpen);
    let error = akto::apply_guardrail(
        &guardrail,
        akto_inputs(),
        &akto_request_data(),
        GuardrailInputType::Request,
        &LitellmParamsMode::One("pre_call".to_owned()),
    )
    .await
    .expect_err("malformed answer");
    assert!(matches!(
        &error,
        ThirdPartyHookError::Akto(AktoError::ResponseJson { .. })
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}
