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

impl DynamicTracerCache {
    pub fn tracer_for_project(
        &self,
        project: &str,
        attributes: &BTreeMap<String, String>,
    ) -> Result<SdkTracer, LegacyExporterError> {
        let mut projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cached) = projects.shift_remove(project) {
            let tracer = cached.provider.tracer(self.tracer_name.clone());
            projects.insert(project.to_owned(), cached);
            return Ok(tracer);
        }
        drop(projects);
        // resource detection and exporter construction can block, so they run without the cache lock
        let fresh = CachedProvider {
            provider: self.project_provider(attributes)?,
            owns_exporter: !matches!(self.config.exporter, Exporter::InMemory),
        };
        let mut projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(raced) = projects.shift_remove(project) {
            let tracer = raced.provider.tracer(self.tracer_name.clone());
            projects.insert(project.to_owned(), raced);
            drop(projects);
            self.dispose(fresh);
            return Ok(tracer);
        }
        // LiteLLM evicts before inserting once the cache is full, so 64 providers stay
        let evicted = if projects.len() >= MAX_PROJECT_PROVIDERS {
            projects.shift_remove_index(0).map(|(_name, cached)| cached)
        } else {
            None
        };
        let tracer = fresh.provider.tracer(self.tracer_name.clone());
        projects.insert(project.to_owned(), fresh);
        drop(projects);
        if let Some(dropped) = evicted {
            self.dispose(dropped);
        }
        Ok(tracer)
    }

    fn project_provider(
        &self,
        attributes: &BTreeMap<String, String>,
    ) -> Result<SdkTracerProvider, LegacyExporterError> {
        let plan = span_processor_plan(&self.config, None, None, self.callback_name.as_deref())?;
        let processor = self.builder.span_processor(&plan)?;
        Ok(SdkTracerProvider::builder()
            .with_resource(project_resource(&self.resource, attributes))
            .with_span_processor(processor)
            .build())
    }

    #[must_use]
    pub fn project_names(&self) -> Vec<String> {
        let projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        let names: Vec<String> = projects.keys().cloned().collect();
        drop(projects);
        names
    }
}
