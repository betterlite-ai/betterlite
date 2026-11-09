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

#[test]
fn the_meta_record_matches_the_datadog_schema() {
    let meta = Meta {
        kind: Some(SpanKind::Llm),
        input: Some(InputMeta {
            messages: vec![DdMessage::Input(InputMessage {
                input: "hi".to_owned(),
            })],
        }),
        output: Some(OutputMeta {
            messages: vec![DdMessage::Message(Message {
                role: "assistant".to_owned(),
                content: Json::Text("done".to_owned()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
            })],
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
    };
    assert_eq!(
        serialized(&meta),
        concat!(
            r#"{"kind":"llm","input":{"messages":[{"input":"hi"}]},"#,
            r#""output":{"messages":[{"role":"assistant","content":"done"}]},"metadata":{},"#,
            r#""error":{"message":"boom","stack":null,"type":"ValueError"},"#,
            r#""tool_definitions":[{"name":"t","description":"d"}]}"#
        )
    );
}

#[test]
fn a_span_payload_keeps_its_identity_and_timing() {
    let payload = LLMObsPayload {
        parent_id: Some("undefined".to_owned()),
        trace_id: Some("trace".to_owned()),
        apm_id: None,
        span_id: Some("span".to_owned()),
        name: Some("chat".to_owned()),
        meta: None,
        start_ns: Some(1_700_000_000_000_000_000),
        duration: Some(1_500_000),
        metrics: None,
        tags: Some(vec!["user:me".to_owned()]),
        status: Some(SpanStatus::Ok),
    };
    assert_eq!(
        serialized(&payload),
        concat!(
            r#"{"parent_id":"undefined","trace_id":"trace","span_id":"span","name":"chat","#,
            r#""start_ns":1700000000000000000,"duration":1500000,"#,
            r#""tags":["user:me"],"status":"ok"}"#
        )
    );
}

#[test]
fn span_metrics_serialize_numbers_and_omit_absent_counts() {
    let metrics = LLMMetrics {
        input_tokens: Some(10.0_f64),
        output_tokens: Some(5.0_f64),
        total_tokens: Some(15.0_f64),
        time_to_first_token: Some(0.2_f64),
        time_per_output_token: None,
        total_cost: Some(Decimal::from_str("0.0012").expect("a decimal")),
        cache_read_input_tokens: Some(4.0_f64),
        cache_write_input_tokens: None,
        non_cached_input_tokens: Some(6.0_f64),
        reasoning_output_tokens: None,
        tool_output_tokens: None,
    };
    assert_eq!(
        serialized(&metrics),
        r#"{"input_tokens":10.0,"output_tokens":5.0,"total_tokens":15.0,"time_to_first_token":0.2,"total_cost":0.0012,"cache_read_input_tokens":4.0,"non_cached_input_tokens":6.0}"#
    );
}

#[test]
fn the_intake_envelope_matches_the_datadog_schema() {
    let envelope = DDIntakePayload {
        kind: "llm_obs".to_owned(),
        attributes: DDSpanAttributes {
            ml_app: "betterlite".to_owned(),
            tags: vec!["env:test".to_owned()],
            spans: Vec::new(),
        },
    };
    assert_eq!(
        serialized(&envelope),
        r#"{"type":"llm_obs","attributes":{"ml_app":"betterlite","tags":["env:test"],"spans":[]}}"#
    );
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
    assert_eq!(serialized(&meta), r#"{"metadata":{},"error":null}"#);
}
