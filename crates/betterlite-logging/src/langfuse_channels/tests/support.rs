#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::langfuse_settings::Settings;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer as _,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::cache::{CacheState, TRACING};
use super::{ChannelSpec, LangfuseChannelRequest, LangfuseTracing, acquire_langfuse_tracing};

pub(super) const PUBLIC_KEY: &str = "pk-test";
pub(super) const BASE_URL: &str = "https://langfuse.test";
pub(super) const SECRET: &str = "sk-test";

pub(super) fn lookup(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn settings(variables: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: lookup(variables),
    }
    .load("")
    .expect("the langfuse settings load")
    .sections
    .langfuse_settings
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(super) fn request(public_key: &str) -> LangfuseChannelRequest {
    LangfuseChannelRequest {
        public_key: public_key.to_owned(),
        secret_key: secret(SECRET),
        base_url: BASE_URL.to_owned(),
        environment: None,
        release: None,
        flush_interval: 1,
        mock_mode: true,
    }
}

pub(super) fn spec() -> ChannelSpec<'static> {
    ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 1.0,
        flush_interval_millis: 60_000,
        flush_at: 512,
    }
}

pub(super) fn schedule(flush_interval_millis: u64, flush_at: usize) -> ChannelSpec<'static> {
    ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 1.0,
        flush_interval_millis,
        flush_at,
    }
}

pub(super) fn acquire_counting(
    request: &LangfuseChannelRequest,
    builds: &Arc<AtomicUsize>,
) -> LangfuseTracing {
    let counter = Arc::clone(builds);
    acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        request,
        move || {
            counter.fetch_add(1, Ordering::AcqRel);
            Ok(Recording::default())
        },
    )
    .expect("the channel is acquired")
}

pub(super) fn acquire_with<E>(request: &LangfuseChannelRequest, exporter: E) -> LangfuseTracing
where
    E: SpanExporter + 'static,
{
    acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        request,
        || Ok(exporter),
    )
    .expect("the live channel is acquired")
}

pub(super) fn lease_of<'state>(
    state: &'state CacheState,
    tracing: &LangfuseTracing,
) -> Option<&'state super::cache::Lease> {
    state
        .leases
        .values()
        .find(|lease| Arc::ptr_eq(&lease.tracing.ledger.exporter, &tracing.ledger.exporter))
}

pub(super) fn cached(tracing: &LangfuseTracing) -> Option<u32> {
    let state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    lease_of(&state, tracing).map(|lease| lease.holders)
}

pub(super) fn retire_id(tracing: &LangfuseTracing) -> Option<u64> {
    let state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    let lease = lease_of(&state, tracing)?;
    let id = lease.retire.as_ref().map(|retire| retire.id);
    drop(state);
    id
}

#[derive(Clone, Debug, Default)]
pub(super) struct Recording {
    pub(super) spans: Arc<Mutex<Vec<SpanData>>>,
    pub(super) resource: Arc<Mutex<Option<Resource>>>,
    pub(super) shut_down: Arc<AtomicUsize>,
}

impl Recording {
    pub(super) fn spans(&self) -> Vec<SpanData> {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn resource(&self) -> Option<Resource> {
        self.resource
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn span_count(&self) -> usize {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn shut_down(&self) -> bool {
        self.shut_down.load(Ordering::Acquire) > 0
    }
}

impl SpanExporter for Recording {
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(batch);
        ready(Ok(()))
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.shut_down.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn set_resource(&mut self, resource: &Resource) {
        *self.resource.lock().unwrap_or_else(PoisonError::into_inner) = Some(resource.clone());
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Failing {
    pub(super) failures: Arc<AtomicUsize>,
}

impl Failing {
    pub(super) fn failure_count(&self) -> usize {
        self.failures.load(Ordering::Acquire)
    }
}

impl SpanExporter for Failing {
    fn export(&self, _batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        self.failures.fetch_add(1, Ordering::AcqRel);
        ready(Err(OTelSdkError::InternalFailure(
            "the destination refused the batch".to_owned(),
        )))
    }
}

#[derive(Clone, Debug)]
pub(super) struct Gated {
    pub(super) gate: Arc<Semaphore>,
}

impl SpanExporter for Gated {
    async fn export(&self, _batch: Vec<SpanData>) -> OTelSdkResult {
        let _permit = self.gate.acquire().await.expect("the gate stays open");
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) struct GatedRecording {
    pub(super) gate: Arc<Semaphore>,
    pub(super) recording: Recording,
}

impl SpanExporter for GatedRecording {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let _permit = self.gate.acquire().await.expect("the gate stays open");
        self.recording.export(batch).await
    }
}

pub(super) fn end_spans(tracing: &LangfuseTracing, count: usize) {
    for _ in 0..count {
        end_span(tracing, "burst");
    }
}

pub(super) fn end_span(tracing: &LangfuseTracing, name: &'static str) {
    let mut span = tracing.tracer.span_builder(name).start(&tracing.tracer);
    span.end();
}

pub(super) async fn wait_until<F>(check: F)
where
    F: Fn() -> bool,
{
    for _ in 0..600_u32 {
        if check() {
            return;
        }
        sleep(Duration::from_millis(5)).await;
    }
    assert!(check(), "the condition did not hold in time");
}

pub(super) fn linked_context(index: usize) -> SpanContext {
    let mut trace = [0_u8; 16];
    trace[15] = u8::try_from(index).expect("a link index fits a byte");
    SpanContext::new(
        TraceId::from_bytes(trace),
        SpanId::from_bytes([2; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::NONE,
    )
}

#[derive(Clone, Default)]
pub(super) struct Logs(pub(super) Arc<Mutex<Vec<(Level, String)>>>);

#[derive(Default)]
pub(super) struct Message(pub(super) String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, message.0));
    }
}

impl Logs {
    pub(super) fn messages(&self, level: Level) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(recorded, _)| *recorded == level)
            .map(|(_, message)| message.clone())
            .collect()
    }
}

pub(super) fn capture<T, F>(level: Option<LevelFilter>, run: F) -> (T, Logs)
where
    F: FnOnce() -> T,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = match level {
        Some(filter) => with_default(subscriber.with(filter), run),
        None => with_default(subscriber, run),
    };
    (value, logs)
}
