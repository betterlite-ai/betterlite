use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;
use std::time::Duration;

use betterlite_logging::otel::legacy_exporters::build::LegacyExporterBuilder;
use betterlite_logging::otel::legacy_exporters::selection::{
    ExporterKind, ExporterPlan, ProcessorKind, SpanProcessorPlan,
};
use betterlite_logging::otel::legacy_exporters::tracer_cache::{
    CacheKey, CacheSettings, DynamicTracerCache, MAX_DYNAMIC_TRACER_PROVIDERS, SHUTDOWN_THREAD_NAME,
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

use super::{Input, RequestInput, cache_key};
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

pub(super) fn plan_json(plan: &ExporterPlan) -> Value {
    json!({
        "kind": kind_text(plan.kind),
        "endpoint": plan.endpoint,
        "headers": plan.headers,
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
        Err(HttpError::from("no connection is opened by this helper"))
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
    sender: SyncSender<usize>,
}

impl SpanProcessor for ShutdownRecorder {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, _span: SpanData) {}

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    // The cache shuts a provider down only on its own worker; the SDK's drop-time shutdown runs elsewhere.
    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        if thread::current().name() == Some(SHUTDOWN_THREAD_NAME) {
            _ = self.sender.try_send(self.index);
        }
        Ok(())
    }
}

fn echo(requests: &[RequestInput], keys: &[CacheKey]) -> Vec<Value> {
    keys.iter()
        .map(|key| {
            requests
                .iter()
                .find(|request| cache_key(&request.key).as_ref() == Some(key))
                .map_or(Value::Null, |request| request.key.clone())
        })
        .collect()
}

fn fill_cache(
    cache: &DynamicTracerCache,
    input: &Input,
    keys: &[CacheKey],
    sender: &SyncSender<usize>,
) -> Result<Vec<bool>, Box<HelperError>> {
    let mut built = Vec::with_capacity(keys.len());
    for (index, (request, key)) in input.requests.iter().zip(keys).enumerate() {
        let ran = Cell::new(false);
        cache
            .cached_dynamic_tracer(key.clone(), request.owns_exporter, || {
                ran.set(true);
                let processor = ShutdownRecorder {
                    index,
                    sender: sender.clone(),
                };
                Ok(SdkTracerProvider::builder()
                    .with_span_processor(processor)
                    .build())
            })
            .map_err(|error| invalid(&error.to_string()))?;
        built.push(ran.get());
    }
    Ok(built)
}

pub(super) fn tracer_cache_sequence(input: &Input) -> HelperResult {
    let keys = input
        .requests
        .iter()
        .map(|request| cache_key(&request.key))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| invalid("a request key must be a config or a headers key"))?;
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
    let built = fill_cache(&cache, input, &keys, &sender)?;
    let _joined = cache.join_shutdowns();
    let mut shut_down: Vec<usize> = receiver.try_iter().collect();
    shut_down.sort_unstable();
    let order = echo(&input.requests, &cache.cached_keys());
    let shut_down_json: Vec<Value> = shut_down
        .iter()
        .filter_map(|index| input.requests.get(*index))
        .map(|request| request.key.clone())
        .collect();
    Ok(json!({ "built": built, "order": order, "shut_down": shut_down_json }))
}
