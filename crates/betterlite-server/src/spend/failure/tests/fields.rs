use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_models::decisions::Json;
use chrono::{DateTime, NaiveDate};
use rstest::rstest;
use rust_decimal::Decimal;

use super::super::fields::{
    CallLog, FailureCostError, completion_start, eligible_route, failure_response_cost,
    failure_start_time, lift_call_log,
};
use super::support::object;
use crate::spend::payload::CompletionStart;

fn aware(text: &str) -> SpendTimestamp {
    SpendTimestamp::Aware(DateTime::parse_from_rfc3339(text).expect("the fixture is rfc3339"))
}

#[rstest]
#[case::a_falsey_object_is_replaced(
    r#"{"standard_logging_object": {}}"#,
    Some(r#"{"id": "call"}"#),
    r#"{"id": "call"}"#
)]
#[case::a_null_object_is_replaced(r#"{"standard_logging_object": null}"#, None, "null")]
#[case::a_truthy_object_stays(
    r#"{"standard_logging_object": {"id": "kept"}}"#,
    Some(r#"{"id": "call"}"#),
    r#"{"id": "kept"}"#
)]
fn lift_fills_a_falsey_standard_logging_object(
    #[case] request: &str,
    #[case] logged: Option<&str>,
    #[case] expected: &str,
) {
    let mut request_data = object(request);
    let call_log = CallLog {
        standard_logging_object: logged.map(object).map(Json::Object),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..CallLog::default()
    };
    lift_call_log(&mut request_data, &call_log);
    let expected_value = if expected == "null" {
        Json::Null
    } else {
        Json::Object(object(expected))
    };
    assert_eq!(request_data["standard_logging_object"], expected_value);
}

#[test]
fn lift_fills_a_null_trace_id() {
    let mut request_data = object(r#"{"litellm_trace_id": null}"#);
    lift_call_log(&mut request_data, &CallLog::default());
    assert_eq!(request_data["litellm_trace_id"], Json::Null);
    let logged = CallLog {
        litellm_trace_id: Some("trace".to_owned()),
        // ast-grep-ignore: no-default-struct-update - the fixture sets only the fields it reads
        ..CallLog::default()
    };
    lift_call_log(&mut request_data, &logged);
    assert_eq!(
        request_data["litellm_trace_id"],
        Json::Text("trace".to_owned())
    );
    request_data.insert("litellm_trace_id".to_owned(), Json::Text("kept".to_owned()));
    lift_call_log(&mut request_data, &logged);
    assert_eq!(
        request_data["litellm_trace_id"],
        Json::Text("kept".to_owned())
    );
    let mut absent = object("{}");
    lift_call_log(&mut absent, &CallLog::default());
    assert_eq!(absent["litellm_trace_id"], Json::Null);
}

#[rstest]
#[case::a_datetime_request_field_wins(
    r#"{"start_time": "2024-01-01T00:00:00Z"}"#,
    Some("2023-06-01T00:00:00Z"),
    "2024-01-01T00:00:00Z"
)]
#[case::a_text_field_is_not_a_datetime(
    r#"{"start_time": "yesterday"}"#,
    Some("2023-06-01T00:00:00Z"),
    "2023-06-01T00:00:00Z"
)]
#[case::logging_start_is_the_fallback("{}", Some("2023-06-01T00:00:00Z"), "2023-06-01T00:00:00Z")]
#[case::now_is_the_last_resort("{}", None, "2024-09-09T09:09:09Z")]
fn start_time_prefers_the_request_then_the_call_log_then_now(
    #[case] request: &str,
    #[case] logged: Option<&str>,
    #[case] expected: &str,
) {
    let now = aware("2024-09-09T09:09:09Z");
    let recorded = CallLog {
        start_time: logged.map(aware),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..CallLog::default()
    };
    let call_log = logged.is_some().then_some(&recorded);
    let start = failure_start_time(&object(request), call_log, now);
    assert_eq!(start, aware(expected));
}

#[test]
fn a_naive_request_start_time_is_read_as_naive() {
    let request = object(r#"{"start_time": "2024-01-01T00:00:00.5"}"#);
    let start = failure_start_time(&request, None, aware("2024-09-09T09:09:09Z"));
    let naive = NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("the date exists")
        .and_hms_milli_opt(0, 0, 0, 500)
        .expect("the time exists");
    assert_eq!(start, SpendTimestamp::Naive(naive));
}

#[rstest]
#[case::absent("{}", "absent")]
#[case::null(r#"{"completion_start_time": null}"#, "null")]
#[case::a_datetime(r#"{"completion_start_time": "2024-01-01T00:00:00Z"}"#, "at")]
#[case::bad_text(r#"{"completion_start_time": "soon"}"#, "absent")]
#[case::a_number(r#"{"completion_start_time": 3}"#, "absent")]
fn completion_start_reads_only_a_datetime(#[case] request: &str, #[case] expected: &str) {
    let start = completion_start(&object(request));
    let matched = matches!(
        (start, expected),
        (CompletionStart::Absent, "absent")
            | (CompletionStart::Null, "null")
            | (CompletionStart::At(_), "at")
    );
    assert!(matched);
}

#[rstest]
#[case::no_usage(r#"{"response_cost": 9}"#, "0")]
#[case::usage_without_cost(r#"{"combined_usage_object": {}}"#, "0")]
#[case::a_typed_usage_carries_the_cost(
    r#"{"combined_usage_object": {}, "response_cost": 1.5}"#,
    "1.5"
)]
#[case::a_negative_cost_is_zero(r#"{"combined_usage_object": {}, "response_cost": -2}"#, "0")]
#[case::a_null_cost_is_zero(r#"{"combined_usage_object": {}, "response_cost": null}"#, "0")]
#[case::an_untyped_usage_carries_nothing(
    r#"{"combined_usage_object": "usage", "response_cost": 2}"#,
    "0"
)]
#[case::a_malformed_usage_carries_nothing(
    r#"{"combined_usage_object": {"prompt_tokens": "many"}, "response_cost": 2}"#,
    "0"
)]
fn response_cost_counts_only_a_typed_combined_usage(#[case] request: &str, #[case] expected: &str) {
    let cost = failure_response_cost(&object(request), None).expect("the cost reads");
    let expected_value = expected.parse::<Decimal>().expect("the fixture is decimal");
    assert_eq!(cost, expected_value);
}

#[test]
fn a_text_response_cost_is_an_error() {
    let request = object(r#"{"combined_usage_object": {}, "response_cost": "1.5"}"#);
    let error = failure_response_cost(&request, None).expect_err("a text cost is rejected");
    assert!(matches!(error, FailureCostError::ResponseCost));
}

#[rstest]
#[case::guardrail_charges_are_added(
    r#"{"standard_logging_guardrail_information": [{"guardrail_cost": 0.5}]}"#,
    "1.5"
)]
#[case::null_guardrail_charges_are_zero(r#"{"standard_logging_guardrail_information": null}"#, "1")]
#[case::an_absent_guardrail_object_charges_zero("{}", "1")]
#[case::a_negative_guardrail_cost_charges_zero(
    r#"{"standard_logging_guardrail_information": [{"guardrail_cost": -4}]}"#,
    "1"
)]
fn guardrail_charges_add_to_the_recovered_cost(#[case] metadata: &str, #[case] expected: &str) {
    let request = object(r#"{"combined_usage_object": {}, "response_cost": 1}"#);
    let metadata_bucket = object(metadata);
    let cost = failure_response_cost(&request, Some(&metadata_bucket)).expect("the cost reads");
    let expected_value = expected.parse::<Decimal>().expect("the fixture is decimal");
    assert_eq!(cost, expected_value);
}

#[test]
fn an_absent_metadata_bucket_charges_no_guardrail() {
    let request = object(r#"{"combined_usage_object": {}, "response_cost": 3}"#);
    let cost = failure_response_cost(&request, None).expect("the cost reads");
    assert_eq!(cost, Decimal::from(3_i32));
}

#[rstest]
#[case::a_null_route(None, true)]
#[case::a_chat_route(Some("/v1/chat/completions"), true)]
#[case::an_embeddings_route(Some("/v1/embeddings"), true)]
#[case::an_info_route(Some("/health"), true)]
#[case::another_route(Some("/v1/unknown"), false)]
fn only_llm_and_info_routes_are_eligible(#[case] route: Option<&str>, #[case] expected: bool) {
    assert_eq!(eligible_route(route), expected);
}
