use std::collections::BTreeMap;
use std::mem;
use std::sync::{Mutex, PoisonError};
use std::thread::{Builder, JoinHandle};

use indexmap::IndexMap;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
use tracing::debug;

use crate::otel::legacy_exporters::build::LegacyExporterBuilder;
use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::selection::span_processor_plan;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};

pub const MAX_DYNAMIC_TRACER_PROVIDERS: usize = 256;

const DEFAULT_TRACER_NAME: &str = "litellm";
const TRACER_NAME_ENV: &str = "OTEL_TRACER_NAME";
const SHUTDOWN_THREAD_NAME: &str = "otel-dynamic-provider-shutdown";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CacheKey {
    Config {
        exporter: String,
        endpoint: Option<String>,
        headers: Option<String>,
    },
    Headers(BTreeMap<String, String>),
}

pub struct CacheSettings {
    pub config: OpenTelemetryConfig,
    pub callback_name: Option<String>,
    pub resource: Resource,
    pub tracer_name: String,
    pub max_providers: usize,
}

struct CachedProvider {
    provider: SdkTracerProvider,
    owns_exporter: bool,
}

pub struct DynamicTracerCache {
    config: OpenTelemetryConfig,
    callback_name: Option<String>,
    builder: LegacyExporterBuilder,
    resource: Resource,
    tracer_name: String,
    max_providers: usize,
    entries: Mutex<IndexMap<CacheKey, CachedProvider>>,
    shutdowns: Mutex<Vec<JoinHandle<()>>>,
}

#[must_use]
pub fn legacy_tracer_name(env: &BTreeMap<String, String>) -> String {
    env.get(TRACER_NAME_ENV)
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_TRACER_NAME.to_owned())
}

fn exporter_text(exporter: &Exporter) -> String {
    match exporter {
        Exporter::Console => "console".to_owned(),
        Exporter::InMemory => "in_memory".to_owned(),
        Exporter::Named(text) => text.clone(),
    }
}

fn evicted(
    entries: &mut IndexMap<CacheKey, CachedProvider>,
    max_providers: usize,
) -> Option<CachedProvider> {
    if entries.len() <= max_providers {
        return None;
    }
    entries
        .shift_remove_index(0)
        .map(|(_key, provider)| provider)
}

impl DynamicTracerCache {
    #[must_use]
    pub fn new(settings: CacheSettings, builder: LegacyExporterBuilder) -> Self {
        Self {
            config: settings.config,
            callback_name: settings.callback_name,
            resource: settings.resource,
            tracer_name: settings.tracer_name,
            max_providers: settings.max_providers.max(1),
            builder,
            entries: Mutex::new(IndexMap::new()),
            shutdowns: Mutex::new(Vec::new()),
        }
    }

    pub fn tracer_for_config(
        &self,
        dynamic_config: &OpenTelemetryConfig,
    ) -> Result<SdkTracer, LegacyExporterError> {
        let key = CacheKey::Config {
            exporter: exporter_text(&dynamic_config.exporter),
            endpoint: dynamic_config.endpoint.clone(),
            headers: dynamic_config.headers.clone(),
        };
        let owns_exporter = !matches!(&dynamic_config.exporter, Exporter::InMemory);
        self.cached_dynamic_tracer(key, owns_exporter, || {
            let plan = span_processor_plan(
                &self.config,
                None,
                Some(dynamic_config),
                self.callback_name.as_deref(),
            )?;
            let processor = self.builder.span_processor(&plan)?;
            Ok(SdkTracerProvider::builder()
                .with_resource(self.resource.clone())
                .with_span_processor(processor)
                .build())
        })
    }

    pub fn tracer_for_headers(
        &self,
        dynamic_headers: &BTreeMap<String, String>,
    ) -> Result<SdkTracer, LegacyExporterError> {
        let key = CacheKey::Headers(dynamic_headers.clone());
        let owns_exporter = !matches!(&self.config.exporter, Exporter::InMemory);
        self.cached_dynamic_tracer(key, owns_exporter, || {
            let plan = span_processor_plan(
                &self.config,
                Some(dynamic_headers),
                None,
                self.callback_name.as_deref(),
            )?;
            let processor = self.builder.span_processor(&plan)?;
            Ok(SdkTracerProvider::builder()
                .with_resource(self.resource.clone())
                .with_span_processor(processor)
                .build())
        })
    }

    pub fn cached_dynamic_tracer<F>(
        &self,
        key: CacheKey,
        owns_exporter: bool,
        build: F,
    ) -> Result<SdkTracer, LegacyExporterError>
    where
        F: FnOnce() -> Result<SdkTracerProvider, LegacyExporterError>,
    {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cached) = entries.shift_remove(&key) {
            let tracer = cached.provider.tracer(self.tracer_name.clone());
            entries.insert(key, cached);
            return Ok(tracer);
        }
        drop(entries);
        // Exporter construction can block on DNS or TLS, so it runs without the cache lock.
        let fresh = CachedProvider {
            provider: build()?,
            owns_exporter,
        };
        let mut cache_entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let racing = cache_entries.shift_remove(&key);
        let (tracer, dropped) = if let Some(raced) = racing {
            let tracer = raced.provider.tracer(self.tracer_name.clone());
            cache_entries.insert(key, raced);
            (tracer, Some(fresh))
        } else {
            let tracer = fresh.provider.tracer(self.tracer_name.clone());
            cache_entries.insert(key, fresh);
            (tracer, evicted(&mut cache_entries, self.max_providers))
        };
        drop(cache_entries);
        if let Some(provider) = dropped {
            self.dispose(provider);
        }
        Ok(tracer)
    }

    fn dispose(&self, dropped: CachedProvider) {
        if !dropped.owns_exporter {
            return;
        }
        let spawned = Builder::new()
            .name(SHUTDOWN_THREAD_NAME.to_owned())
            .spawn(move || {
                if let Err(error) = dropped.provider.shutdown() {
                    debug!(error = %error, "otel: a dropped dynamic tracer provider failed to shut down");
                }
            });
        match spawned {
            Ok(handle) => self
                .shutdowns
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(handle),
            Err(error) => {
                debug!(error = %error, "otel: no thread started to shut a dropped dynamic tracer provider down");
            }
        }
    }

    #[must_use]
    pub fn cached_keys(&self) -> Vec<CacheKey> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let mut keys = Vec::with_capacity(entries.len());
        keys.extend(entries.keys().cloned());
        keys
    }

    pub fn join_shutdowns(&self) -> usize {
        let handles = mem::take(
            &mut *self
                .shutdowns
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        let joined = handles.len();
        for handle in handles {
            if handle.join().is_err() {
                debug!("otel: a dynamic tracer provider shutdown thread panicked");
            }
        }
        joined
    }
}

#[path = "tracer_cache/tests.rs"]
mod tests;
