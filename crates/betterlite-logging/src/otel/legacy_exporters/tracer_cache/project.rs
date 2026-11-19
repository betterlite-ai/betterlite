use std::collections::BTreeMap;
use std::sync::PoisonError;

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};

use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::selection::span_processor_plan;
use crate::otel::legacy_policy::config::Exporter;

use super::{CachedProvider, DynamicTracerCache};

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
    let mut projects = cache
        .projects
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(cached) = projects.shift_remove(project) {
        let tracer = cached.provider.tracer(cache.tracer_name.clone());
        projects.insert(project.to_owned(), cached);
        return Ok(tracer);
    }
    drop(projects);
    // resource detection and exporter construction can block, so they run without the cache lock
    let plan = span_processor_plan(&cache.config, None, None, cache.callback_name.as_deref())?;
    let processor = cache.builder.span_processor(&plan)?;
    let fresh = CachedProvider {
        provider: SdkTracerProvider::builder()
            .with_resource(project_resource(&cache.resource, attributes))
            .with_span_processor(processor)
            .build(),
        owns_exporter: !matches!(cache.config.exporter, Exporter::InMemory),
    };
    let mut cached_projects = cache
        .projects
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(raced) = cached_projects.shift_remove(project) {
        let tracer = raced.provider.tracer(cache.tracer_name.clone());
        cached_projects.insert(project.to_owned(), raced);
        drop(cached_projects);
        cache.dispose(fresh);
        return Ok(tracer);
    }
    // LiteLLM evicts before inserting once the cache is full, so 64 providers stay
    let evicted = if cached_projects.len() >= MAX_PROJECT_PROVIDERS {
        cached_projects
            .shift_remove_index(0)
            .map(|(_name, cached)| cached)
    } else {
        None
    };
    let tracer = fresh.provider.tracer(cache.tracer_name.clone());
    cached_projects.insert(project.to_owned(), fresh);
    drop(cached_projects);
    if let Some(dropped) = evicted {
        cache.dispose(dropped);
    }
    Ok(tracer)
}

#[must_use]
pub(crate) fn project_names(cache: &DynamicTracerCache) -> Vec<String> {
    let projects = cache
        .projects
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let names: Vec<String> = projects.keys().cloned().collect();
    drop(projects);
    names
}
