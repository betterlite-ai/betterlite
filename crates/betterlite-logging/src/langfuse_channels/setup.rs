use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{Sampler, ShouldSample};

use crate::langfuse_observations::sampler::TraceIdHashSampler;
use crate::langfuse_otel_attributes::keys::LangfuseSpanAttributes;

pub(super) fn resource(environment: Option<&str>, release: Option<&str>) -> Resource {
    let attributes: Vec<KeyValue> = [
        (LangfuseSpanAttributes::LangfuseEnvironment, environment),
        (LangfuseSpanAttributes::Release, release),
    ]
    .into_iter()
    .filter_map(|(key, present)| present.map(|value| KeyValue::new(key.value(), value.to_owned())))
    .collect();
    Resource::builder_empty()
        .with_attributes(attributes)
        .build()
}

pub(super) fn sampler(rate: f64, disabled: bool) -> Box<dyn ShouldSample> {
    if disabled {
        Box::new(Sampler::AlwaysOff)
    } else if rate >= 1.0 {
        Box::new(Sampler::AlwaysOn)
    } else {
        Box::new(TraceIdHashSampler { rate })
    }
}
