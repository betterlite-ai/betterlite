use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;
use std::time::Duration;

use betterlite_logging::otel::legacy_exporters::build::LegacyExporterBuilder;
use betterlite_logging::otel::legacy_exporters::selection::{
    ExporterKind, ProcessorKind, SpanProcessorPlan,
};
use betterlite_logging::otel::legacy_exporters::tracer_cache::{
    CacheKey, CacheSettings, DynamicTracerCache, MAX_DYNAMIC_TRACER_PROVIDERS,
};
use betterlite_logging::otel::plumbing::exporters::HttpClients;
use opentelemetry::Context;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracerProvider, Span, SpanData, SpanProcessor,
};
use serde_json::{Value, json};
use tracing::debug;

use super::Input;
use super::inputs::{RequestInput, cache_key};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

const fn kind_text(kind: ExporterKind) -> &'static str {
    match kind {
        ExporterKind::Shared => "shared",
        ExporterKind::Console => "console",
        ExporterKind::OtlpHttp => "otlp_http",
        ExporterKind::OtlpGrpc => "otlp_grpc",
    }
}

const fn processor_text(kind: ProcessorKind) -> &'static str {
    match kind {
        ProcessorKind::Simple => "simple",
        ProcessorKind::Batch => "batch",
    }
}

pub(super) fn plan_fields(
    kind: ExporterKind,
    endpoint: Option<&str>,
    headers: &BTreeMap<String, String>,
) -> Value {
    json!({
        "kind": kind_text(kind),
        "endpoint": endpoint,
        "headers": headers,
    })
}

pub(super) fn span_plan_json(plan: &SpanProcessorPlan) -> Value {
    json!({
        "kind": kind_text(plan.kind),
        "endpoint": plan.endpoint,
        "headers": plan.headers,
        "processor": processor_text(plan.processor),
    })
}

pub(super) fn resource_json(resource: &Resource) -> Value {
    let attributes: BTreeMap<String, String> = resource
        .iter()
        .map(|(key, value)| (key.as_str().to_owned(), value.as_str().into_owned()))
        .collect();
    json!({ "attributes": attributes })
}

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from(
            "the exporter cache helper opens no connection",
        ))
    }
}

fn no_server_clients() -> HttpClients {
    let shared: Arc<dyn HttpClient> = Arc::new(NoServer);
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

#[derive(Debug)]
struct ShutdownRecorder {
    index: usize,
    caller_thread: Option<String>,
    sender: SyncSender<usize>,
}

impl SpanProcessor for ShutdownRecorder {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, _span: SpanData) {}

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    // A provider dropped without owning its exporter still shuts down on the dropping thread.
    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        if thread::current().name().map(str::to_owned) != self.caller_thread
            && self.sender.try_send(self.index).is_err()
        {
            debug!("otel: the tracer cache helper lost a shutdown record");
        }
        Ok(())
    }
}

fn recording_provider(
    index: usize,
    caller_thread: Option<&str>,
    sender: &SyncSender<usize>,
) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(ShutdownRecorder {
            index,
            caller_thread: caller_thread.map(str::to_owned),
            sender: sender.clone(),
        })
        .build()
}

fn echo(requests: &[RequestInput], keys: &[CacheKey]) -> Vec<Value> {
    let parsed: Vec<Option<CacheKey>> = requests
        .iter()
        .map(|request| cache_key(&request.key))
        .collect();
    keys.iter()
        .map(|key| {
            parsed
                .iter()
                .position(|candidate| candidate.as_ref() == Some(key))
                .and_then(|index| requests.get(index))
                .map_or(Value::Null, |request| request.key.clone())
        })
        .collect()
}

fn fill_cache(
    cache: &DynamicTracerCache,
    input: &Input,
    keys: &[CacheKey],
    sender: &SyncSender<usize>,
    caller_thread: Option<&str>,
) -> Result<Vec<bool>, Box<HelperError>> {
    let mut built = Vec::with_capacity(keys.len());
    for (index, (request, key)) in input.requests.iter().zip(keys).enumerate() {
        let ran = Cell::new(false);
        let provider_sender = sender.clone();
        cache
            .cached_dynamic_tracer(key.clone(), request.owns_exporter, || {
                ran.set(true);
                Ok(recording_provider(index, caller_thread, &provider_sender))
            })
            .map_err(|error| invalid(&error.to_string()))?;
        built.push(ran.get());
    }
    Ok(built)
}

pub(super) fn tracer_cache_sequence(input: &Input) -> HelperResult {
    let keys: Vec<CacheKey> = input
        .requests
        .iter()
        .map(|request| cache_key(&request.key))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| invalid("every request key must be a config or a headers key"))?;
    let (sender, receiver) = sync_channel(input.requests.len().max(1));
    let cache = DynamicTracerCache::new(
        CacheSettings {
            config: input.config.to_config(&input.environment),
            callback_name: input.callback_name.clone(),
            resource: Resource::builder_empty().build(),
            tracer_name: "litellm".to_owned(),
            max_providers: input.max_providers.unwrap_or(MAX_DYNAMIC_TRACER_PROVIDERS),
        },
        LegacyExporterBuilder::new(no_server_clients(), InMemorySpanExporter::default()),
    );
    let caller_thread = thread::current().name().map(str::to_owned);
    let built = fill_cache(&cache, input, &keys, &sender, caller_thread.as_deref())?;
    cache.join_shutdowns();
    let mut shut_down = Vec::new();
    while let Ok(index) = receiver.try_recv() {
        shut_down.push(index);
    }
    shut_down.sort_unstable();
    let order = echo(&input.requests, &cache.cached_keys());
    let shut_down_json: Vec<Value> = shut_down
        .iter()
        .filter_map(|index| input.requests.get(*index))
        .map(|request| request.key.clone())
        .collect();
    Ok(json!({
        "built": built,
        "order": order,
        "shut_down": shut_down_json,
    }))
}
