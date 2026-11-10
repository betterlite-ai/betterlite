#![cfg(test)]

use opentelemetry::Value;

use super::grounding::log_grounding_spans;
use super::support::child;
use super::support::{
    attribute, attribute_json, generation, grounding_enrichments, prompt_value, provider,
    sdk_tracer, spans,
};

#[test]
fn grounding_list_dict_entries_become_one_span_per_key() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let enrichments = grounding_enrichments(prompt_value(r#"[{"a": 1, "b": "two"}, "stray"]"#));
    log_grounding_spans(&observations, &enrichments);
    let finished = spans(&exporter);
    assert_eq!(finished.len(), 3);
    assert_eq!(finished.first().expect("a span").name, "a");
    assert_eq!(finished.get(1).expect("a span").name, "b");
    assert_eq!(
        finished.get(2).expect("a span").name,
        "vertex_ai_grounding_metadata"
    );
    assert_eq!(
        attribute(
            finished.first().expect("a span"),
            "langfuse.observation.input"
        ),
        Some(Value::from("1"))
    );
    assert_eq!(
        attribute(
            finished.get(1).expect("a span"),
            "langfuse.observation.input"
        ),
        Some(Value::from("two"))
    );
    assert_eq!(
        attribute(
            finished.get(2).expect("a span"),
            "langfuse.observation.input"
        ),
        Some(Value::from("stray"))
    );
}

#[test]
fn grounding_list_non_dict_entries_each_get_one_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let enrichments = grounding_enrichments(prompt_value(r#"[1, "two"]"#));
    log_grounding_spans(&observations, &enrichments);
    let finished = spans(&exporter);
    assert_eq!(finished.len(), 2);
    assert!(
        finished
            .iter()
            .all(|data| data.name == "vertex_ai_grounding_metadata")
    );
    assert_eq!(
        attribute(
            finished.first().expect("a span"),
            "langfuse.observation.input"
        ),
        Some(Value::from("1"))
    );
    assert_eq!(
        attribute(
            finished.get(1).expect("a span"),
            "langfuse.observation.input"
        ),
        Some(Value::from("two"))
    );
}

#[test]
fn grounding_list_empty_list_logs_no_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    log_grounding_spans(&observations, &grounding_enrichments(prompt_value("[]")));
    assert!(spans(&exporter).is_empty());
}

#[test]
fn grounding_list_dict_entries_keep_nested_values_as_json() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let enrichments = grounding_enrichments(prompt_value(
        r#"[{"sources": {"uri": "https://example.com"}}]"#,
    ));
    log_grounding_spans(&observations, &enrichments);
    let finished = spans(&exporter);
    let data = finished.first().expect("a span");
    assert_eq!(
        attribute_json(data, "langfuse.observation.input"),
        Some(prompt_value(r#"{"uri": "https://example.com"}"#))
    );
}
