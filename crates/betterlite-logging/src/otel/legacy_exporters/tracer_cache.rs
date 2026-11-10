use std::collections::BTreeMap;
use std::fmt;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, PoisonError};
use std::thread::Builder;

use indexmap::IndexMap;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
use tracing::debug;

use crate::otel::legacy_exporters::build::LegacyExporterBuilder;
use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::headers::HeadersError;
use crate::otel::legacy_exporters::selection::{SpanProcessorPlan, span_processor_plan};
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};

pub const MAX_DYNAMIC_TRACER_PROVIDERS: usize = 256;

const DEFAULT_TRACER_NAME: &str = "litellm";
const TRACER_NAME_ENV: &str = "OTEL_TRACER_NAME";
pub const SHUTDOWN_THREAD_NAME: &str = "otel-dynamic-provider-shutdown";
// LiteLLM shuts dropped providers down on a four-worker pool; one worker behind this queue has the same cap.
const SHUTDOWN_QUEUE: usize = 4;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum CacheKey {
    Config {
        exporter: String,
        endpoint: Option<String>,
        headers: Option<String>,
    },
    Headers(BTreeMap<String, String>),
}

// The header values are collector credentials, so Debug prints only the header names.
impl fmt::Debug for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config { exporter, .. } => write!(f, "Config({exporter}, redacted)"),
            Self::Headers(headers) => {
                write!(f, "Headers({:?})", headers.keys().collect::<Vec<_>>())
            }
        }
    }
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

enum ShutdownJob {
    Drop(CachedProvider),
    Flush(SyncSender<usize>),
}

pub struct DynamicTracerCache {
    config: OpenTelemetryConfig,
    callback_name: Option<String>,
    builder: LegacyExporterBuilder,
    resource: Resource,
    tracer_name: String,
    max_providers: usize,
    entries: Mutex<IndexMap<CacheKey, CachedProvider>>,
    shutdowns: SyncSender<ShutdownJob>,
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

fn shut_down(dropped: &CachedProvider, shut_down_count: &mut usize) {
    if let Err(error) = dropped.provider.shutdown() {
        debug!(error = %error, "otel: a dropped dynamic tracer provider failed to shut down");
    }
    *shut_down_count = shut_down_count.saturating_add(1);
}

fn start_shutdown_worker(receiver: Receiver<ShutdownJob>) {
    let worker = Builder::new()
        .name(SHUTDOWN_THREAD_NAME.to_owned())
        .spawn(move || {
            let mut shut_down_count = 0_usize;
            while let Ok(job) = receiver.recv() {
                match job {
                    ShutdownJob::Drop(dropped) => shut_down(&dropped, &mut shut_down_count),
                    ShutdownJob::Flush(done) => _ = done.send(shut_down_count),
                }
            }
        });
    if let Err(error) = worker {
        debug!(error = %error, "otel: no worker started to shut dropped tracer providers down");
    }
}

impl DynamicTracerCache {
    #[must_use]
    pub fn new(settings: CacheSettings, builder: LegacyExporterBuilder) -> Self {
        let (shutdowns, receiver) = sync_channel(SHUTDOWN_QUEUE);
        start_shutdown_worker(receiver);
        Self {
            config: settings.config,
            callback_name: settings.callback_name,
            resource: settings.resource,
            tracer_name: settings.tracer_name,
            max_providers: settings.max_providers.max(1),
            builder,
            entries: Mutex::new(IndexMap::new()),
            shutdowns,
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
        self.tracer_for_plan(key, &dynamic_config.exporter, Some(dynamic_config), None)
    }

    pub fn tracer_for_headers(
        &self,
        dynamic_headers: &BTreeMap<String, String>,
    ) -> Result<SdkTracer, LegacyExporterError> {
        let key = CacheKey::Headers(dynamic_headers.clone());
        let exporter = &self.config.exporter;
        self.tracer_for_plan(key, exporter, None, Some(dynamic_headers))
    }

    fn tracer_for_plan(
        &self,
        key: CacheKey,
        exporter: &Exporter,
        config_override: Option<&OpenTelemetryConfig>,
        dynamic_headers: Option<&BTreeMap<String, String>>,
    ) -> Result<SdkTracer, LegacyExporterError> {
        let owns_exporter = !matches!(exporter, Exporter::InMemory);
        let callback_name = self.callback_name.as_deref();
        self.tracer_for_key(key, owns_exporter, || {
            span_processor_plan(
                &self.config,
                dynamic_headers,
                config_override,
                callback_name,
            )
        })
    }

    fn tracer_for_key<F>(
        &self,
        key: CacheKey,
        owns_exporter: bool,
        plan: F,
    ) -> Result<SdkTracer, LegacyExporterError>
    where
        F: FnOnce() -> Result<SpanProcessorPlan, HeadersError>,
    {
        self.cached_dynamic_tracer(key, owns_exporter, move || {
            let processor = self.builder.span_processor(&plan()?)?;
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
        if self.shutdowns.send(ShutdownJob::Drop(dropped)).is_err() {
            debug!("otel: no shutdown worker took a dropped dynamic tracer provider");
        }
    }

    #[must_use]
    pub fn cached_keys(&self) -> Vec<CacheKey> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let keys: Vec<CacheKey> = entries.keys().cloned().collect();
        drop(entries);
        keys
    }

    #[must_use]
    pub fn join_shutdowns(&self) -> usize {
        let (done, ack) = sync_channel(1);
        if self.shutdowns.send(ShutdownJob::Flush(done)).is_err() {
            return 0;
        }
        ack.recv().unwrap_or(0)
    }
}

#[path = "tracer_cache/tests.rs"]
mod tests;
