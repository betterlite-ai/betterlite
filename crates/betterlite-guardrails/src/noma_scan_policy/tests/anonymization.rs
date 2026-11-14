use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;
use tracing_subscriber::fmt;

use crate::guardrail_records::record::GuardrailStatus;
use crate::noma_scan_policy::error::{NomaBlockedMessage, NomaError};

use super::support::{
    CapturedLog, USER_MESSAGES, anonymizing_settings, context, data, field, mock_noma,
    monitoring_settings, policy, policy_with, records, response, value,
};

#[tokio::test]
async fn blocked_verdict_returns_a_400_with_the_scan_result() {
    let reply = r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"harmfulContent": {"result": true}}}]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let original = request_data.clone();
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    let NomaError::Blocked(message) = error else {
        panic!("expected a blocked error");
    };
    assert_eq!(NomaBlockedMessage::STATUS_CODE.as_u16(), 400);
    assert_eq!(
        message.details,
        response(reply).scan_result.flatten().unwrap()
    );
    let mut detail = BTreeMap::new();
    detail.insert(
        "error".to_owned(),
        GuardrailValue::Text(String::from("Request blocked by Noma guardrail")),
    );
    detail.insert("details".to_owned(), message.details.clone());
    assert_eq!(message.detail(), GuardrailValue::Object(detail));
    assert_eq!(request_data, original);
    let records = records(&request_data);
    assert_eq!(
        field(&records[0], "guardrail_status"),
        Some(&GuardrailValue::Text(String::from("guardrail_intervened")))
    );
}

#[tokio::test]
async fn monitor_mode_logs_without_blocking_or_rewriting() {
    let reply = r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}, "anonymizedContent": {"anonymized": "clean"}}}]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let mut settings = monitoring_settings();
    settings.api_base = Some(mock.base.clone());
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let original = request_data.clone();
    let user_auth = data("{}");
    let extra_data = data("{}");
    let captured_log = CapturedLog::default();
    let writer = captured_log.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.clone())
        .finish();
    let outcome = {
        let _guard = tracing::subscriber::set_default(subscriber);
        policy
            .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
            .await
            .unwrap()
            .unwrap()
    };
    assert_eq!(outcome.status, GuardrailStatus::GuardrailIntervened);
    assert_eq!(outcome.anonymized, None);
    assert_eq!(request_data, original);
    let output = String::from_utf8(captured_log.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Noma guardrail blocked user message"));
    assert_eq!(
        field(&records(&request_data)[0], "guardrail_status"),
        Some(&GuardrailValue::Text(String::from("guardrail_intervened")))
    );
}

#[tokio::test]
async fn sensitive_data_anonymizes_the_last_user_message() {
    let reply = r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}, "anonymizedContent": {"anonymized": "clean"}}}]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let mut settings = anonymizing_settings();
    settings.api_base = Some(mock.base.clone());
    let policy = policy_with(settings);
    let mut request_data = data(
        r#"{"messages": [{"role": "user", "content": "first"}, {"role": "assistant", "content": "reply"}, {"role": "user", "content": "secret"}]}"#,
    );
    let user_auth = data("{}");
    let extra_data = data("{}");
    let outcome = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.anonymized.as_deref(), Some("clean"));
    let Some(GuardrailValue::Array(messages)) = request_data.get("messages") else {
        panic!("the request lost its messages");
    };
    assert_eq!(messages.len(), 3);
    let GuardrailValue::Object(last) = &messages[2] else {
        panic!("the last message is not an object");
    };
    assert_eq!(
        last.get("content"),
        Some(&GuardrailValue::Text(String::from("clean")))
    );
    let GuardrailValue::Object(first) = &messages[0] else {
        panic!("the first message is not an object");
    };
    assert_eq!(
        first.get("content"),
        Some(&GuardrailValue::Text(String::from("first")))
    );
}

#[tokio::test]
async fn another_detector_blocks_despite_anonymized_content() {
    let reply = r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}, "harmfulContent": {"result": true}, "anonymizedContent": {"anonymized": "clean"}}}]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let mut settings = anonymizing_settings();
    settings.api_base = Some(mock.base.clone());
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(error, NomaError::Blocked(_)));
}

#[tokio::test]
async fn sensitive_data_without_an_anonymized_version_blocks() {
    let reply = r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}}}]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let mut settings = anonymizing_settings();
    settings.api_base = Some(mock.base.clone());
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(error, NomaError::Blocked(_)));
}

#[tokio::test]
async fn a_safe_verdict_without_anonymization_keeps_the_message() {
    let reply = r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let original = request_data.clone();
    let user_auth = data("{}");
    let extra_data = data("{}");
    let outcome = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.anonymized, None);
    assert_eq!(request_data, original);
}

#[tokio::test]
async fn anonymization_replaces_every_choice_with_content() {
    let reply = r#"{"aggregatedScanResult": true, "scanResult": [{"role": "assistant", "results": {"sensitiveData": {"PCI": {"result": true}}, "anonymizedContent": {"anonymized": "clean"}}}]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let mut settings = anonymizing_settings();
    settings.api_base = Some(mock.base.clone());
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let mut model_response = value(
        r#"{"id": "resp-1", "choices": [{"message": {"content": "one"}}, {"message": {"content": ""}}, {"message": {"content": "two"}}]}"#,
    );
    let user_auth = data("{}");
    let extra_data = data("{}");
    let outcome = policy
        .process_llm_response_check(
            &mut request_data,
            &mut model_response,
            context(&user_auth, &extra_data),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.anonymized.as_deref(), Some("clean"));
    let GuardrailValue::Object(fields) = &model_response else {
        panic!("the model response is not an object");
    };
    let Some(GuardrailValue::Array(choices)) = fields.get("choices") else {
        panic!("the model response lost its choices");
    };
    let contents: Vec<Option<&GuardrailValue>> = choices
        .iter()
        .map(|choice| {
            let GuardrailValue::Object(fields) = choice else {
                return None;
            };
            let Some(GuardrailValue::Object(message)) = fields.get("message") else {
                return None;
            };
            message.get("content")
        })
        .collect();
    assert_eq!(
        contents,
        vec![
            Some(&GuardrailValue::Text(String::from("clean"))),
            Some(&GuardrailValue::Text(String::new())),
            Some(&GuardrailValue::Text(String::from("clean"))),
        ]
    );
}

#[rstest]
#[case::non_list_scan_result(r#"{"aggregatedScanResult": false, "scanResult": {}}"#, "scanResult")]
#[case::non_object_item(r#"{"aggregatedScanResult": true, "scanResult": [7]}"#, "scanResult")]
#[case::non_object_results(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": 7}]}"#,
    "scanResult.results"
)]
#[case::non_object_item_when_safe(
    r#"{"aggregatedScanResult": false, "scanResult": [7]}"#,
    "scanResult"
)]
#[case::non_object_results_when_safe(
    r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": 7}]}"#,
    "scanResult.results"
)]
#[case::non_object_anonymized_content(
    r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": {"anonymizedContent": 7}}]}"#,
    "anonymizedContent"
)]
#[tokio::test]
async fn anonymization_shape_errors_name_the_field(
    #[case] reply: &str,
    #[case] field: &'static str,
) {
    let mock = mock_noma(StatusCode::OK, reply).await;
    let mut settings = anonymizing_settings();
    settings.api_base = Some(mock.base.clone());
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(error, NomaError::ResponseField { field: found } if found == field));
}

#[test]
fn replacing_a_user_message_handles_absent_and_bad_shapes() {
    let mut absent = data("{}");
    crate::noma_scan_policy::messages::replace_last_user_message(&mut absent, "clean").unwrap();
    let mut null = data(r#"{"messages": null}"#);
    crate::noma_scan_policy::messages::replace_last_user_message(&mut null, "clean").unwrap();
    let mut assistant_only = data(r#"{"messages": [{"role": "assistant", "content": "x"}]}"#);
    crate::noma_scan_policy::messages::replace_last_user_message(&mut assistant_only, "clean")
        .unwrap();
    let mut not_a_list = data(r#"{"messages": "x"}"#);
    assert!(matches!(
        crate::noma_scan_policy::messages::replace_last_user_message(&mut not_a_list, "clean"),
        Err(NomaError::Field { field: "messages" })
    ));
    let mut bad_message = data(r#"{"messages": [7]}"#);
    assert!(matches!(
        crate::noma_scan_policy::messages::replace_last_user_message(&mut bad_message, "clean"),
        Err(NomaError::Field { field: "messages" })
    ));
}
