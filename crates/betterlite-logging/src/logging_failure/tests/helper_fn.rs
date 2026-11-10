use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use std::str::FromStr as _;

use crate::logging_failure::FailureError;
use crate::logging_failure::events::payload_to_map;
use crate::logging_failure::helper_fn::failure_handler_helper_fn;
use crate::logging_failure::{FAILED_API_CALL, FailureRecord, FailureReport};
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;

use super::support::{
    Harness, build, exception, init, moment, object, params, payload, report, text,
};

struct Fixture {
    state: LoggingState,
    record: FailureRecord,
    context: CorrelationContext,
    harness: Harness,
}

fn fixture() -> Fixture {
    Fixture {
        state: build(init()),
        record: FailureRecord::default(),
        context: CorrelationContext::default(),
        harness: Harness::new(),
    }
}

fn with_headers(message: &str, headers: &[(&str, &str)]) -> FailureReport {
    let mut facts = exception(message);
    let mut values = IndexMap::new();
    for (name, value) in headers {
        values.insert((*name).to_owned(), (*value).to_owned());
    }
    facts.headers = Some(values);
    FailureReport {
        exception: Arc::new(facts),
        traceback: "Traceback (most recent call last): sk-abcdefghijklmnop".to_owned(),
        start_time: None,
        end_time: None,
    }
}

#[test]
fn details_carry_the_failure_facts() {
    let mut fixture = fixture();
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let timing = failure_handler_helper_fn(
        &mut call,
        &report("provider exploded"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    assert_eq!(timing.start_time, moment("2026-01-01T00:00:00+00:00"));
    assert_eq!(timing.end_time, moment("2026-01-01T00:00:02+00:00"));
    let details = &fixture.state.model_call_details;
    assert_eq!(details.get("log_event_type"), Some(&text(FAILED_API_CALL)));
    assert_eq!(details.get("original_response"), Some(&Json::Null));
    assert_eq!(
        details.get("response_cost"),
        Some(&Json::Number(Number::from(0_i64)))
    );
    assert_eq!(
        details.get("end_time"),
        Some(&text("2026-01-01T00:00:02+00:00"))
    );
}

#[test]
fn tracebacks_are_redacted() {
    let mut fixture = fixture();
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let mut failure = report("provider exploded");
    failure.traceback = "Traceback: sk-abcdefghijklmnop".to_owned();
    failure_handler_helper_fn(&mut call, &failure, &fixture.harness.deps())
        .expect("the details are prepared");
    let traceback = match fixture.state.model_call_details.get("traceback_exception") {
        Some(Json::Text(value)) => value.clone(),
        other => panic!("the traceback should be text, got {other:?}"),
    };
    assert!(!traceback.contains("sk-abcdefghijklmnop"));
    assert!(traceback.contains("Traceback"));
}

#[test]
fn reported_start_and_end_times_win() {
    let mut fixture = fixture();
    let mut failure = report("provider exploded");
    failure.start_time = Some(moment("2026-01-02T00:00:00+00:00"));
    failure.end_time = Some(moment("2026-01-02T00:00:03+00:00"));
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let timing = failure_handler_helper_fn(&mut call, &failure, &fixture.harness.deps())
        .expect("the details are prepared");
    assert_eq!(timing.start_time, moment("2026-01-02T00:00:00+00:00"));
    assert_eq!(timing.end_time, moment("2026-01-02T00:00:03+00:00"));
}

#[test]
fn partial_usage_is_preserved() {
    let mut fixture = fixture();
    fixture.state.model_call_details.insert(
        "combined_usage_object".to_owned(),
        object(&[("prompt_tokens", Json::Number(Number::from(7_i64)))]),
    );
    fixture.state.model_call_details.insert(
        "response_cost".to_owned(),
        Json::Number(Number::from_str("0.5").expect("a number")),
    );
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    failure_handler_helper_fn(
        &mut call,
        &report("provider exploded"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    assert_eq!(
        fixture.state.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from_str("0.5").expect("a number")))
    );
}

#[test]
fn a_null_usage_object_still_zeroes_the_cost() {
    let mut fixture = fixture();
    fixture
        .state
        .model_call_details
        .insert("combined_usage_object".to_owned(), Json::Null);
    fixture.state.model_call_details.insert(
        "response_cost".to_owned(),
        Json::Number(Number::from(9_i64)),
    );
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    failure_handler_helper_fn(
        &mut call,
        &report("provider exploded"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    assert_eq!(
        fixture.state.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from(0_i64)))
    );
}

#[test]
fn an_original_response_is_kept() {
    let mut fixture = fixture();
    fixture
        .state
        .model_call_details
        .insert("original_response".to_owned(), text("the first attempt"));
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    failure_handler_helper_fn(
        &mut call,
        &report("provider exploded"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    assert_eq!(
        fixture.state.model_call_details.get("original_response"),
        Some(&text("the first attempt"))
    );
}

#[test]
fn exception_headers_are_merged_into_the_metadata() {
    let mut fixture = fixture();
    fixture.state.litellm_params = params(&[("metadata", object(&[("team_id", text("team-a"))]))]);
    fixture.state.model_call_details.insert(
        "litellm_params".to_owned(),
        Json::Object(fixture.state.litellm_params.clone()),
    );
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let failure = with_headers(
        "provider exploded",
        &[("x-request-id", "request-1"), ("team_id", "header-team")],
    );
    failure_handler_helper_fn(&mut call, &failure, &fixture.harness.deps())
        .expect("the details are prepared");
    let metadata = metadata_of(&fixture.state);
    assert_eq!(metadata.get("x-request-id"), Some(&text("request-1")));
    assert_eq!(metadata.get("team_id"), Some(&text("header-team")));
}

#[test]
fn missing_metadata_leaves_the_details_unchanged() {
    let mut fixture = fixture();
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let failure = with_headers("provider exploded", &[("x-request-id", "request-1")]);
    failure_handler_helper_fn(&mut call, &failure, &fixture.harness.deps())
        .expect("the details are prepared");
    assert_eq!(
        fixture.state.model_call_details.get("litellm_params"),
        Some(&object(&[]))
    );
}

#[test]
fn a_non_object_litellm_params_is_a_typed_error() {
    let mut fixture = fixture();
    fixture
        .state
        .model_call_details
        .insert("litellm_params".to_owned(), text("not-a-map"));
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let failure = with_headers("provider exploded", &[("x-request-id", "request-1")]);
    let error = failure_handler_helper_fn(&mut call, &failure, &fixture.harness.deps())
        .expect_err("the headers cannot be merged");
    assert!(matches!(error, FailureError::LitellmParamsNotObject));
    assert!(!fixture.record_marker());
}

#[test]
fn a_non_object_metadata_is_a_typed_error() {
    let mut fixture = fixture();
    fixture.state.model_call_details.insert(
        "litellm_params".to_owned(),
        object(&[("metadata", text("not-a-map"))]),
    );
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let failure = with_headers("provider exploded", &[("x-request-id", "request-1")]);
    let error = failure_handler_helper_fn(&mut call, &failure, &fixture.harness.deps())
        .expect_err("the headers cannot be merged");
    assert!(matches!(error, FailureError::MetadataNotObject));
}

#[test]
fn a_missing_payload_stores_null() {
    let mut fixture = fixture();
    fixture.harness = Harness::without_payload();
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    failure_handler_helper_fn(
        &mut call,
        &report("provider exploded"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    assert_eq!(
        fixture
            .state
            .model_call_details
            .get("standard_logging_object"),
        Some(&Json::Null)
    );
    assert!(fixture.record.payload.is_none());
}

#[test]
fn the_built_payload_is_stored_and_records_the_timing() {
    let mut fixture = fixture();
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    failure_handler_helper_fn(
        &mut call,
        &report("provider exploded"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    let stored = fixture
        .state
        .model_call_details
        .get("standard_logging_object")
        .expect("the payload is stored");
    assert_eq!(
        stored,
        &Json::Object(payload_to_map(&payload()).expect("the map is built"))
    );
    let inputs = fixture.harness.payloads.inputs();
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].starts_with("provider exploded|"));
    assert!(inputs[0].ends_with("|2026-01-01T00:00:02+00:00"));
}

#[test]
fn the_same_exception_is_prepared_once() {
    let mut fixture = fixture();
    let first = report("provider exploded");
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    let first_timing = failure_handler_helper_fn(&mut call, &first, &fixture.harness.deps())
        .expect("the details are prepared");
    let mut second = first.clone();
    second.end_time = Some(moment("2026-01-01T00:00:09+00:00"));
    let second_timing = failure_handler_helper_fn(&mut call, &second, &fixture.harness.deps())
        .expect("the details are prepared");
    assert_eq!(second_timing.end_time, first_timing.end_time);
    assert_eq!(fixture.harness.payloads.calls(), 1);
}

#[test]
fn a_new_exception_is_prepared_again() {
    let mut fixture = fixture();
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    failure_handler_helper_fn(
        &mut call,
        &report("provider exploded"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    failure_handler_helper_fn(
        &mut call,
        &report("another failure"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    assert_eq!(fixture.harness.payloads.calls(), 2);
}

#[test]
fn the_error_text_is_redacted_for_the_payload_builder() {
    let mut fixture = fixture();
    let mut call = super::support::call(
        &mut fixture.state,
        &mut fixture.record,
        &mut fixture.context,
    );
    failure_handler_helper_fn(
        &mut call,
        &report("provider key sk-abcdefghijklmnop rejected"),
        &fixture.harness.deps(),
    )
    .expect("the details are prepared");
    let inputs = fixture.harness.payloads.inputs();
    assert!(!inputs[0].contains("sk-abcdefghijklmnop"));
}

fn metadata_of(state: &LoggingState) -> IndexMap<String, Json> {
    let params = match state.model_call_details.get("litellm_params") {
        Some(Json::Object(fields)) => fields.clone(),
        other => panic!("litellm_params should be an object, got {other:?}"),
    };
    match params.get("metadata") {
        Some(Json::Object(fields)) => IndexMap::clone(fields),
        other => panic!("metadata should be an object, got {other:?}"),
    }
}

impl Fixture {
    fn record_marker(&self) -> bool {
        self.record.exception.is_some()
    }
}

#[rstest]
#[case::details_carry_the_failure_facts(details_carry_the_failure_facts)]
#[case::tracebacks_are_redacted(tracebacks_are_redacted)]
#[case::reported_start_and_end_times_win(reported_start_and_end_times_win)]
#[case::partial_usage_is_preserved(partial_usage_is_preserved)]
#[case::a_null_usage_object_still_zeroes_the_cost(a_null_usage_object_still_zeroes_the_cost)]
#[case::an_original_response_is_kept(an_original_response_is_kept)]
#[case::exception_headers_are_merged_into_the_metadata(
    exception_headers_are_merged_into_the_metadata
)]
#[case::missing_metadata_leaves_the_details_unchanged(
    missing_metadata_leaves_the_details_unchanged
)]
#[case::a_non_object_litellm_params_is_a_typed_error(a_non_object_litellm_params_is_a_typed_error)]
#[case::a_non_object_metadata_is_a_typed_error(a_non_object_metadata_is_a_typed_error)]
#[case::a_missing_payload_stores_null(a_missing_payload_stores_null)]
#[case::the_built_payload_is_stored_and_records_the_timing(
    the_built_payload_is_stored_and_records_the_timing
)]
#[case::the_same_exception_is_prepared_once(the_same_exception_is_prepared_once)]
#[case::a_new_exception_is_prepared_again(a_new_exception_is_prepared_again)]
#[case::the_error_text_is_redacted_for_the_payload_builder(
    the_error_text_is_redacted_for_the_payload_builder
)]
fn logging_failure_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
