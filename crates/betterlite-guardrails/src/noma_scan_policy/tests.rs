#![cfg(test)]

#[path = "tests/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use reqwest::StatusCode;
use rstest::rstest;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::config::{NomaGuardrailConfigModel, NomaV2GuardrailConfigModel, default_use_v2};
use super::error::{NomaBlockedMessage, NomaError};
use super::{DEFAULT_APPLICATION_ID, MessageRole, NomaScanPolicy, NomaSettings, ScanCallContext};
use crate::guardrail_records::record::GuardrailStatus;

use self::support::{
    SAFE_REPLY, captured, captured_headers, captured_path, context, data, mock_noma, parts, policy,
    policy_with, records, request_count, response, settings, slow_noma, text_field, ticking_policy,
    value,
};

const USER_MESSAGES: &str = r#"{"messages": [{"role": "user", "content": "hello"}]}"#;

fn anonymizing_settings() -> NomaSettings {
    let mut settings = settings();
    settings.anonymize_input = true;
    settings
}

fn monitoring_settings() -> NomaSettings {
    let mut settings = settings();
    settings.monitor_mode = true;
    settings.anonymize_input = true;
    settings
}

fn response_of(record: &GuardrailValue) -> &GuardrailValue {
    let GuardrailValue::Object(fields) = record else {
        panic!("the record is not an object");
    };
    let Some(scan_response) = fields.get("guardrail_response") else {
        panic!("the record has no guardrail_response");
    };
    scan_response
}

#[test]
fn config_models_carry_litellm_fields_and_names() {
    assert_eq!(
        NomaGuardrailConfigModel::ui_friendly_name(),
        "Noma Security"
    );
    assert_eq!(
        NomaV2GuardrailConfigModel::ui_friendly_name(),
        "Noma Security v2"
    );
    assert_eq!(default_use_v2(), Some(false));
    let legacy: NomaGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(legacy.use_v2, Some(false));
    assert_eq!(legacy.api_key, None);
    assert_eq!(legacy.api_base, None);
    assert_eq!(legacy.application_id, None);
    let explicit: NomaGuardrailConfigModel = serde_json::from_str(
        r#"{"use_v2": true, "api_key": "key", "api_base": "https://base/", "application_id": "app"}"#,
    )
    .unwrap();
    assert_eq!(explicit.use_v2, Some(true));
    assert_eq!(explicit.api_key.as_deref(), Some("key"));
    assert_eq!(explicit.api_base.as_deref(), Some("https://base/"));
    assert_eq!(explicit.application_id.as_deref(), Some("app"));
    let null: NomaGuardrailConfigModel = serde_json::from_str(r#"{"use_v2": null}"#).unwrap();
    assert_eq!(null.use_v2, None);
    let v2: NomaV2GuardrailConfigModel = serde_json::from_str(
        r#"{"api_key": "key", "api_base": "https://base/", "application_id": "app", "gateway_name": "gw", "monitor_mode": true, "block_failures": false}"#,
    )
    .unwrap();
    assert_eq!(v2.api_key.as_deref(), Some("key"));
    assert_eq!(v2.api_base.as_deref(), Some("https://base/"));
    assert_eq!(v2.application_id.as_deref(), Some("app"));
    assert_eq!(v2.gateway_name.as_deref(), Some("gw"));
    assert_eq!(v2.monitor_mode, Some(true));
    assert_eq!(v2.block_failures, Some(false));
    let v2_default: NomaV2GuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(v2_default, NomaV2GuardrailConfigModel::default());
}

#[rstest]
#[case::missing(r#"{"scanResult": []}"#, GuardrailStatus::Success)]
#[case::explicit_false(r#"{"aggregatedScanResult": false}"#, GuardrailStatus::Success)]
#[case::explicit_true(
    r#"{"aggregatedScanResult": true}"#,
    GuardrailStatus::GuardrailIntervened
)]
#[case::explicit_null(
    r#"{"aggregatedScanResult": null}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case::string_value(
    r#"{"aggregatedScanResult": "yes"}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case::number_value(
    r#"{"aggregatedScanResult": 1}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
fn determine_guardrail_status_reads_the_aggregated_result(
    #[case] text: &str,
    #[case] expected: GuardrailStatus,
) {
    assert_eq!(
        NomaScanPolicy::determine_guardrail_status(&response(text)),
        expected
    );
}

#[rstest]
#[case::only_pii(
    r#"{"sensitiveData": {"PII": {"result": true}}, "harmfulContent": {"result": false}}"#,
    true
)]
#[case::data_detector(r#"{"dataDetector": {"SECRET": {"result": true}}}"#, true)]
#[case::another_detector_true(
    r#"{"sensitiveData": {"PII": {"result": true}}, "harmfulContent": {"result": true}}"#,
    false
)]
#[case::no_sensitive_data(
    r#"{"sensitiveData": {"PII": {"result": false}}, "harmfulContent": {"result": false}}"#,
    false
)]
#[case::empty_classification(r#"{}"#, false)]
#[case::nested_topic_true(
    r#"{"sensitiveData": {"PII": {"result": true}}, "customLlm": {"topic": {"result": true}}}"#,
    false
)]
#[case::anonymized_content_is_no_detector(
    r#"{"sensitiveData": {"PII": {"result": true}}, "anonymizedContent": {"anonymized": "x"}}"#,
    true
)]
#[case::string_result_is_no_failure(r#"{"sensitiveData": {"PII": {"result": "true"}}}"#, false)]
#[case::sensitive_value_not_an_object(r#"{"sensitiveData": true}"#, false)]
#[case::non_object_detector_is_ignored(
    r#"{"sensitiveData": {"PII": {"result": true}}, "harmfulContent": true}"#,
    true
)]
fn only_sensitive_data_detectors_allow_anonymization(
    #[case] classification: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        NomaScanPolicy::should_only_sensitive_data_failed(&data(classification)),
        expected
    );
}

#[rstest]
#[case::user(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#,
    MessageRole::User,
    Some("clean")
)]
#[case::assistant(
    r#"{"scanResult": [{"role": "assistant", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#,
    MessageRole::Assistant,
    Some("clean")
)]
#[case::other_role(
    r#"{"scanResult": [{"role": "assistant", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#,
    MessageRole::User,
    None
)]
#[case::missing_results(r#"{"scanResult": [{"role": "user"}]}"#, MessageRole::User, None)]
#[case::empty_anonymized(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": ""}}}]}"#,
    MessageRole::User,
    None
)]
#[case::no_scan_result(r#"{}"#, MessageRole::User, None)]
fn extracted_anonymized_content_comes_from_the_matching_role(
    #[case] text: &str,
    #[case] role: MessageRole,
    #[case] expected: Option<&str>,
) {
    let extracted = NomaScanPolicy::extract_anonymized_content(&response(text), role).unwrap();
    assert_eq!(extracted.as_deref(), expected);
}

#[rstest]
#[case::non_list_scan_result(r#"{"scanResult": {}}"#, "scanResult")]
#[case::non_object_item(r#"{"scanResult": [7]}"#, "scanResult")]
#[case::non_object_results(
    r#"{"scanResult": [{"role": "user", "results": 7}]}"#,
    "scanResult.results"
)]
#[case::non_object_anonymized_content(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": 7}}]}"#,
    "anonymizedContent"
)]
#[case::non_string_anonymized(
    r#"{"scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": 7}}}]}"#,
    "anonymizedContent.anonymized"
)]
fn mismatched_anonymized_content_shapes_name_the_field(
    #[case] text: &str,
    #[case] field: &'static str,
) {
    let error =
        NomaScanPolicy::extract_anonymized_content(&response(text), MessageRole::User).unwrap_err();
    assert!(matches!(error, NomaError::ResponseField { field: found } if found == field));
}

#[rstest]
#[case::safe(r#"{"aggregatedScanResult": false}"#, true)]
#[case::missing_verdict(r#"{}"#, true)]
#[case::null_verdict(r#"{"aggregatedScanResult": null}"#, true)]
#[case::only_sensitive(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}}}]}"#,
    true
)]
#[case::other_detector(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}, "harmfulContent": {"result": true}}}]}"#,
    false
)]
#[case::no_scan_result(r#"{"aggregatedScanResult": true}"#, false)]
#[case::empty_scan_result(r#"{"aggregatedScanResult": true, "scanResult": []}"#, false)]
#[case::non_list_scan_result(r#"{"aggregatedScanResult": true, "scanResult": {}}"#, false)]
#[case::other_role(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "assistant", "results": {"sensitiveData": {"PII": {"result": true}}}}]}"#,
    false
)]
#[case::missing_results(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user"}]}"#,
    false
)]
#[case::invalid_verdict(
    r#"{"aggregatedScanResult": "yes", "scanResult": [{"role": "user", "results": {"sensitiveData": {"PII": {"result": true}}}}]}"#,
    true
)]
fn should_anonymize_follows_the_verdict_and_the_detectors(
    #[case] text: &str,
    #[case] expected: bool,
) {
    let policy = policy_with(anonymizing_settings());
    assert_eq!(
        policy
            .should_anonymize(&response(text), MessageRole::User)
            .unwrap(),
        expected
    );
}

#[test]
fn monitor_mode_never_anonymizes() {
    let policy = policy_with(monitoring_settings());
    let reply = r#"{"aggregatedScanResult": false}"#;
    assert!(
        !policy
            .should_anonymize(&response(reply), MessageRole::User)
            .unwrap()
    );
}

#[test]
fn disabled_anonymization_never_applies() {
    let policy = policy_with(settings());
    let reply = r#"{"aggregatedScanResult": false}"#;
    assert!(
        !policy
            .should_anonymize(&response(reply), MessageRole::User)
            .unwrap()
    );
}

#[tokio::test]
async fn user_message_check_sends_responses_input_and_context() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(
        r#"{"messages": [{"role": "system", "content": "be nice"}, {"role": "user", "content": "hello"}], "litellm_call_id": "call-1", "metadata": {"requester_ip_address": "10.0.0.1", "headers": {"x-noma-application-id": "header-app"}}}"#,
    );
    let user_auth =
        data(r#"{"user_id": "user-1", "user_email": "a@example.com", "key_alias": "alias-app"}"#);
    let extra_data = data("{}");
    let outcome = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.status, GuardrailStatus::Success);
    assert_eq!(outcome.anonymized, None);
    assert_eq!(captured_path(&mock), "/ai-dr/v2/prompt/scan");
    let headers = captured_headers(&mock);
    assert_eq!(
        headers.get("authorization").unwrap().to_str().unwrap(),
        "Bearer test-api-key"
    );
    assert_eq!(
        headers.get("x-noma-request-id").unwrap().to_str().unwrap(),
        "call-1"
    );
    let seen = captured(&mock);
    assert_eq!(seen.input.len(), 2);
    assert_eq!(seen.input[0].kind, "message");
    assert_eq!(seen.input[0].role.as_deref(), Some("system"));
    let system_parts = parts(&seen.input[0]);
    assert_eq!(system_parts[0].kind, "input_text");
    assert_eq!(system_parts[0].text, "be nice");
    assert_eq!(seen.input[1].role.as_deref(), Some("user"));
    assert_eq!(parts(&seen.input[1])[0].text, "hello");
    assert_eq!(seen.context.application_id.as_deref(), Some("header-app"));
    assert_eq!(seen.context.ip_address.as_deref(), Some("10.0.0.1"));
    assert_eq!(seen.context.user_id.as_deref(), Some("a@example.com"));
    assert_eq!(seen.context.session_id.as_deref(), Some("call-1"));
    assert_eq!(seen.context.request_id, None);
}

#[rstest]
#[case::dynamic(
    r#"{"application_id": "dynamic-app"}"#,
    None,
    Some("config-app"),
    Some("alias-app"),
    "dynamic-app"
)]
#[case::header(
    "{}",
    Some("header-app"),
    Some("config-app"),
    Some("alias-app"),
    "header-app"
)]
#[case::configured("{}", None, Some("config-app"), Some("alias-app"), "config-app")]
#[case::alias("{}", None, None, Some("alias-app"), "alias-app")]
#[case::default("{}", None, None, None, DEFAULT_APPLICATION_ID)]
#[tokio::test]
async fn application_id_follows_litellms_precedence(
    #[case] extra: &str,
    #[case] header: Option<&str>,
    #[case] configured: Option<&str>,
    #[case] alias: Option<&str>,
    #[case] expected: &str,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.application_id = configured.map(str::to_owned);
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    if let Some(header) = header {
        let metadata = format!(r#"{{"headers": {{"x-noma-application-id": "{header}"}}}}"#);
        request_data.insert(
            "metadata".to_owned(),
            GuardrailValue::Object(data(&metadata)),
        );
    }
    let mut user_auth: BTreeMap<String, GuardrailValue> = BTreeMap::new();
    if let Some(alias) = alias {
        user_auth.insert(
            "key_alias".to_owned(),
            GuardrailValue::Text(alias.to_owned()),
        );
    }
    let extra_data = data(extra);
    policy
        .process_user_message_check(
            &mut request_data,
            ScanCallContext {
                user_auth: &user_auth,
                extra_data: &extra_data,
                event_type: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        captured(&mock).context.application_id.as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::both(Some("test-api-key"), Some("call-1"), true, true)]
#[case::no_key(None, Some("call-1"), false, true)]
#[case::empty_key(Some(""), Some("call-1"), false, true)]
#[case::no_call_id(Some("test-api-key"), None, true, false)]
#[tokio::test]
async fn headers_follow_the_configured_key_and_call_id(
    #[case] api_key: Option<&str>,
    #[case] call_id: Option<&str>,
    #[case] has_authorization: bool,
    #[case] has_request_id: bool,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.api_key = api_key.map(|key| secrecy::SecretString::from(key.to_owned()));
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    if let Some(call_id) = call_id {
        request_data.insert(
            "litellm_call_id".to_owned(),
            GuardrailValue::Text(call_id.to_owned()),
        );
    }
    let user_auth = data("{}");
    let extra_data = data("{}");
    policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    let headers = captured_headers(&mock);
    assert_eq!(headers.contains_key("authorization"), has_authorization);
    assert_eq!(headers.contains_key("x-noma-request-id"), has_request_id);
}

#[rstest]
#[case::plain("")]
#[case::trailing_slash("/prefix/")]
#[case::prefix_without_slash("/prefix")]
#[tokio::test]
async fn endpoint_is_joined_like_urljoin(#[case] suffix: &str) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let base = format!("{}{suffix}", mock.base);
    let policy = policy(&base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    assert_eq!(captured_path(&mock), "/ai-dr/v2/prompt/scan");
}

#[rstest]
#[case::absent(r#"{}"#)]
#[case::null(r#"{"messages": null}"#)]
#[case::empty(r#"{"messages": []}"#)]
#[case::null_contents(r#"{"messages": [{"role": "user", "content": null}]}"#)]
#[tokio::test]
async fn requests_without_scannable_messages_skip_the_api(#[case] body: &str) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(body);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let outcome = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    assert_eq!(outcome, None);
    assert_eq!(request_count(&mock), 0);
}

#[tokio::test]
async fn a_scan_writes_a_standard_logging_record() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = ticking_policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data(r#"{"user_id": "user-1"}"#);
    let extra_data = data("{}");
    policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    let records = records(&request_data);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(
        text_field(record, "guardrail_name").as_deref(),
        Some("noma-test")
    );
    assert_eq!(
        text_field(record, "guardrail_provider").as_deref(),
        Some("noma")
    );
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("success")
    );
    assert_eq!(
        text_field(record, "guardrail_mode").as_deref(),
        Some("pre_call")
    );
    assert_eq!(
        support::number_field(record, "start_time"),
        Some(1_700_000_000.0)
    );
    assert_eq!(
        support::number_field(record, "end_time"),
        Some(1_700_000_001.0)
    );
    assert_eq!(support::number_field(record, "duration"), Some(1.0));
    assert_eq!(
        response_of(record),
        &GuardrailValue::Object(data(
            r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": {}}]}"#
        ))
    );
}

#[tokio::test]
async fn the_record_goes_into_litellm_metadata_when_present() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data =
        data(r#"{"messages": [{"role": "user", "content": "hello"}], "litellm_metadata": {}}"#);
    let user_auth = data("{}");
    let extra_data = data("{}");
    policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    assert!(request_data.get("metadata").is_none());
    assert_eq!(records(&request_data).len(), 1);
}

#[tokio::test]
async fn record_mode_falls_back_to_the_configured_event_hook() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.event_hook = Some(LitellmParamsMode::One(String::from("post_call")));
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    policy
        .process_user_message_check(
            &mut request_data,
            ScanCallContext {
                user_auth: &user_auth,
                extra_data: &extra_data,
                event_type: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        text_field(&records(&request_data)[0], "guardrail_mode").as_deref(),
        Some("post_call")
    );
}

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
        text_field(&records[0], "guardrail_status").as_deref(),
        Some("guardrail_intervened")
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
        text_field(&records(&request_data)[0], "guardrail_status").as_deref(),
        Some("guardrail_intervened")
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

#[rstest]
#[case::not_json("not json", "json")]
#[case::not_an_object("[]", "shape")]
#[tokio::test]
async fn unreadable_answers_are_reported(#[case] reply: &str, #[case] kind: &str) {
    let mock = mock_noma(StatusCode::OK, reply).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    if kind == "json" {
        assert!(matches!(error, NomaError::ResponseJson { .. }));
    } else {
        assert!(matches!(error, NomaError::ResponseShape { .. }));
    }
    assert!(records(&request_data).is_empty());
}

#[rstest]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR)]
#[case::client_error(StatusCode::BAD_REQUEST)]
#[tokio::test]
async fn api_status_failures_are_reported_and_write_no_record(#[case] status: StatusCode) {
    let mock = mock_noma(status, "{}").await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(error, NomaError::HttpStatus { status: actual } if actual == status));
    assert!(records(&request_data).is_empty());
}

#[tokio::test]
async fn the_configured_timeout_ends_a_slow_scan() {
    let mock = slow_noma(Duration::from_secs(30)).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.timeout = Some(Duration::from_millis(50));
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    let NomaError::Transport { source } = error else {
        panic!("expected a transport error");
    };
    assert!(source.is_timeout());
    assert!(records(&request_data).is_empty());
}

#[rstest]
#[case::messages_not_a_list(r#"{"messages": "hello"}"#, "messages")]
#[case::call_id_not_a_string(
    r#"{"messages": [{"role": "user", "content": "x"}], "litellm_call_id": 7}"#,
    "litellm_call_id"
)]
#[case::metadata_not_an_object(
    r#"{"messages": [{"role": "user", "content": "x"}], "metadata": "x"}"#,
    "metadata"
)]
#[case::headers_not_an_object(
    r#"{"messages": [{"role": "user", "content": "x"}], "metadata": {"headers": "x"}}"#,
    "metadata.headers"
)]
#[tokio::test]
async fn request_fields_of_the_wrong_type_name_the_field(
    #[case] body: &str,
    #[case] field: &'static str,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(body);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(error, NomaError::Field { field: found } if found == field));
    assert_eq!(request_count(&mock), 0);
}

#[tokio::test]
async fn a_message_that_is_not_a_chat_message_is_reported() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(r#"{"messages": [7]}"#);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(error, NomaError::Message { .. }));
}

#[tokio::test]
async fn a_non_string_user_field_names_the_field() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data(r#"{"user_email": 7}"#);
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        NomaError::Field {
            field: "user_email"
        }
    ));
}

#[rstest]
#[case::first_nonempty(
    r#"{"id": "resp-1", "choices": [{"message": {"content": "first"}}, {"message": {"content": "second"}}]}"#,
    "first"
)]
#[case::skips_empty(
    r#"{"id": "resp-1", "choices": [{"message": {"content": ""}}, {"message": {"content": "second"}}]}"#,
    "second"
)]
#[tokio::test]
async fn llm_response_check_scans_the_first_nonempty_content(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let mut model_response = value(body);
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
    assert_eq!(outcome.status, GuardrailStatus::Success);
    let seen = captured(&mock);
    assert_eq!(seen.input.len(), 1);
    assert_eq!(seen.input[0].role.as_deref(), Some("assistant"));
    assert_eq!(parts(&seen.input[0])[0].text, expected);
    assert_eq!(seen.context.request_id.as_deref(), Some("resp-1"));
}

#[rstest]
#[case::not_an_object(r#""text""#)]
#[case::no_choices(r#"{"id": "resp-1"}"#)]
#[case::empty_choices(r#"{"id": "resp-1", "choices": []}"#)]
#[case::empty_content(r#"{"id": "resp-1", "choices": [{"message": {"content": ""}}]}"#)]
#[case::null_content(r#"{"id": "resp-1", "choices": [{"message": {"content": null}}]}"#)]
#[case::no_content(r#"{"id": "resp-1", "choices": [{"message": {}}]}"#)]
#[tokio::test]
async fn responses_without_text_are_not_scanned(#[case] body: &str) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let mut model_response = value(body);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let outcome = policy
        .process_llm_response_check(
            &mut request_data,
            &mut model_response,
            context(&user_auth, &extra_data),
        )
        .await
        .unwrap();
    assert_eq!(outcome, None);
    assert_eq!(request_count(&mock), 0);
}

#[rstest]
#[case::choices_not_a_list(r#"{"id": "resp-1", "choices": "x"}"#)]
#[case::choice_not_an_object(r#"{"id": "resp-1", "choices": [7]}"#)]
#[case::message_not_an_object(r#"{"id": "resp-1", "choices": [{"message": 7}]}"#)]
#[case::content_not_a_string(r#"{"id": "resp-1", "choices": [{"message": {"content": 7}}]}"#)]
#[case::id_not_a_string(r#"{"id": 7, "choices": []}"#)]
#[tokio::test]
async fn malformed_responses_name_the_field(#[case] body: &str) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let mut model_response = value(body);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_llm_response_check(
            &mut request_data,
            &mut model_response,
            context(&user_auth, &extra_data),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        NomaError::ResponseField { .. } | NomaError::Field { .. }
    ));
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
#[case::non_object_anonymized_content(
    r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"anonymizedContent": 7}}]}"#,
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
    super::messages::replace_last_user_message(&mut absent, "clean").unwrap();
    let mut null = data(r#"{"messages": null}"#);
    super::messages::replace_last_user_message(&mut null, "clean").unwrap();
    let mut assistant_only = data(r#"{"messages": [{"role": "assistant", "content": "x"}]}"#);
    super::messages::replace_last_user_message(&mut assistant_only, "clean").unwrap();
    let mut not_a_list = data(r#"{"messages": "x"}"#);
    assert!(matches!(
        super::messages::replace_last_user_message(&mut not_a_list, "clean"),
        Err(NomaError::Field { field: "messages" })
    ));
    let mut bad_message = data(r#"{"messages": [7]}"#);
    assert!(matches!(
        super::messages::replace_last_user_message(&mut bad_message, "clean"),
        Err(NomaError::Field { field: "messages" })
    ));
}

#[rstest]
#[case::blocked(
    r#"{"aggregatedScanResult": true}"#,
    tracing::Level::WARN,
    "Noma guardrail blocked user message: hi"
)]
#[case::allowed(
    r#"{"aggregatedScanResult": false}"#,
    tracing::Level::INFO,
    "Noma guardrail allowed user message: hi"
)]
fn background_verdicts_are_logged(
    #[case] reply: &str,
    #[case] level: tracing::Level,
    #[case] expected: &str,
) {
    let captured_log = CapturedLog::default();
    let writer = captured_log.clone();
    let subscriber = fmt()
        .with_max_level(level)
        .with_writer(move || writer.clone())
        .finish();
    with_default(subscriber, || {
        NomaScanPolicy::handle_verdict_background(MessageRole::User, "hi", &response(reply));
    });
    let output = String::from_utf8(captured_log.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains(expected));
}

#[rstest]
#[case::blocked(
    r#"{"aggregatedScanResult": true}"#,
    tracing::Level::WARN,
    "Noma guardrail blocked user message: hi"
)]
#[case::allowed(
    r#"{"aggregatedScanResult": false}"#,
    tracing::Level::INFO,
    "Noma guardrail allowed user message: hi"
)]
fn monitor_mode_verdicts_are_logged_and_allowed(
    #[case] reply: &str,
    #[case] level: tracing::Level,
    #[case] expected: &str,
) {
    let policy = policy_with(monitoring_settings());
    let captured_log = CapturedLog::default();
    let writer = captured_log.clone();
    let subscriber = fmt()
        .with_max_level(level)
        .with_writer(move || writer.clone())
        .finish();
    with_default(subscriber, || {
        policy
            .check_verdict(MessageRole::User, "hi", &response(reply))
            .unwrap();
    });
    let output = String::from_utf8(captured_log.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains(expected));
}

#[derive(Clone, Default)]
struct CapturedLog(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
