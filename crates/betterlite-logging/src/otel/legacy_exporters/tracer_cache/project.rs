use std::collections::BTreeMap;

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};

use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::selection::span_processor_plan;
use crate::otel::legacy_policy::config::Exporter;

use super::{CachedProvider, DynamicTracerCache, cached_provider};

// Arize Phoenix's _MAX_PROJECT_PROVIDERS
pub const MAX_PROJECT_PROVIDERS: usize = 64;

#[must_use]
pub(crate) fn project_resource(base: &Resource, attributes: &BTreeMap<String, String>) -> Resource {
    let inherited = base
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value.clone()));
    // the project attributes are merged last so OTEL_RESOURCE_ATTRIBUTES cannot pin every provider to one project
    let routed = attributes
        .iter()
        .map(|(name, value)| KeyValue::new(name.clone(), value.clone()));
    Resource::builder_empty()
        .with_attributes(inherited)
        .with_attributes(routed)
        .build()
}

pub(crate) fn tracer_for(
    cache: &DynamicTracerCache,
    project: &str,
    attributes: &BTreeMap<String, String>,
) -> Result<SdkTracer, LegacyExporterError> {
    let owns_exporter = !matches!(cache.config.exporter, Exporter::InMemory);
    // LiteLLM keeps 64 project providers, evicting the least recently used one
    cached_provider(
        cache,
        &cache.projects,
        project.to_owned(),
        MAX_PROJECT_PROVIDERS,
        || {
            Ok(CachedProvider {
                provider: project_provider(cache, attributes)?,
                owns_exporter,
            })
        },
    )
}

fn project_provider(
    cache: &DynamicTracerCache,
    attributes: &BTreeMap<String, String>,
) -> Result<SdkTracerProvider, LegacyExporterError> {
    let plan = span_processor_plan(&cache.config, None, None, cache.callback_name.as_deref())?;
    let processor = cache.builder.span_processor(&plan)?;
    Ok(SdkTracerProvider::builder()
        .with_resource(project_resource(&cache.resource, attributes))
        .with_span_processor(processor)
        .build())
}
