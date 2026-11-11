#![cfg(test)]

use super::grounding::log_grounding_spans;
use super::support::child;
use super::support::{
    attribute_json, generation, grounding_enrichments, prompt_value, provider, sdk_tracer, span,
    spans,
};

#[test]
fn grounding_dict_top_level_value_becomes_one_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let enrichments = grounding_enrichments(prompt_value(r#"{"uri": "https://example.com"}"#));
    log_grounding_spans(&observations, &enrichments);
    let finished = spans(&exporter);
    assert_eq!(finished.len(), 1);
    let data = finished.first().expect("a span");
    assert_eq!(data.name, "vertex_ai_grounding_metadata");
    assert_eq!(data.parent_span_id, span("0102030405060708"));
    assert_eq!(
        attribute_json(data, "langfuse.observation.input"),
        Some(prompt_value(r#"{"uri": "https://example.com"}"#))
    );
}

#[test]
fn grounding_dict_empty_object_still_becomes_one_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    log_grounding_spans(&observations, &grounding_enrichments(prompt_value("{}")));
    let finished = spans(&exporter);
    assert_eq!(finished.len(), 1);
    assert_eq!(
        attribute_json(
            finished.first().expect("a span"),
            "langfuse.observation.input"
        ),
        Some(prompt_value("{}"))
    );
}
