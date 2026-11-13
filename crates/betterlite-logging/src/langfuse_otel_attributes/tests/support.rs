#![cfg(test)]

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::LLMObsAttributes as _;

use super::llm_obs::LangfuseLLMObsOTELAttributes;
use super::metadata::set_metadata_attributes;
use super::observation::set_observation_output;
use super::writer::set_langfuse_otel_attributes;

pub(super) fn value(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn model(text: &str) -> OtelModelValue {
    serde_json::from_str(text).unwrap()
}

pub(super) fn kwargs(text: &str) -> MetadataMap {
    serde_json::from_str(text).unwrap()
}

pub(super) fn attr(name: &str, value: &str) -> (String, Value) {
    (name.to_owned(), Value::from(value.to_owned()))
}

pub(super) fn flag(name: &str, value: bool) -> (String, Value) {
    (name.to_owned(), Value::Bool(value))
}

pub(super) fn attributes<F>(record: F) -> Vec<(String, Value)>
where
    F: FnOnce(&mut SdkSpan),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("langfuse-otel-attributes").start("test");
    record(&mut span);
    span.end();
    let spans = exporter.get_finished_spans().unwrap();
    let finished = spans.first().unwrap();
    finished
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

pub(super) fn messages_attributes(text: &str) -> Vec<(String, Value)> {
    let kwargs = kwargs(text);
    attributes(|span| LangfuseLLMObsOTELAttributes::set_messages(span, &kwargs))
}

pub(super) fn response_attributes(text: &str) -> Vec<(String, Value)> {
    let response = model(text);
    attributes(|span| LangfuseLLMObsOTELAttributes::set_response_output_messages(span, &response))
}

pub(super) fn metadata_attributes(text: &str) -> Vec<(String, Value)> {
    let Json::Object(metadata) = value(text) else {
        panic!("metadata is an object");
    };
    attributes(|span| set_metadata_attributes(span, &metadata))
}

pub(super) fn observation_attributes(text: &str) -> Vec<(String, Value)> {
    let response = value(text);
    attributes(|span| set_observation_output(span, Some(&response)))
}

pub(super) fn entry_attributes(
    kwargs_text: &str,
    response_text: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> Vec<(String, Value)> {
    let kwargs = kwargs(kwargs_text);
    let response = response_text.map(model);
    attributes(|span| {
        set_langfuse_otel_attributes(span, &kwargs, response.as_ref(), environment, false);
    })
}

pub(super) struct NoEnvironment;

impl EnvironmentLookup for NoEnvironment {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }
}
