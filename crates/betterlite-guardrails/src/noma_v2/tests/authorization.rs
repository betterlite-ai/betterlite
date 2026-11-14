use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::noma_v2::config::NomaV2Settings;
use betterlite_guardrails::noma_v2::error::NomaV2Error;

use crate::support::{
    API_KEY, NONE_REPLY, captured_headers, captured_path, empty_env, guardrail, inputs, mock_noma,
    params, request_data, scan, settings,
};

#[tokio::test]
async fn the_authorization_header_carries_the_bearer_key() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    outcome.result.unwrap();
    let headers = captured_headers(&mock);
    let bearer = format!("Bearer {API_KEY}");
    assert_eq!(headers.get(AUTHORIZATION).unwrap(), bearer.as_str());
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
    assert_eq!(captured_path(&mock), "/litellm/guardrail");
}

#[tokio::test]
async fn a_custom_hostname_no_api_key_sends_no_authorization_header() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.api_key = None;
    let outcome = scan(
        &guardrail(config),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    outcome.result.unwrap();
    assert!(captured_headers(&mock).get(AUTHORIZATION).is_none());
}

#[test]
fn a_saas_api_base_requires_api_key() {
    let mut configured = params();
    configured.api_base = Some(String::from("https://api.noma.security"));
    let error = NomaV2Settings::resolve(&configured, &empty_env()).unwrap_err();
    assert!(matches!(error, NomaV2Error::MissingApiKey));
}

#[test]
fn a_saas_api_base_with_a_port_requires_api_key() {
    let mut configured = params();
    configured.api_base = Some(String::from("https://api.noma.security:8443/"));
    let error = NomaV2Settings::resolve(&configured, &empty_env()).unwrap_err();
    assert!(matches!(error, NomaV2Error::MissingApiKey));
}

#[test]
fn a_custom_hostname_no_api_key_resolves_without_one() {
    let mut configured = params();
    configured.api_base = Some(String::from("https://noma.example"));
    let resolved = NomaV2Settings::resolve(&configured, &empty_env()).unwrap();
    assert!(resolved.api_key.is_none());
    assert_eq!(resolved.api_base, "https://noma.example");
}
