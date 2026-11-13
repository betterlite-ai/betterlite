#![cfg(test)]

use opentelemetry::Value;
use rstest::rstest;

use super::grounding::log_grounding_spans;
use super::support::child;
use super::support::{
    attribute, generation, grounding_enrichments, prompt_value, provider, sdk_tracer, spans,
};

#[rstest]
#[case::a_string(r#""grounded""#, "grounded")]
#[case::a_number("5", "5")]
#[case::a_bool("true", "true")]
fn grounding_scalar_top_level_value_becomes_one_span(#[case] grounding: &str, #[case] input: &str) {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let enrichments = grounding_enrichments(prompt_value(grounding));
    log_grounding_spans(&observations, &enrichments);
    let exported = spans(&exporter);
    assert_eq!(exported.len(), 1);
    let data = exported.first().expect("a span");
    assert_eq!(data.name, "vertex_ai_grounding_metadata");
    assert_eq!(
        attribute(data, "langfuse.observation.input"),
        Some(Value::from(input.to_owned()))
    );
}
