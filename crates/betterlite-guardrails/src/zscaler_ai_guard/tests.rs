#![cfg(test)]

mod settings;
mod support;

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::base_builder;

use super::error::{ZscalerAiGuardError, ZscalerErrorDetail};
use super::metadata;
use super::printed::{quote_text, repr_text, str_text};
use super::{SUPPORTED_EVENT_HOOKS, ZscalerAiGuard};
use crate::generic_guardrail_contracts::GuardrailInputType;
use support::{args, environment, guardrail, inputs, mock, request_data};

#[test]
fn supported_hooks_are_pre_call_and_post_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [super::CallHook::PreCall, super::CallHook::PostCall]
    );
}

#[test]
fn policy_id_comes_from_request_metadata_first() {
    let data = request_data(
        r#"{
            "metadata": {
                "zguard_policy_id": 1,
                "user_api_key_metadata": {"zguard_policy_id": 2},
                "team_metadata": {"zguard_policy_id": 3}
            }
        }"#,
    );
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(1));
}

#[test]
fn policy_id_comes_from_the_user_key_metadata_next() {
    let data = request_data(
        r#"{
            "metadata": {
                "user_api_key_metadata": {"zguard_policy_id": 2},
                "team_metadata": {"zguard_policy_id": 3}
            }
        }"#,
    );
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(2));
}

#[test]
fn policy_id_comes_from_the_team_metadata_next() {
    let data = request_data(r#"{"metadata": {"team_metadata": {"zguard_policy_id": 3}}}"#);
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(3));
}

#[test]
fn a_null_request_policy_id_still_wins_over_the_configured_id() {
    let data = request_data(
        r#"{"metadata": {"zguard_policy_id": null, "team_metadata": {"zguard_policy_id": 3}}}"#,
    );
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::Null);
}

#[test]
fn policy_id_defaults_to_the_configured_id() {
    let data = request_data(r#"{"metadata": {}}"#);
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(4));
}

#[test]
fn metadata_values_prefer_litellm_metadata_over_metadata() {
    let data = request_data(
        r#"{
            "litellm_metadata": {"user_api_key_alias": "from-litellm"},
            "metadata": {"user_api_key_alias": "from-metadata"}
        }"#,
    );
    let value = metadata::header_value(&data, "user_api_key_alias").unwrap();
    assert_eq!(value, "from-litellm");
}

#[test]
fn the_alias_falls_back_to_the_key_alias_in_litellm_metadata() {
    let data = request_data(r#"{"litellm_metadata": {"user_api_key_key_alias": "key-alias"}}"#);
    let value = metadata::header_value(&data, "user_api_key_alias").unwrap();
    assert_eq!(value, "key-alias");
}

#[test]
fn metadata_values_are_trimmed() {
    let data = request_data(r#"{"metadata": {"user_api_key_user_id": "  user-1  "}}"#);
    let value = metadata::header_value(&data, "user_api_key_user_id").unwrap();
    assert_eq!(value, "user-1");
}

#[test]
fn metadata_header_values_default_to_not_available() {
    let empty = request_data(r#"{"metadata": {"user_api_key_user_id": "   "}}"#);
    assert_eq!(
        metadata::header_value(&empty, "user_api_key_user_id").unwrap(),
        "N/A"
    );
    assert_eq!(
        metadata::header_value(&empty, "user_api_key_alias").unwrap(),
        "N/A"
    );
}

#[test]
fn metadata_non_object_sections_are_refused() {
    let data = request_data(r#"{"metadata": "not-an-object"}"#);
    let error = metadata::selected_policy_id(&data, 4).unwrap_err();
    assert!(matches!(
        error,
        ZscalerAiGuardError::MetadataShape { name: "metadata" }
    ));
}

#[tokio::test]
async fn no_texts_make_no_request_and_return_the_inputs() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let result = guardrail
        .apply_guardrail(
            inputs(&[]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(result, inputs(&[]));
    assert!(mock.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_request_carries_the_documented_headers_and_body() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let mut args = args();
    args.api_base = Some(format!("{}/v1/detection/execute-policy", mock.base));
    args.send_user_api_key_alias = Some(true);
    args.send_user_api_key_user_id = Some(true);
    args.send_user_api_key_team_id = Some(true);
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let guardrail =
        ZscalerAiGuard::new(args, &environment, base_builder().build().unwrap()).unwrap();
    let data = request_data(
        r#"{
            "metadata": {"user_api_key_metadata": {"user_api_key_user_id": "user-1"}},
            "litellm_metadata": {"user_api_key_alias": "alias-1", "user_api_key_team_id": "team-1"}
        }"#,
    );
    guardrail
        .apply_guardrail(
            inputs(&["hello", "world"]),
            &data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/detection/execute-policy");
    assert_eq!(
        seen[0]
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer oracle-key"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("user-api-key-alias")
            .unwrap()
            .to_str()
            .unwrap(),
        "alias-1"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("user-api-key-team-id")
            .unwrap()
            .to_str()
            .unwrap(),
        "team-1"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("user-api-key-user-id")
            .unwrap()
            .to_str()
            .unwrap(),
        "user-1"
    );
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello world", "policyId": 42})
    );
}

#[tokio::test]
async fn a_missing_api_key_sends_an_empty_bearer_token() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let mut args = args();
    args.api_base = Some(mock.base.clone());
    args.api_key = None;
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let guardrail =
        ZscalerAiGuard::new(args, &environment, base_builder().build().unwrap()).unwrap();
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    let authorization = seen[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(authorization.trim_end(), "Bearer");
}

#[tokio::test]
async fn the_policy_id_is_only_sent_when_it_is_at_least_one() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let mut args = args();
    args.api_base = Some(mock.base.clone());
    args.policy_id = Some(0);
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let guardrail =
        ZscalerAiGuard::new(args, &environment, base_builder().build().unwrap()).unwrap();
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello"})
    );
}

#[tokio::test]
async fn a_metadata_policy_id_overrides_the_configured_id() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 7}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello", "policyId": 7})
    );
}

#[tokio::test]
async fn a_null_metadata_policy_id_sends_no_policy_id() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": null}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello"})
    );
}

#[tokio::test]
async fn a_zero_metadata_policy_id_is_not_sent() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 0}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello"})
    );
}

#[test]
fn an_unsigned_zero_policy_id_is_not_sent() {
    let wire = metadata::policy_id_wire(&GuardrailValue::UnsignedInteger(0)).unwrap();
    assert_eq!(wire, None);
}

#[tokio::test]
async fn a_response_direction_is_out() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "DETECT"}"#).await;
    let guardrail = guardrail(&mock.base);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Response,
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "OUT", "content": "hello", "policyId": 42})
    );
}

#[tokio::test]
async fn a_block_answer_raises_400_with_the_transaction_and_detectors() {
    let mock = mock(
        200,
        r#"{
            "statusCode": 200,
            "action": "BLOCK",
            "transactionId": "txn-1",
            "detectorResponses": {
                "detector-b": {"action": "BLOCK"},
                "detector-a": {"action": "ALLOW"},
                "detector-c": {"action": "BLOCK"}
            }
        }"#,
    )
    .await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["attack"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Block {
            error: "Content blocked by Zscaler AI Guard: {'transactionId': 'txn-1', 'blockingDetectors': ['detector-b', 'detector-c']}".to_owned()
        }
    );
}

#[tokio::test]
async fn a_missing_transaction_id_prints_as_none() {
    let mock = mock(
        200,
        r#"{"statusCode": 200, "action": "BLOCK", "detectorResponses": {}}"#,
    )
    .await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["attack"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Block {
            error: "Content blocked by Zscaler AI Guard: {'transactionId': None, 'blockingDetectors': []}".to_owned()
        }
    );
}

async fn passing_answer(action: &str) {
    let reply = format!(r#"{{"statusCode": 200, "action": "{action}"}}"#);
    let mock = mock(200, &reply).await;
    let guardrail = guardrail(&mock.base);
    let result = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(result, inputs(&["hello"]));
}

#[tokio::test]
async fn an_allow_answer_passes() {
    passing_answer("ALLOW").await;
}

#[tokio::test]
async fn a_detect_answer_passes() {
    passing_answer("DETECT").await;
}

#[tokio::test]
async fn a_rate_limit_answer_is_a_500() {
    let mock = mock(429, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Rate limit reached. status_code: 429".to_owned()
        }
    );
}

#[tokio::test]
async fn a_server_error_is_a_500() {
    let mock = mock(503, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Service is unavailable (HTTP 503)".to_owned()
        }
    );
}

#[tokio::test]
async fn a_bad_status_code_in_the_answer_is_a_500() {
    let mock = mock(200, r#"{"statusCode": 400, "errorMsg": "bad policy"}"#).await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "statusCode in response: 400, errorMsg: bad policy".to_owned()
        }
    );
}

#[tokio::test]
async fn an_unknown_action_is_a_500() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "REVIEW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Action field in response is REVIEW, expecting 'ALLOW', 'BLOCK' or 'DETECT'"
                .to_owned()
        }
    );
}

#[tokio::test]
async fn any_other_success_status_keeps_its_status() {
    let mock = mock(201, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::CREATED);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Response status code: 201".to_owned()
        }
    );
}

#[tokio::test]
async fn a_client_error_is_a_500_like_litellms_http_client() {
    let mock = mock(404, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Response status code: 404".to_owned()
        }
    );
}

#[tokio::test]
async fn a_redirect_status_is_a_500_like_litellms_http_client() {
    let mock = mock(300, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Response status code: 300".to_owned()
        }
    );
}

#[tokio::test]
async fn an_invalid_json_answer_is_a_500() {
    let mock = mock(200, "not json").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::ResponseJson { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_transport_error_is_a_500() {
    let guardrail = guardrail("http://127.0.0.1:1");
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::Request { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn printed_text_is_a_string_or_the_value_repr() {
    assert_eq!(str_text(&GuardrailValue::Text("abc".to_owned())), "abc");
    assert_eq!(str_text(&GuardrailValue::SignedInteger(7)), "7");
    assert_eq!(str_text(&GuardrailValue::Null), "None");
    assert_eq!(str_text(&GuardrailValue::Boolean(true)), "True");
}

#[test]
fn printed_repr_renders_each_value_like_python() {
    assert_eq!(repr_text(&GuardrailValue::Null), "None");
    assert_eq!(repr_text(&GuardrailValue::Boolean(false)), "False");
    assert_eq!(repr_text(&GuardrailValue::SignedInteger(-3)), "-3");
    assert_eq!(repr_text(&GuardrailValue::UnsignedInteger(3)), "3");
    assert_eq!(repr_text(&GuardrailValue::Decimal(2.0)), "2.0");
    assert_eq!(repr_text(&GuardrailValue::Decimal(1.25)), "1.25");
    assert_eq!(repr_text(&GuardrailValue::Text("a".to_owned())), "'a'");
    assert_eq!(
        repr_text(&GuardrailValue::Array(vec![
            GuardrailValue::SignedInteger(1),
            GuardrailValue::Null,
        ])),
        "[1, None]"
    );
    let mut fields = BTreeMap::new();
    let _previous = fields.insert("k".to_owned(), GuardrailValue::Text("v".to_owned()));
    assert_eq!(repr_text(&GuardrailValue::Object(fields)), "{'k': 'v'}");
}

#[test]
fn printed_repr_quotes_and_escapes_strings() {
    assert_eq!(quote_text("plain"), "'plain'");
    assert_eq!(quote_text("a space"), "'a space'");
    assert_eq!(quote_text("it's"), "\"it's\"");
    assert_eq!(quote_text("a'b\"c"), r#"'a\'b"c'"#);
    assert_eq!(quote_text("a\\b\n\t"), r"'a\\b\n\t'");
    assert_eq!(quote_text("\u{7}"), r"'\x07'");
    assert_eq!(quote_text("\u{2028}"), r"'\u2028'");
    assert_eq!(quote_text("\u{e0001}"), r"'\U000e0001'");
}

#[test]
fn a_null_litellm_metadata_value_falls_through_to_metadata() {
    let data = request_data(
        r#"{"litellm_metadata": {"user_api_key_alias": null}, "metadata": {"user_api_key_alias": "from-metadata"}}"#,
    );
    assert_eq!(
        metadata::header_value(&data, "user_api_key_alias").unwrap(),
        "from-metadata"
    );
}

#[test]
fn a_null_key_alias_falls_through_to_metadata() {
    let data = request_data(
        r#"{"litellm_metadata": {"user_api_key_key_alias": null}, "metadata": {"user_api_key_alias": "from-metadata"}}"#,
    );
    assert_eq!(
        metadata::header_value(&data, "user_api_key_alias").unwrap(),
        "from-metadata"
    );
}

#[test]
fn a_null_metadata_value_is_not_available() {
    let data = request_data(r#"{"metadata": {"user_api_key_alias": null}}"#);
    assert_eq!(
        metadata::header_value(&data, "user_api_key_alias").unwrap(),
        "N/A"
    );
}

#[test]
fn the_key_alias_fallback_only_applies_to_the_alias() {
    let data = request_data(
        r#"{"litellm_metadata": {"user_api_key_key_alias": "key-alias"}, "metadata": {"user_api_key_team_id": "team-1"}}"#,
    );
    assert_eq!(
        metadata::header_value(&data, "user_api_key_team_id").unwrap(),
        "team-1"
    );
}

#[tokio::test]
async fn an_answer_under_the_size_limit_is_read() {
    let content = "x".repeat(3000);
    let reply =
        format!(r#"{{"statusCode": 200, "action": "ALLOW", "transactionId": "{content}"}}"#);
    let mock = mock(200, &reply).await;
    let guardrail = guardrail(&mock.base);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_decimal_200_status_code_is_accepted() {
    let mock = mock(200, r#"{"statusCode": 200.0, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_decimal_status_code_other_than_200_is_a_500() {
    let mock = mock(200, r#"{"statusCode": 400.0, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_decimal_policy_id_from_metadata_is_sent() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 5.0}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello", "policyId": 5.0})
    );
}

#[tokio::test]
async fn a_fractional_policy_id_below_one_is_not_sent() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 0.5}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello"})
    );
}

#[tokio::test]
async fn a_large_unsigned_policy_id_is_sent() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 18446744073709551615}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello", "policyId": 18446744073709551615_u64})
    );
}

#[tokio::test]
async fn inputs_pass_through_unchanged() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let mut given = inputs(&["hello"]);
    let _inserted = given.rest.insert(
        "images".to_owned(),
        serde_json::from_str(r#"["aGVsbG8="]"#).unwrap(),
    );
    let result = guardrail
        .apply_guardrail(
            given.clone(),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(result, given);
}
