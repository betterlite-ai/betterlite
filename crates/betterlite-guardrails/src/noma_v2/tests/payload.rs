use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;

use crate::support::{
    NONE_REPLY, captured_body, data, guardrail, inputs, mock_noma, request_data, scan, scan_with,
    settings, value,
};

#[tokio::test]
async fn the_payload_carries_the_inputs_and_request_data() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    let body = captured_body(&mock);
    let inputs_json = serde_json::to_string(&inputs()).unwrap();
    assert_eq!(body.get("inputs"), Some(&value(&inputs_json)));
    let Some(GuardrailValue::Object(sent)) = body.get("request_data") else {
        panic!("the payload has no request_data object");
    };
    assert_eq!(
        sent.get("model"),
        Some(&GuardrailValue::Text(String::from("gpt-4o")))
    );
    assert!(sent.contains_key("litellm_metadata"));
    assert_eq!(
        body.get("input_type"),
        Some(&GuardrailValue::Text(String::from("request")))
    );
    assert_eq!(
        body.get("monitor_mode"),
        Some(&GuardrailValue::Boolean(false))
    );
    assert_eq!(
        body.get("application_id"),
        Some(&GuardrailValue::Text(String::from("app-1")))
    );
    assert_eq!(body.get("gateway_name"), None);
}

#[tokio::test]
async fn the_payload_excludes_messages_and_input_from_request_data() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    let body = captured_body(&mock);
    let Some(GuardrailValue::Object(sent)) = body.get("request_data") else {
        panic!("the payload has no request_data object");
    };
    assert!(!sent.contains_key("messages"));
    assert!(!sent.contains_key("input"));
    assert!(sent.contains_key("model"));
    let Some(GuardrailValue::Object(scanned)) = body.get("inputs") else {
        panic!("the payload has no inputs object");
    };
    assert!(scanned.contains_key("texts"));
}

#[tokio::test]
async fn the_payload_carries_the_gateway_name_when_configured() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.gateway_name = Some(String::from("gw-1"));
    let outcome = scan(
        &guardrail(config),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("gateway_name"),
        Some(&GuardrailValue::Text(String::from("gw-1")))
    );
}

#[tokio::test]
async fn the_payload_marks_a_response_input_type() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Response,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("input_type"),
        Some(&GuardrailValue::Text(String::from("response")))
    );
}

#[tokio::test]
async fn the_payload_omits_an_unresolved_application_id() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.application_id = None;
    let outcome = scan(
        &guardrail(config),
        inputs(),
        data(r#"{"model": "gpt-4o"}"#),
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(captured_body(&mock).get("application_id"), None);
}

#[tokio::test]
async fn the_payload_logging_object_drops_the_duplicate_keys() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let target = GuardrailLogTarget {
        litellm_params: Some(BTreeMap::new()),
        model_call_params: Some(data(
            r#"{
                "messages": [],
                "input": "x",
                "additional_args": {},
                "standard_logging_object": {},
                "original_response": {},
                "metadata_key": "kept"
            }"#,
        )),
    };
    let outcome = scan_with(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
        target,
    )
    .await;
    assert!(outcome.result.is_ok());
    let body = captured_body(&mock);
    let Some(GuardrailValue::Object(sent)) = body.get("request_data") else {
        panic!("the payload has no request_data object");
    };
    let Some(GuardrailValue::Object(logging)) = sent.get("litellm_logging_obj") else {
        panic!("the payload has no logging object");
    };
    assert_eq!(logging.len(), 1);
    assert_eq!(
        logging.get("metadata_key"),
        Some(&GuardrailValue::Text(String::from("kept")))
    );
    assert!(!sent.contains_key("standard_logging_guardrail_information"));
}

#[tokio::test]
async fn the_payload_logging_object_is_null_without_model_call_details() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let target = GuardrailLogTarget {
        litellm_params: None,
        model_call_params: None,
    };
    let outcome = scan_with(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
        target,
    )
    .await;
    assert!(outcome.result.is_ok());
    let body = captured_body(&mock);
    let Some(GuardrailValue::Object(sent)) = body.get("request_data") else {
        panic!("the payload has no request_data object");
    };
    assert_eq!(sent.get("litellm_logging_obj"), Some(&GuardrailValue::Null));
}

#[tokio::test]
async fn application_id_fallback_prefers_the_dynamic_value() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let sent_data = data(
        r#"{
            "guardrails": [{"noma-v2-test": {"extra_body": {"application_id": "dynamic-app"}}}],
            "litellm_metadata": {"user_api_key_alias": "alias-app"}
        }"#,
    );
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("application_id"),
        Some(&GuardrailValue::Text(String::from("dynamic-app")))
    );
}

#[tokio::test]
async fn application_id_fallback_uses_the_configured_value() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let sent_data = data(r#"{"litellm_metadata": {"user_api_key_alias": "alias-app"}}"#);
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("application_id"),
        Some(&GuardrailValue::Text(String::from("app-1")))
    );
}

#[tokio::test]
async fn application_id_fallback_uses_the_litellm_metadata_alias() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.application_id = None;
    let sent_data = data(r#"{"litellm_metadata": {"user_api_key_alias": "alias-app"}}"#);
    let outcome = scan(
        &guardrail(config),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("application_id"),
        Some(&GuardrailValue::Text(String::from("alias-app")))
    );
}

#[tokio::test]
async fn application_id_fallback_uses_the_metadata_alias() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.application_id = None;
    let sent_data = data(r#"{"metadata": {"user_api_key_alias": "alias-app"}}"#);
    let outcome = scan(
        &guardrail(config),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("application_id"),
        Some(&GuardrailValue::Text(String::from("alias-app")))
    );
}

#[tokio::test]
async fn application_id_fallback_skips_an_empty_dynamic_value() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.application_id = None;
    let sent_data = data(
        r#"{
            "guardrails": [{"noma-v2-test": {"extra_body": {"application_id": "   "}}}],
            "litellm_metadata": {"user_api_key_alias": "alias-app"}
        }"#,
    );
    let outcome = scan(
        &guardrail(config),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("application_id"),
        Some(&GuardrailValue::Text(String::from("alias-app")))
    );
}

#[tokio::test]
async fn application_id_fallback_skips_a_non_object_metadata() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.application_id = None;
    let sent_data = data(r#"{"litellm_metadata": 5, "metadata": 7}"#);
    let outcome = scan(
        &guardrail(config),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(captured_body(&mock).get("application_id"), None);
}

#[tokio::test]
async fn application_id_fallback_skips_a_non_text_dynamic_value() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let mut config = settings(&mock.base);
    config.application_id = None;
    let sent_data = data(
        r#"{
            "guardrails": [{"noma-v2-test": {"extra_body": {"application_id": 7}}}],
            "litellm_metadata": {"user_api_key_alias": "alias-app"}
        }"#,
    );
    let outcome = scan(
        &guardrail(config),
        inputs(),
        sent_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(outcome.result.is_ok());
    assert_eq!(
        captured_body(&mock).get("application_id"),
        Some(&GuardrailValue::Text(String::from("alias-app")))
    );
}
