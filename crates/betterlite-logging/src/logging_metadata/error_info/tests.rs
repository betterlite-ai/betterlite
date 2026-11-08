#![cfg(test)]

use betterlite_error::normalize::Normalizer;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use rust_decimal::Decimal;
use std::str::FromStr as _;

use super::ErrorInformationError;
use super::ExceptionLogging;
use super::PayloadErrorRequest;
use super::get_error_information;
use super::payload_error_information;
use crate::fields::payload::records::{DecimalValue, ErrorInformation};
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_metadata::params::MetadataMap;

fn facts(encoded: &str) -> ExceptionFacts {
    serde_json::from_str(encoded).expect("exception facts decode")
}

fn metadata(encoded: &str) -> MetadataMap {
    serde_json::from_str(encoded).expect("metadata decodes")
}

fn normalizer() -> Normalizer {
    Normalizer::new().expect("normalizer patterns compile")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redaction patterns compile")
}

fn context<'input>(
    exception: Option<&'input ExceptionFacts>,
    normalizer: &'input Normalizer,
) -> ExceptionLogging<'input> {
    ExceptionLogging {
        exception,
        log_client_error_tracebacks: false,
        expected_client_error: false,
        redactor: None,
        normalizer,
    }
}

fn logged_information(encoded: &str) -> ErrorInformation {
    let exception = facts(encoded);
    let normalizer = normalizer();
    get_error_information(&context(Some(&exception), &normalizer), None)
}

fn payload_request<'input>(
    context: &'input ExceptionLogging<'input>,
    metadata: &'input MetadataMap,
    error_str: Option<String>,
) -> PayloadErrorRequest<'input> {
    PayloadErrorRequest {
        context,
        metadata,
        error_str,
    }
}

fn field_count(information: &ErrorInformation) -> usize {
    let value = serde_json::to_value(information).expect("encodes");
    let json: Json = serde_json::from_value(value).expect("decodes");
    let Json::Object(fields) = json else {
        panic!("the record is an object");
    };
    fields.len()
}

#[test]
fn an_absent_exception_still_produces_the_declared_record() {
    let normalizer = normalizer();
    let information = get_error_information(&context(None, &normalizer), Some("given"));
    assert_eq!(information.error_code, Some(Some(String::new())));
    assert_eq!(information.error_class, Some(Some(String::new())));
    assert_eq!(information.llm_provider, Some(Some(String::new())));
    assert_eq!(information.traceback, Some(Some(String::from("given"))));
    assert_eq!(information.error_message, Some(Some(String::new())));
    assert_eq!(information.error_provider_request_id, Some(None));
    assert_eq!(information.error_rate_limit_category, Some(None));
    assert_eq!(information.error_rate_limit_type, Some(None));
    assert_eq!(information.error_budget_entity_type, Some(None));
    assert_eq!(information.error_budget_entity_id, Some(None));
    assert_eq!(information.error_budget_limit, Some(None));
    assert_eq!(information.error_budget_spend, Some(None));
    assert_eq!(information.normalized_error, Some(None));
    assert_eq!(field_count(&information), 13);
}

#[rstest]
#[case::code_wins(r#"{"class_name": "BadRequestError", "code": "418", "status_code": 400, "response_status_code": 401}"#, "418")]
#[case::the_string_none_is_not_a_code(
    r#"{"class_name": "BadRequestError", "code": "None", "status_code": 400}"#,
    "400"
)]
#[case::an_empty_code_falls_through(
    r#"{"class_name": "BadRequestError", "code": "", "status_code": 400}"#,
    "400"
)]
#[case::status_before_the_response(
    r#"{"class_name": "BadRequestError", "status_code": 404, "response_status_code": 401}"#,
    "404"
)]
#[case::the_response_status_is_last(
    r#"{"class_name": "BadRequestError", "response_status_code": 403}"#,
    "403"
)]
#[case::nothing_recorded(r#"{"class_name": "BadRequestError"}"#, "")]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str, #[case] expected: &str) {
    let exception = facts(encoded);
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), None);
    assert_eq!(information.error_code, Some(Some(expected.to_owned())));
}

#[test]
fn the_class_name_and_provider_are_copied() {
    let exception = facts(r#"{"class_name": "AuthenticationError", "llm_provider": "openai"}"#);
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), None);
    assert_eq!(
        information.error_class,
        Some(Some("AuthenticationError".to_owned()))
    );
    assert_eq!(information.llm_provider, Some(Some("openai".to_owned())));
}

#[test]
fn an_absent_provider_renders_as_an_empty_string() {
    let exception = facts(r#"{"class_name": "ValueError"}"#);
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), None);
    assert_eq!(information.llm_provider, Some(Some(String::new())));
}

#[rstest]
#[case::by_class(
    r#"{"class_name": "AuthenticationError"}"#,
    Some("401_AUTHENTICATION_FAILED")
)]
#[case::by_status(
    r#"{"class_name": "Other", "status_code": 502}"#,
    Some("500_PROVIDER_INTERNAL_ERROR")
)]
#[case::by_proxy_type(
    r#"{"class_name": "Other", "proxy_type": "budget_exceeded"}"#,
    Some("429_BUDGET_EXCEEDED")
)]
#[case::by_message(
    r#"{"class_name": "Other", "message": "the context window is too small"}"#,
    Some("400_CONTEXT_WINDOW_EXCEEDED")
)]
#[case::unreadable_status(r#"{"class_name": "Other", "status_code": 418}"#, Some("UNCLASSIFIED"))]
fn normalized_errors_follow_the_shared_classifier(
    #[case] encoded: &str,
    #[case] expected: Option<&str>,
) {
    let information = logged_information(encoded);
    assert_eq!(
        information.normalized_error,
        Some(expected.map(str::to_owned))
    );
}

#[test]
fn the_exception_traceback_is_appended_when_the_error_is_not_expected() {
    let exception =
        facts(r#"{"class_name": "ValueError", "traceback_lines": ["line-1\n", "line-2\n"]}"#);
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), Some("given"));
    assert_eq!(
        information.traceback,
        Some(Some(String::from("givenline-1\nline-2\n")))
    );
}

#[rstest]
#[case::expected_and_not_logged(false, true, "given")]
#[case::expected_but_logged(true, true, "givenline-1\n")]
#[case::unexpected_and_not_logged(false, false, "givenline-1\n")]
#[case::unexpected_but_logged(true, false, "givenline-1\n")]
fn traceback_append_depends_on_the_policy(
    #[case] log_client_error_tracebacks: bool,
    #[case] expected_client_error: bool,
    #[case] expected: &str,
) {
    let exception = facts(r#"{"class_name": "ValueError", "traceback_lines": ["line-1\n"]}"#);
    let normalizer = normalizer();
    let mut context = context(Some(&exception), &normalizer);
    context.log_client_error_tracebacks = log_client_error_tracebacks;
    context.expected_client_error = expected_client_error;
    let information = get_error_information(&context, Some("given"));
    assert_eq!(information.traceback, Some(Some(expected.to_owned())));
}

fn numbered_lines(count: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for index in 0..count {
        lines.push(format!("line-{index}\n"));
    }
    lines
}

#[test]
fn at_most_one_hundred_traceback_lines_are_kept() {
    let lines = serde_json::to_string(&numbered_lines(101)).expect("encodes");
    let encoded = format!(r#"{{"class_name": "ValueError", "traceback_lines": {lines}}}"#);
    let exception = facts(&encoded);
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), None);
    let Some(Some(traceback)) = information.traceback else {
        panic!("a traceback is recorded");
    };
    assert!(traceback.contains("line-99\n"));
    assert!(!traceback.contains("line-100\n"));
}

#[rstest]
#[case::known_category(
    r#"{"class_name": "RateLimitError", "category": "litellm_rate_limit"}"#,
    Some("litellm_rate_limit")
)]
#[case::unknown_category(r#"{"class_name": "RateLimitError", "category": "third_party"}"#, None)]
fn rate_limit_categories_are_validated(#[case] encoded: &str, #[case] category: Option<&str>) {
    let information = logged_information(encoded);
    assert_eq!(
        information.error_rate_limit_category,
        Some(category.map(str::to_owned))
    );
}

#[rstest]
#[case::known_type(
    r#"{"class_name": "RateLimitError", "rate_limit_type": "budget"}"#,
    Some("budget")
)]
#[case::unknown_type(r#"{"class_name": "RateLimitError", "rate_limit_type": "cpu"}"#, None)]
fn rate_limit_types_are_validated(#[case] encoded: &str, #[case] limit_type: Option<&str>) {
    let information = logged_information(encoded);
    assert_eq!(
        information.error_rate_limit_type,
        Some(limit_type.map(str::to_owned))
    );
}

#[test]
fn budget_errors_carry_the_budget_fields() {
    let exception = facts(
        r#"{"class_name": "BudgetExceededError", "budget_entity_type": "key", "budget_entity_id": "key-1", "budget_limit": 10.5, "budget_spend": 12.25}"#,
    );
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), None);
    assert_eq!(
        information.error_budget_entity_type,
        Some(Some("key".to_owned()))
    );
    assert_eq!(
        information.error_budget_entity_id,
        Some(Some("key-1".to_owned()))
    );
    assert_eq!(
        information.error_budget_limit,
        Some(Some(DecimalValue(
            Decimal::from_str("10.5").expect("decimal")
        )))
    );
    assert_eq!(
        information.error_budget_spend,
        Some(Some(DecimalValue(
            Decimal::from_str("12.25").expect("decimal")
        )))
    );
    assert_eq!(
        information.normalized_error,
        Some(Some("429_BUDGET_EXCEEDED".to_owned()))
    );
}

#[test]
fn a_subclass_keeps_its_own_class_name_and_the_base_semantics() {
    let exception = facts(
        r#"{"class_name": "TenantBudgetError", "class": "BudgetExceededError", "budget_entity_type": "team"}"#,
    );
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), None);
    assert_eq!(
        information.error_class,
        Some(Some("TenantBudgetError".to_owned()))
    );
    assert_eq!(
        information.error_budget_entity_type,
        Some(Some("team".to_owned()))
    );
    assert_eq!(
        information.normalized_error,
        Some(Some("429_BUDGET_EXCEEDED".to_owned()))
    );
}

#[test]
fn a_non_budget_error_keeps_its_budget_attributes_out_of_the_record() {
    let exception = facts(r#"{"class_name": "RateLimitError", "budget_entity_type": "key"}"#);
    let normalizer = normalizer();
    let information = get_error_information(&context(Some(&exception), &normalizer), None);
    assert_eq!(information.error_budget_entity_type, Some(None));
}

#[rstest]
#[case::headers(
    r#"{"class_name": "ValueError", "headers": {"x-request-id": "req-1"}}"#,
    Some("req-1")
)]
#[case::response_headers(
    r#"{"class_name": "ValueError", "response_headers": {"x-ms-request-id": "req-2"}}"#,
    Some("req-2")
)]
#[case::litellm_headers(
    r#"{"class_name": "ValueError", "litellm_response_headers": {"cf-ray": "req-3"}}"#,
    Some("req-3")
)]
#[case::header_priority_wins(r#"{"class_name": "ValueError", "headers": {"x-request-id": "req-1", "x-amzn-requestid": "req-0"}}"#, Some("req-0"))]
#[case::source_order_within_one_name(r#"{"class_name": "ValueError", "headers": {"x-request-id": "req-1"}, "litellm_response_headers": {"x-amzn-requestid": "req-0"}}"#, Some("req-0"))]
#[case::names_ignore_case(
    r#"{"class_name": "ValueError", "headers": {"X-Request-Id": "req-1"}}"#,
    Some("req-1")
)]
#[case::empty_values_are_skipped(
    r#"{"class_name": "ValueError", "headers": {"x-request-id": "", "cf-ray": "req-3"}}"#,
    Some("req-3")
)]
#[case::nothing_recorded(r#"{"class_name": "ValueError"}"#, None)]
fn provider_request_ids_are_found_in_header_sources(
    #[case] encoded: &str,
    #[case] expected: Option<&str>,
) {
    let found = logged_information(encoded)
        .error_provider_request_id
        .flatten();
    assert_eq!(found.as_deref(), expected);
}

#[test]
fn secrets_are_redacted_from_the_message_and_traceback() {
    let exception = facts(
        r#"{"class_name": "ValueError", "message": "sk-abcdefghijklmno", "traceback_lines": ["sk-abcdefghijklmno"]}"#,
    );
    let normalizer = normalizer();
    let redactor = redactor();
    let mut context = context(Some(&exception), &normalizer);
    context.redactor = Some(&redactor);
    let information = get_error_information(&context, None);
    assert_eq!(information.error_message, Some(Some("REDACTED".to_owned())));
    assert_eq!(information.traceback, Some(Some("REDACTED".to_owned())));
}

#[test]
fn normalization_reads_the_unredacted_message() {
    let exception = facts(
        r#"{"class_name": "ValueError", "message": "budget has been exceeded sk-abcdefghijklmno"}"#,
    );
    let normalizer = normalizer();
    let redactor = redactor();
    let mut context = context(Some(&exception), &normalizer);
    context.redactor = Some(&redactor);
    let information = get_error_information(&context, None);
    let Some(Some(redacted)) = information.error_message else {
        panic!("a message is recorded");
    };
    assert_ne!(redacted, "budget has been exceeded sk-abcdefghijklmno");
    assert_eq!(
        information.normalized_error,
        Some(Some("429_BUDGET_EXCEEDED".to_owned()))
    );
}

#[test]
fn a_client_disconnect_alone_keeps_the_original_error_string() {
    let normalizer = normalizer();
    let context = context(None, &normalizer);
    let metadata = metadata("{}");
    let (_, error_str) = payload_error_information(&payload_request(
        &context,
        &metadata,
        Some("boom".to_owned()),
    ))
    .expect("payload error information");
    assert_eq!(error_str, Some("boom".to_owned()));
}

#[test]
fn a_client_disconnect_replaces_the_record_and_the_message() {
    let normalizer = normalizer();
    let context = context(None, &normalizer);
    let metadata = metadata(r#"{"client_disconnected": true}"#);
    let (information, error_str) =
        payload_error_information(&payload_request(&context, &metadata, None))
            .expect("payload error information");
    assert_eq!(information.error_code, Some(Some("499".to_owned())));
    assert_eq!(
        information.error_class,
        Some(Some("ClientDisconnected".to_owned()))
    );
    assert_eq!(
        information.error_message,
        Some(Some("Client disconnected the request".to_owned()))
    );
    assert_eq!(field_count(&information), 3);
    assert_eq!(
        error_str,
        Some("Client disconnected the request".to_owned())
    );
}

#[test]
fn a_recorded_disconnect_record_is_used_as_is() {
    let normalizer = normalizer();
    let context = context(None, &normalizer);
    let metadata = metadata(
        r#"{"client_disconnected": true, "error_information": {"error_code": "499", "error_class": "ClientDisconnected", "error_message": "recorded"}}"#,
    );
    let (information, error_str) = payload_error_information(&payload_request(
        &context,
        &metadata,
        Some("kept".to_owned()),
    ))
    .expect("payload error information");
    assert_eq!(information.error_message, Some(Some("recorded".to_owned())));
    assert_eq!(error_str, Some("kept".to_owned()));
}

#[test]
fn a_disconnect_flag_that_is_not_a_bool_is_refused() {
    let normalizer = normalizer();
    let context = context(None, &normalizer);
    let metadata = metadata(r#"{"client_disconnected": "yes"}"#);
    let error = payload_error_information(&payload_request(&context, &metadata, None))
        .expect_err("wrong type");
    assert!(matches!(
        error,
        ErrorInformationError::ClientDisconnectedType
    ));
}

#[test]
fn a_false_disconnect_flag_ignores_a_recorded_record() {
    let normalizer = normalizer();
    let context = context(None, &normalizer);
    let metadata = metadata(
        r#"{"client_disconnected": false, "error_information": {"error_code": "499", "error_class": "ClientDisconnected", "error_message": "recorded"}}"#,
    );
    let (information, _) = payload_error_information(&payload_request(&context, &metadata, None))
        .expect("payload error information");
    assert_eq!(information.error_message, Some(Some(String::new())));
}
