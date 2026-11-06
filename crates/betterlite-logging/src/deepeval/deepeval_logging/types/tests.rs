#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde::Serialize;

use super::{BaseApiSpan, Environment, SpanApiType, SpanValue, TraceApi, TraceSpanApiStatus};

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn encoded<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn span() -> BaseApiSpan {
    BaseApiSpan {
        uuid: "span-id".to_owned(),
        name: "litellm_success_callback".to_owned(),
        status: TraceSpanApiStatus::Success,
        span_type: SpanApiType::Llm,
        trace_uuid: "trace-id".to_owned(),
        start_time: "2024-05-01T10:00:00.120Z".to_owned(),
        end_time: "2024-05-01T10:00:01.500Z".to_owned(),
        input: Some(SpanValue::Text("hi".to_owned())),
        output: Some(SpanValue::List(vec![json(r#"{"text": "hello"}"#)])),
        model: None,
        input_token_count: None,
        output_token_count: None,
    }
}

#[test]
fn base_span_leaves_a_null_input_and_output_out() {
    let empty = BaseApiSpan {
        uuid: "span-id".to_owned(),
        name: "litellm_success_callback".to_owned(),
        status: TraceSpanApiStatus::Success,
        span_type: SpanApiType::Llm,
        trace_uuid: "trace-id".to_owned(),
        start_time: "2024-05-01T10:00:00.120Z".to_owned(),
        end_time: "2024-05-01T10:00:01.500Z".to_owned(),
        input: None,
        output: None,
        model: None,
        input_token_count: None,
        output_token_count: None,
    };
    let Json::Object(fields) = encoded(&empty) else {
        panic!("the span is not an object");
    };
    assert!(!fields.contains_key("input"));
    assert!(!fields.contains_key("output"));
}

#[test]
fn base_span_writes_camel_case_aliases_and_skips_absent_optional_fields() {
    assert_eq!(
        encoded(&span()),
        json(
            r#"{
                "uuid": "span-id",
                "name": "litellm_success_callback",
                "status": "SUCCESS",
                "type": "llm",
                "traceUuid": "trace-id",
                "startTime": "2024-05-01T10:00:00.120Z",
                "endTime": "2024-05-01T10:00:01.500Z",
                "input": "hi",
                "output": [{"text": "hello"}]
            }"#
        )
    );
}

#[test]
fn base_span_writes_the_model_and_token_counts_under_their_aliases() {
    let full = BaseApiSpan {
        uuid: "span-id".to_owned(),
        name: "litellm_success_callback".to_owned(),
        status: TraceSpanApiStatus::Success,
        span_type: SpanApiType::Llm,
        trace_uuid: "trace-id".to_owned(),
        start_time: "2024-05-01T10:00:00.120Z".to_owned(),
        end_time: "2024-05-01T10:00:01.500Z".to_owned(),
        input: Some(SpanValue::Text("hi".to_owned())),
        output: Some(SpanValue::Text("hello".to_owned())),
        model: Some("gpt-4o".to_owned()),
        input_token_count: Some(3),
        output_token_count: Some(5),
    };
    let Json::Object(fields) = encoded(&full) else {
        panic!("the span is not an object");
    };
    assert_eq!(fields["model"], json(r#""gpt-4o""#));
    assert_eq!(fields["inputTokenCount"], json("3"));
    assert_eq!(fields["outputTokenCount"], json("5"));
}

#[test]
fn trace_writes_every_span_list_and_the_environment() {
    let trace = TraceApi {
        uuid: "trace-id".to_owned(),
        base_spans: Vec::new(),
        agent_spans: Vec::new(),
        llm_spans: vec![span()],
        retriever_spans: Vec::new(),
        tool_spans: Vec::new(),
        start_time: "2024-05-01T10:00:00.120Z".to_owned(),
        end_time: "2024-05-01T10:00:01.500Z".to_owned(),
        environment: Environment::Staging,
    };
    let Json::Object(fields) = encoded(&trace) else {
        panic!("the trace is not an object");
    };
    assert_eq!(fields["uuid"], json(r#""trace-id""#));
    assert_eq!(fields["baseSpans"], json("[]"));
    assert_eq!(fields["agentSpans"], json("[]"));
    assert_eq!(fields["llmSpans"], Json::List(vec![encoded(&span())]));
    assert_eq!(fields["retrieverSpans"], json("[]"));
    assert_eq!(fields["toolSpans"], json("[]"));
    assert_eq!(fields["startTime"], json(r#""2024-05-01T10:00:00.120Z""#));
    assert_eq!(fields["endTime"], json(r#""2024-05-01T10:00:01.500Z""#));
    assert_eq!(fields["environment"], json(r#""staging""#));
    assert!(!fields.contains_key("metadata"));
    assert!(!fields.contains_key("tags"));
}

#[rstest]
#[case::text(r#""hi""#)]
#[case::object(r#"{"role": "user"}"#)]
#[case::list(r#"[{"role": "user"}]"#)]
fn span_values_round_trip_the_declared_shapes(#[case] raw: &str) {
    let value: SpanValue = serde_json::from_str(raw).unwrap();
    assert_eq!(encoded(&value), json(raw));
}
