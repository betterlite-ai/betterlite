#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{CallbackKwargs, base_api_span};
use crate::deepeval::deepeval_logging::test_support::CountingUuids;
use crate::deepeval::deepeval_logging::types::{
    BaseApiSpan, SpanApiType, SpanTimes, SpanValue, TraceSpanApiStatus,
};

const START: &str = "2024-05-01T10:00:00.120Z";
const END: &str = "2024-05-01T10:00:01.500Z";

fn kwargs(text: &str) -> CallbackKwargs {
    serde_json::from_str(text).unwrap()
}

fn times() -> SpanTimes<'static> {
    SpanTimes {
        start: START,
        end: END,
    }
}

fn span(kwargs: &CallbackKwargs, status: TraceSpanApiStatus) -> BaseApiSpan {
    base_api_span(kwargs, times(), status, &CountingUuids::default())
}

fn text(value: &str) -> SpanValue {
    SpanValue::Text(value.to_owned())
}

#[test]
fn success_span_takes_the_output_and_usage_from_the_logging_object() {
    let kwargs = kwargs(
        r#"{
            "standard_logging_object": {
                "id": "span-id",
                "trace_id": "trace-id",
                "model": "gpt-4o",
                "response": {
                    "usage": {"prompt_tokens": 3, "completion_tokens": 5},
                    "choices": [{"message": {"content": "hello"}}]
                }
            }
        }"#,
    );
    let span = span(&kwargs, TraceSpanApiStatus::Success);
    assert_eq!(span.uuid, "span-id");
    assert_eq!(span.trace_uuid, "trace-id");
    assert_eq!(span.start_time, START);
    assert_eq!(span.end_time, END);
    assert_eq!(span.input, Some(text("NO_INPUT")));
    assert_eq!(span.output, Some(text("hello")));
    assert_eq!(span.model, Some("gpt-4o".to_owned()));
    assert_eq!(span.input_token_count, Some(3));
    assert_eq!(span.output_token_count, Some(5));
}

#[test]
fn success_span_names_the_success_callback_with_the_llm_type() {
    let span = span(&kwargs("{}"), TraceSpanApiStatus::Success);
    assert_eq!(span.name, "litellm_success_callback");
    assert_eq!(span.status, TraceSpanApiStatus::Success);
    assert_eq!(span.span_type, SpanApiType::Llm);
}

#[test]
fn failure_span_names_the_failure_callback_with_the_llm_type() {
    let span = span(&kwargs("{}"), TraceSpanApiStatus::Errored);
    assert_eq!(span.name, "litellm_failure_callback");
    assert_eq!(span.status, TraceSpanApiStatus::Errored);
    assert_eq!(span.span_type, SpanApiType::Llm);
}

#[test]
fn failure_span_takes_the_error_string_and_omits_the_token_counts() {
    let kwargs = kwargs(
        r#"{
            "standard_logging_object": {
                "error_string": "upstream returned 500",
                "response": {"usage": {"prompt_tokens": 3, "completion_tokens": 5}}
            }
        }"#,
    );
    let span = span(&kwargs, TraceSpanApiStatus::Errored);
    assert_eq!(span.output, Some(text("upstream returned 500")));
    assert_eq!(span.input_token_count, None);
    assert_eq!(span.output_token_count, None);
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"standard_logging_object": null}"#)]
#[case::null_error_string(r#"{"standard_logging_object": {"error_string": null}}"#)]
fn failure_span_outputs_an_empty_string_without_an_error_string(#[case] raw: &str) {
    let span = span(&kwargs(raw), TraceSpanApiStatus::Errored);
    assert_eq!(span.output, Some(text("")));
    assert_eq!(span.model, None);
}

#[rstest]
#[case::absent("{}")]
#[case::null_standard_logging_object(r#"{"standard_logging_object": null}"#)]
#[case::null_response(r#"{"standard_logging_object": {"response": null}}"#)]
#[case::null_usage(r#"{"standard_logging_object": {"response": {"usage": null}}}"#)]
#[case::missing_content(
    r#"{"standard_logging_object": {"response": {"choices": [{"message": {}}]}}}"#
)]
#[case::null_message(
    r#"{"standard_logging_object": {"response": {"choices": [{"message": null}]}}}"#
)]
#[case::empty_choices(r#"{"standard_logging_object": {"response": {"choices": []}}}"#)]
#[case::null_choices(r#"{"standard_logging_object": {"response": {"choices": null}}}"#)]
#[case::text_response(r#"{"standard_logging_object": {"response": "upstream text"}}"#)]
#[case::list_response(r#"{"standard_logging_object": {"response": ["upstream text"]}}"#)]
fn success_span_falls_back_to_no_output(#[case] raw: &str) {
    let span = span(&kwargs(raw), TraceSpanApiStatus::Success);
    assert_eq!(span.output, Some(text("NO_OUTPUT")));
}

#[test]
fn a_null_content_leaves_the_output_out() {
    let kwargs = kwargs(
        r#"{"standard_logging_object": {"response": {"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{"id": "call-1"}]}}]}}}"#,
    );
    let span = span(&kwargs, TraceSpanApiStatus::Success);
    assert_eq!(span.output, None);
}

#[test]
fn success_span_keeps_a_content_that_is_a_list_of_parts() {
    let kwargs = kwargs(
        r#"{"standard_logging_object": {"response": {"choices": [{"message": {"content": [{"type": "text", "text": "hi"}]}}]}}}"#,
    );
    let span = span(&kwargs, TraceSpanApiStatus::Success);
    assert_eq!(
        span.output,
        Some(SpanValue::List(vec![
            serde_json::from_str(r#"{"type": "text", "text": "hi"}"#).unwrap()
        ]))
    );
}

#[test]
fn an_absent_input_falls_back_to_no_input() {
    let span = span(&kwargs("{}"), TraceSpanApiStatus::Success);
    assert_eq!(span.input, Some(text("NO_INPUT")));
}

#[test]
fn a_null_input_is_left_out() {
    let span = span(&kwargs(r#"{"input": null}"#), TraceSpanApiStatus::Success);
    assert_eq!(span.input, None);
}

#[test]
fn the_callback_input_is_copied_into_the_span() {
    let kwargs = kwargs(r#"{"input": [{"role": "user", "content": "hi"}]}"#);
    let span = span(&kwargs, TraceSpanApiStatus::Success);
    let expected: Json = serde_json::from_str(r#"{"role": "user", "content": "hi"}"#).unwrap();
    assert_eq!(span.input, Some(SpanValue::List(vec![expected])));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"standard_logging_object": {"id": null, "trace_id": null}}"#)]
fn missing_identifiers_use_generated_uuids(#[case] raw: &str) {
    let kwargs = kwargs(raw);
    let uuids = CountingUuids::default();
    let span = base_api_span(&kwargs, times(), TraceSpanApiStatus::Success, &uuids);
    assert_eq!(span.uuid, "generated-0");
    assert_eq!(span.trace_uuid, "generated-1");
}
