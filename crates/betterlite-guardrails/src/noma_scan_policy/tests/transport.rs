use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;

use crate::guardrail_records::record::GuardrailStatus;
use crate::noma_scan_policy::error::NomaError;
use crate::noma_scan_policy::verdict::determine_guardrail_status;
use crate::noma_scan_policy::wire::ScanResponse;
use crate::noma_scan_policy::{DEFAULT_APPLICATION_ID, ScanCallContext};

use super::support::{
    SAFE_REPLY, USER_MESSAGES, captured, captured_headers, captured_path, context, corpus_answer,
    data, mock_noma, parts, policy, policy_with, records, request_count, settings, value,
};

#[tokio::test]
async fn user_message_check_sends_responses_input_and_context() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.application_id = configured.map(str::to_owned);
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    if let Some(sent) = header {
        let metadata = format!(r#"{{"headers": {{"x-noma-application-id": "{sent}"}}}}"#);
        request_data.insert(
            "metadata".to_owned(),
            GuardrailValue::Object(data(&metadata)),
        );
    }
    let mut user_auth: BTreeMap<String, GuardrailValue> = BTreeMap::new();
    if let Some(name) = alias {
        user_auth.insert(
            "key_alias".to_owned(),
            GuardrailValue::Text(name.to_owned()),
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
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.api_key = api_key.map(|key| secrecy::SecretString::from(key.to_owned()));
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    if let Some(sent) = call_id {
        request_data.insert(
            "litellm_call_id".to_owned(),
            GuardrailValue::Text(sent.to_owned()),
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
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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
#[case::absent("{}")]
#[case::null(r#"{"messages": null}"#)]
#[case::empty(r#"{"messages": []}"#)]
#[case::null_contents(r#"{"messages": [{"role": "user", "content": null}]}"#)]
#[tokio::test]
async fn requests_without_scannable_messages_skip_the_api(#[case] body: &str) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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

#[rstest]
#[case::not_json("not json", "json")]
#[case::not_an_object("[]", "shape")]
#[tokio::test]
async fn unreadable_answers_are_reported(#[case] reply: &str, #[case] kind: &str) {
    let mock = mock_noma(StatusCode::OK, reply, None).await;
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
    let mock = mock_noma(status, "{}", None).await;
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

#[rstest]
#[case::configured(Some(Duration::from_millis(50)), Duration::from_secs(30))]
#[case::unset(None, Duration::from_secs(700))]
#[tokio::test(start_paused = true)]
async fn a_stalled_scan_ends_at_its_deadline(
    #[case] timeout: Option<Duration>,
    #[case] delay: Duration,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, Some(delay)).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.timeout = timeout;
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

#[test]
fn replays_the_noma_scan_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/noma_scan_response");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        ScanResponse::from_body(&bytes).is_ok()
    }));
}

#[test]
fn the_corpus_extra_fields_ride_along_in_rest() {
    let answer = corpus_answer("extra_fields.json");
    assert_eq!(
        answer.rest.get("provider"),
        Some(&value(r#"{"id": "abc"}"#))
    );
    assert_eq!(answer.rest.get("extra"), Some(&GuardrailValue::Null));
    assert_eq!(
        answer.aggregated_scan_result,
        Some(Some(GuardrailValue::Text(String::from("yes"))))
    );
    assert_eq!(
        answer.scan_result,
        Some(Some(value(r#"{"unexpected": [1, 2.5, "x"]}"#)))
    );
}

#[test]
fn the_corpus_null_fields_stay_explicit_nulls() {
    let answer = corpus_answer("null_fields.json");
    assert_eq!(answer.aggregated_scan_result, Some(None));
    assert_eq!(answer.scan_result, Some(None));
    assert!(answer.rest.is_empty());
}

#[test]
fn the_corpus_repeated_key_keeps_the_last_value() {
    let answer = corpus_answer("repeated_key.json");
    assert_eq!(
        answer.aggregated_scan_result,
        Some(Some(GuardrailValue::Boolean(false)))
    );
    assert_eq!(
        determine_guardrail_status(&answer),
        GuardrailStatus::Success
    );
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
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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
#[case::after_a_non_chat_choice(
    r#"{"id": "resp-1", "choices": [{"delta": {"content": "skip"}}, {"message": {"content": "scan me"}}]}"#,
    "scan me"
)]
#[tokio::test]
async fn llm_response_check_scans_the_first_nonempty_content(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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
#[case::choices_not_a_list(r#"{"id": "resp-1", "choices": "x"}"#)]
#[case::choice_not_an_object(r#"{"id": "resp-1", "choices": [7]}"#)]
#[case::message_not_an_object(r#"{"id": "resp-1", "choices": [{"message": 7}]}"#)]
#[case::null_message(r#"{"id": "resp-1", "choices": [{"message": null}]}"#)]
#[case::text_completion(r#"{"id": "resp-1", "choices": [{"text": "hi"}]}"#)]
#[case::streamed_delta(r#"{"id": "resp-1", "choices": [{"delta": {"content": "hi"}}]}"#)]
#[tokio::test]
async fn responses_without_text_are_not_scanned(#[case] body: &str) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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
    assert_eq!(model_response, value(body));
}

#[rstest]
#[case::content_not_a_string(r#"{"id": "resp-1", "choices": [{"message": {"content": 7}}]}"#)]
#[case::id_not_a_string(r#"{"id": 7, "choices": []}"#)]
#[tokio::test]
async fn malformed_responses_name_the_field(#[case] body: &str) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
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

#[rstest]
#[case::top_level_nulls(
    r#"{"messages": [{"role": "user", "content": "hello"}], "litellm_call_id": null, "metadata": null}"#,
    r#"{"application_id": null}"#
)]
#[case::null_header(
    r#"{"messages": [{"role": "user", "content": "hello"}], "metadata": {"headers": {"x-noma-application-id": null}}}"#,
    "{}"
)]
#[tokio::test]
async fn null_request_fields_fall_through_like_absent_ones(
    #[case] body: &str,
    #[case] extra: &str,
) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY, None).await;
    let policy = policy(&mock.base);
    let mut request_data = data(body);
    let user_auth = data(r#"{"user_email": null, "user_id": "user-1"}"#);
    let extra_data = data(extra);
    policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    let seen = captured(&mock);
    assert_eq!(seen.context.application_id.as_deref(), Some("test-app"));
    assert_eq!(seen.context.user_id.as_deref(), Some("user-1"));
    assert_eq!(seen.context.session_id, None);
    assert_eq!(seen.context.ip_address, None);
    assert!(!captured_headers(&mock).contains_key("x-noma-request-id"));
}

#[tokio::test]
async fn a_large_scan_answer_within_one_mebibyte_is_read() {
    let padding = "a".repeat(100_000);
    let reply = format!("{{\"aggregatedScanResult\": false, \"padding\": \"{padding}\"}}");
    let mock = mock_noma(StatusCode::OK, &reply, None).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let outcome = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.status, GuardrailStatus::Success);
}

#[tokio::test]
async fn a_scan_answer_over_one_mebibyte_is_refused() {
    let padding = "a".repeat(0x0010_0001);
    let reply = format!("{{\"aggregatedScanResult\": false, \"padding\": \"{padding}\"}}");
    let mock = mock_noma(StatusCode::OK, &reply, None).await;
    let policy = policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    let error = policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        NomaError::ResponseTooLarge { limit: 0x0010_0000 }
    ));
    assert!(records(&request_data).is_empty());
}
