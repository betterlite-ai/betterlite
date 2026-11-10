#![cfg(test)]

use opentelemetry::Value;
use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use crate::otel::mappers::registry::{MapperError, resolve_mappers};
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn resolution(values: &[&str]) -> Vec<(&'static str, Option<usize>)> {
    resolve_mappers(&names(values))
        .expect("mappers resolve")
        .iter()
        .map(|mapper| (mapper.name(), mapper.tool_attr_budget()))
        .collect()
}

#[test]
fn resolve_mappers_returns_instances_in_order() {
    assert_eq!(
        resolution(&["genai", "openinference", "weave", "langtrace", "langfuse"]),
        vec![
            ("genai", Some(16)),
            ("openinference", Some(16)),
            ("weave", None),
            ("langtrace", None),
            ("langfuse", None),
        ]
    );
}

#[test]
fn resolve_mappers_counts_each_vocabulary_once() {
    assert_eq!(
        resolution(&["genai", "openinference", "genai", "weave"]),
        vec![
            ("genai", Some(16)),
            ("openinference", Some(16)),
            ("genai", Some(16)),
            ("weave", None),
        ]
    );
}

#[test]
fn resolve_mappers_shares_the_budget_with_legacy() {
    assert_eq!(
        resolution(&["genai", "legacy", "openinference"]),
        vec![
            ("genai", Some(10)),
            ("legacy", Some(10)),
            ("openinference", Some(10)),
        ]
    );
}

#[test]
fn resolve_mappers_keeps_plain_mappers_out_of_the_budget() {
    assert_eq!(
        resolution(&["langfuse", "weave", "langtrace"]),
        vec![("langfuse", None), ("weave", None), ("langtrace", None)]
    );
}

#[test]
fn resolve_mappers_rejects_an_unknown_name() {
    let Err(error) = resolve_mappers(&names(&["genai", "nope"])) else {
        panic!("an unknown mapper name resolved");
    };
    let MapperError::Unknown { name, known } = error;
    assert_eq!(name, "nope");
    assert_eq!(
        known,
        "genai, langfuse, langtrace, legacy, openinference, weave"
    );
}

fn call() -> LLMCallSpanData {
    let payload: Payload =
        serde_json::from_str(r#"{"id": "call-1", "model": "gpt-4o", "call_type": "completion"}"#)
            .expect("payload");
    LLMCallSpanData::from_standard_logging_payload(&payload, LLMCallOptions::default())
        .expect("llm call")
}

fn attributes_of(first: &str) -> AttributeMap {
    let span_call = call();
    let mappers = resolve_mappers(&names(&[first])).expect("mappers resolve");
    let mapper = mappers.first().expect("one mapper");
    mapper.map(&SpanData::LLMCall(&span_call), &BTreeMap::new())
}

#[test]
fn resolved_mappers_map_an_llm_call_span() {
    assert_eq!(
        attributes_of("genai").get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert_eq!(
        attributes_of("langfuse").get("langfuse.observation.type"),
        Some(&Value::from("generation"))
    );
    assert_eq!(
        attributes_of("openinference").get("openinference.span.kind"),
        Some(&Value::from("LLM"))
    );
    assert_eq!(
        attributes_of("weave").get("weave.display_name"),
        Some(&Value::from("chat gpt-4o"))
    );
    assert_eq!(
        attributes_of("langtrace").get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
}
