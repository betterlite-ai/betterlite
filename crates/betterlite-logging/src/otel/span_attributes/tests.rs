#![cfg(test)]

use super::*;
use opentelemetry::trace::{Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

fn exported_attributes(exporter: &InMemorySpanExporter) -> Vec<(String, Value)> {
    let spans = exporter.get_finished_spans().expect("spans");
    spans
        .first()
        .expect("one span")
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

#[rstest::rstest]
#[case::null("null", Value::from(""))]
#[case::string(r#""text""#, Value::from("text"))]
#[case::empty_string(r#""""#, Value::from(""))]
#[case::boolean_true("true", Value::Bool(true))]
#[case::boolean_false("false", Value::Bool(false))]
#[case::whole_number("42", Value::I64(42))]
#[case::negative_number("-7", Value::I64(-7))]
#[case::float_number("1.5", Value::F64(1.5))]
#[case::whole_float("1.0", Value::F64(1.0))]
#[case::array("[1,2]", Value::from("[1,2]"))]
#[case::object(r#"{"a":1}"#, Value::from(r#"{"a":1}"#))]
fn primitive_value_matches_litellm(#[case] raw: &str, #[case] expected: Value) {
    let value: OtelModelValue = serde_json::from_str(raw).expect("value");
    assert_eq!(primitive_value(&value), expected);
}

#[test]
fn safe_set_attribute_writes_primitives() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("span-attributes").start("test");
    span.safe_set_attribute("litellm.string", &OtelModelValue::String("text".to_owned()));
    span.safe_set_attribute(
        "litellm.number",
        &OtelModelValue::Number(serde_json::Number::from(3_i64)),
    );
    span.safe_set_attribute("litellm.flag", &OtelModelValue::Bool(true));
    span.safe_set_attribute("litellm.null", &OtelModelValue::Null(()));
    span.end();
    assert_eq!(
        exported_attributes(&exporter),
        vec![
            ("litellm.string".to_owned(), Value::from("text")),
            ("litellm.number".to_owned(), Value::I64(3)),
            ("litellm.flag".to_owned(), Value::Bool(true)),
            ("litellm.null".to_owned(), Value::from("")),
        ]
    );
}

struct NoOverrides;

impl LLMObsAttributes for NoOverrides {}

#[test]
fn llm_obs_defaults_write_nothing() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("span-attributes").start("test");
    NoOverrides::set_messages(&mut span, &MetadataMap::new());
    NoOverrides::set_response_output_messages(&mut span, &OtelModelValue::Null(()));
    span.end();
    assert!(exported_attributes(&exporter).is_empty());
}
