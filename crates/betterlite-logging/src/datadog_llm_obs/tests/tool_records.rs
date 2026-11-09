use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{json, serialized};
use crate::datadog_llm_obs::arguments::ToolArguments;
use crate::datadog_llm_obs::records::{DdMessage, InputMessage, Message, ToolDefinition};
use crate::datadog_llm_obs::span_records::{
    DDIntakePayload, DDLLMObsError, DDSpanAttributes, InputMeta, LLMMetrics, LLMObsPayload, Meta,
    OutputMeta, SpanKind, SpanStatus,
};

#[test]
fn message_records_omit_absent_optional_fields() {
    let message = Message {
        role: "user".to_owned(),
        content: json(r#"[{"type": "text", "text": "hi"}]"#),
        reasoning_content: None,
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    };
    assert_eq!(
        serialized(&DdMessage::Message(message)),
        r#"{"role":"user","content":[{"type":"text","text":"hi"}]}"#
    );
}

#[test]
fn the_input_fallback_record_serializes_as_an_input_field() {
    assert_eq!(
        serialized(&DdMessage::Input(InputMessage {
            input: "hello".to_owned()
        })),
        r#"{"input":"hello"}"#
    );
}

#[test]
fn a_tool_definition_without_a_schema_omits_it() {
    let definition = ToolDefinition {
        name: "t".to_owned(),
        description: String::new(),
        schema: None,
    };
    assert_eq!(serialized(&definition), r#"{"name":"t","description":""}"#);
}

#[test]
fn an_object_tool_argument_serializes_as_its_object() {
    let Json::Object(fields) = json(r#"{"city": "Paris"}"#) else {
        panic!("the fixture is an object");
    };
    assert_eq!(
        serialized(&ToolArguments::Object(fields)),
        r#"{"city":"Paris"}"#
    );
}

fn sample_meta() -> Meta {
    Meta {
        kind: Some(SpanKind::Llm),
        input: Some(InputMeta {
            messages: vec![DdMessage::Input(InputMessage {
                input: "hi".to_owned(),
            })],
        }),
        output: Some(OutputMeta {
            messages: vec![json(r#"{"role": "assistant"}"#)],
        }),
        metadata: Some(indexmap::IndexMap::new()),
        error: Some(DDLLMObsError {
            message: Some("boom".to_owned()),
            stack: None,
            kind: Some("ValueError".to_owned()),
        }),
        tool_definitions: vec![ToolDefinition {
            name: "t".to_owned(),
            description: "d".to_owned(),
            schema: None,
        }],
    }
}

fn sample_metrics() -> LLMMetrics {
    LLMMetrics {
        input_tokens: Some(10.0_f64),
        output_tokens: Some(5.0_f64),
        total_tokens: Some(15.0_f64),
        time_to_first_token: Some(0.2_f64),
        time_per_output_token: None,
        total_cost: Some(Decimal::from_str("0.0012").expect("a decimal")),
        cache_read_input_tokens: None,
        cache_write_input_tokens: None,
        non_cached_input_tokens: None,
        reasoning_output_tokens: None,
        tool_output_tokens: None,
    }
}

fn sample_span() -> LLMObsPayload {
    LLMObsPayload {
        parent_id: Some("undefined".to_owned()),
        trace_id: Some("trace".to_owned()),
        apm_id: None,
        span_id: Some("span".to_owned()),
        name: Some("chat".to_owned()),
        meta: Some(sample_meta()),
        start_ns: Some(1_700_000_000_000_000_000),
        duration: Some(1_500_000),
        metrics: Some(sample_metrics()),
        tags: Some(vec!["user:me".to_owned()]),
        status: Some(SpanStatus::Ok),
    }
}

#[test]
fn the_intake_envelope_matches_the_datadog_schema() {
    let envelope = DDIntakePayload {
        kind: "llm_obs".to_owned(),
        attributes: DDSpanAttributes {
            ml_app: "betterlite".to_owned(),
            tags: vec!["env:test".to_owned()],
            spans: vec![sample_span()],
        },
    };
    assert_eq!(
        serialized(&envelope),
        concat!(
            r#"{"type":"llm_obs","attributes":{"ml_app":"betterlite","tags":["env:test"],"spans":[{"#,
            r#""parent_id":"undefined","trace_id":"trace","span_id":"span","name":"chat","#,
            r#""meta":{"kind":"llm","input":{"messages":[{"input":"hi"}]},"output":{"messages":[{"role":"assistant"}]},"metadata":{},"error":{"message":"boom","type":"ValueError"},"tool_definitions":[{"name":"t","description":"d"}]},"#,
            r#""start_ns":1700000000000000000,"duration":1500000,"#,
            r#""metrics":{"input_tokens":10.0,"output_tokens":5.0,"total_tokens":15.0,"time_to_first_token":0.2,"total_cost":0.0012},"#,
            r#""tags":["user:me"],"status":"ok"}]}}"#
        )
    );
}

#[test]
fn an_empty_span_payload_serializes_as_an_empty_object() {
    let payload = LLMObsPayload {
        parent_id: None,
        trace_id: None,
        apm_id: None,
        span_id: None,
        name: None,
        meta: None,
        start_ns: None,
        duration: None,
        metrics: None,
        tags: None,
        status: None,
    };
    assert_eq!(serialized(&payload), "{}");
}

#[rstest]
#[case::llm(SpanKind::Llm, "\"llm\"")]
#[case::tool(SpanKind::Tool, "\"tool\"")]
#[case::task(SpanKind::Task, "\"task\"")]
#[case::embedding(SpanKind::Embedding, "\"embedding\"")]
#[case::retrieval(SpanKind::Retrieval, "\"retrieval\"")]
fn span_kinds_serialize_lowercase(#[case] kind: SpanKind, #[case] expected: &str) {
    assert_eq!(serialized(&kind), expected);
}

#[rstest]
#[case::ok(SpanStatus::Ok, "\"ok\"")]
#[case::error(SpanStatus::Error, "\"error\"")]
fn span_statuses_serialize_lowercase(#[case] status: SpanStatus, #[case] expected: &str) {
    assert_eq!(serialized(&status), expected);
}

#[test]
fn an_empty_metadata_map_is_kept_when_it_is_present() {
    let meta = Meta {
        kind: None,
        input: None,
        output: None,
        metadata: Some(indexmap::IndexMap::new()),
        error: None,
        tool_definitions: Vec::new(),
    };
    assert_eq!(serialized(&meta), r#"{"metadata":{}}"#);
}
