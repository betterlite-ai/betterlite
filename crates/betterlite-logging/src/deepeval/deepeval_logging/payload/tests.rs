#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{CallbackKwargs, base_api_span, trace_api};
use crate::deepeval::deepeval_logging::test_support::CountingUuids;
use crate::deepeval::deepeval_logging::types::{
    BaseApiSpan, Environment, SpanApiType, SpanTimes, SpanValue, TraceSpanApiStatus,
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
    assert_eq!(span.input, SpanValue::Text("NO_INPUT".to_owned()));
    assert_eq!(span.output, SpanValue::Text("hello".to_owned()));
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
    assert_eq!(
        span.output,
        SpanValue::Text("upstream returned 500".to_owned())
    );
    assert_eq!(span.input_token_count, None);
    assert_eq!(span.output_token_count, None);
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"standard_logging_object": null}"#)]
fn failure_span_outputs_an_empty_string_without_an_error_string(#[case] raw: &str) {
    let span = span(&kwargs(raw), TraceSpanApiStatus::Errored);
    assert_eq!(span.output, SpanValue::Text(String::new()));
    assert_eq!(span.model, None);
}

#[rstest]
#[case::absent("{}")]
#[case::missing_content(
    r#"{"standard_logging_object": {"response": {"choices": [{"message": {}}]}}}"#
)]
#[case::empty_choices(r#"{"standard_logging_object": {"response": {"choices": []}}}"#)]
#[case::text_response(r#"{"standard_logging_object": {"response": "upstream text"}}"#)]
fn success_span_falls_back_to_no_output(#[case] raw: &str) {
    let span = span(&kwargs(raw), TraceSpanApiStatus::Success);
    assert_eq!(span.output, SpanValue::Text("NO_OUTPUT".to_owned()));
}

#[test]
fn success_span_keeps_a_content_that_is_a_list_of_parts() {
    let kwargs = kwargs(
        r#"{"standard_logging_object": {"response": {"choices": [{"message": {"content": [{"type": "text", "text": "hi"}]}}]}}}"#,
    );
    let span = span(&kwargs, TraceSpanApiStatus::Success);
    assert_eq!(
        span.output,
        SpanValue::List(vec![
            serde_json::from_str(r#"{"type": "text", "text": "hi"}"#).unwrap()
        ])
    );
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"input": null}"#)]
fn an_absent_or_null_input_falls_back_to_no_input(#[case] raw: &str) {
    let span = span(&kwargs(raw), TraceSpanApiStatus::Success);
    assert_eq!(span.input, SpanValue::Text("NO_INPUT".to_owned()));
}

#[test]
fn the_callback_input_is_copied_into_the_span() {
    let kwargs = kwargs(r#"{"input": [{"role": "user", "content": "hi"}]}"#);
    let span = span(&kwargs, TraceSpanApiStatus::Success);
    let expected: Json = serde_json::from_str(r#"{"role": "user", "content": "hi"}"#).unwrap();
    assert_eq!(span.input, SpanValue::List(vec![expected]));
}

#[test]
fn missing_identifiers_use_generated_uuids() {
    let kwargs = kwargs("{}");
    let uuids = CountingUuids::default();
    let span = base_api_span(&kwargs, times(), TraceSpanApiStatus::Success, &uuids);
    let trace_uuid = span.trace_uuid.clone();
    assert_eq!(span.uuid, "generated-0");
    assert_eq!(trace_uuid, "generated-1");
    let trace = trace_api(span, times(), Environment::Development);
    assert_eq!(trace.uuid, trace_uuid);
}

#[test]
fn trace_wraps_the_span_with_the_configured_environment() {
    let span = span(&kwargs("{}"), TraceSpanApiStatus::Success);
    let trace = trace_api(span, times(), Environment::Production);
    assert_eq!(trace.uuid, trace.llm_spans[0].trace_uuid);
    assert_eq!(trace.llm_spans.len(), 1);
    assert!(trace.base_spans.is_empty());
    assert!(trace.agent_spans.is_empty());
    assert!(trace.retriever_spans.is_empty());
    assert!(trace.tool_spans.is_empty());
    assert_eq!(trace.start_time, START);
    assert_eq!(trace.end_time, END);
    assert_eq!(trace.environment, Environment::Production);
}
