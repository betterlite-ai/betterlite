#![cfg(test)]

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, InstrumentationScope, Key, KeyValue, Value};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{Span, SpanData, SpanEvents, SpanLinks, SpanProcessor};

use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::fan_out::destination::DestinationProcessorFactory;
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use crate::otel::plumbing::exporters::{ExporterFactories, HttpClients};

pub(crate) const TRACE_ID: u128 = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10;
pub(crate) const SPAN_ID: u64 = 0x0102_0304_0506_0708;
pub(crate) const PARENT_ID: u64 = 0x1112_1314_1516_1718;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn attributes(pairs: &[(&str, Value)]) -> Vec<KeyValue> {
    pairs
        .iter()
        .map(|(key, value)| KeyValue::new(Key::new((*key).to_owned()), value.clone()))
        .collect()
}

pub(crate) fn string_attribute(key: &str, value: &str) -> KeyValue {
    KeyValue::new(key.to_owned(), value.to_owned())
}

pub(crate) fn span(attributes: Vec<KeyValue>, parent_span_id: SpanId) -> SpanData {
    SpanData {
        span_context: SpanContext::new(
            TraceId::from(TRACE_ID),
            SpanId::from(SPAN_ID),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Borrowed("call model"),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: SystemTime::UNIX_EPOCH,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("fan-out-test").build(),
    }
}

pub(crate) fn root_span(attributes: Vec<KeyValue>) -> SpanData {
    span(attributes, SpanId::INVALID)
}

pub(crate) fn keys(span: &SpanData) -> BTreeSet<String> {
    span.attributes
        .iter()
        .map(|pair| pair.key.to_string())
        .collect()
}

fn destination_of(
    endpoint: &str,
    headers: &[(&str, &str)],
    protocol: Option<&str>,
    callback_name: Option<&str>,
    scope: SpanScope,
) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: endpoint.to_owned(),
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        resource_attributes: BTreeMap::new(),
        callback_name: callback_name.map(str::to_owned),
        protocol: protocol.map(str::to_owned),
        span_scope: scope,
    })
}

pub(crate) fn destination(endpoint: &str, scope: SpanScope) -> OtelDestination {
    destination_of(endpoint, &[], None, None, scope)
}

pub(crate) fn destination_with(
    endpoint: &str,
    headers: &[(&str, &str)],
    callback_name: Option<&str>,
) -> OtelDestination {
    destination_of(endpoint, headers, None, callback_name, SpanScope::Full)
}

pub(crate) fn destination_with_protocol(
    endpoint: &str,
    protocol: &str,
    scope: SpanScope,
) -> OtelDestination {
    destination_of(endpoint, &[], Some(protocol), None, scope)
}

#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub(crate) spans: Mutex<Vec<SpanData>>,
    pub(crate) starts: Mutex<usize>,
    pub(crate) flushes: Mutex<usize>,
    pub(crate) shutdowns: Mutex<usize>,
    pub(crate) resources: Mutex<Vec<Resource>>,
    pub(crate) flush_fails: bool,
}

impl Recorder {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn delivered(&self) -> Vec<SpanData> {
        lock(&self.spans).clone()
    }

    pub(crate) fn shutdown_count(&self) -> usize {
        *lock(&self.shutdowns)
    }

    pub(crate) fn flush_count(&self) -> usize {
        *lock(&self.flushes)
    }

    pub(crate) fn start_count(&self) -> usize {
        *lock(&self.starts)
    }

    pub(crate) fn resources(&self) -> Vec<Resource> {
        lock(&self.resources).clone()
    }
}

impl SpanProcessor for Recorder {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {
        let mut starts = lock(&self.starts);
        *starts = starts.saturating_add(1);
    }

    fn on_end(&self, span: SpanData) {
        lock(&self.spans).push(span);
    }

    fn set_resource(&mut self, resource: &Resource) {
        lock(&self.resources).push(resource.clone());
    }

    fn force_flush(&self) -> OTelSdkResult {
        let mut flushes = lock(&self.flushes);
        *flushes = flushes.saturating_add(1);
        drop(flushes);
        if self.flush_fails {
            return Err(OTelSdkError::InternalFailure("flush failed".to_owned()));
        }
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        let mut shutdowns = lock(&self.shutdowns);
        *shutdowns = shutdowns.saturating_add(1);
        drop(shutdowns);
        Ok(())
    }
}

// Lets a wrapped processor hand the same recorder to the test that inspects it.
#[derive(Debug, Clone)]
pub(crate) struct SharedRecorder(pub(crate) Arc<Recorder>);

impl SpanProcessor for SharedRecorder {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.0.on_start(span, cx);
    }

    fn on_end(&self, span: SpanData) {
        self.0.on_end(span);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        lock(&self.0.resources).push(resource.clone());
    }
}

#[derive(Debug, Default)]
pub(crate) struct RecordingFactory {
    pub(crate) built: Mutex<Vec<(String, Resource)>>,
    pub(crate) unbuildable: BTreeSet<String>,
    pub(crate) recorders: Mutex<Vec<Arc<Recorder>>>,
    pub(crate) flush_fails: bool,
}

impl RecordingFactory {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn build_count(&self, endpoint: &str) -> usize {
        lock(&self.built)
            .iter()
            .filter(|(built, _resource)| built == endpoint)
            .count()
    }

    pub(crate) fn endpoints(&self) -> Vec<String> {
        lock(&self.built)
            .iter()
            .map(|(endpoint, _resource)| endpoint.clone())
            .collect()
    }

    pub(crate) fn recorder_for(&self, endpoint: &str) -> Option<Arc<Recorder>> {
        let endpoints: Vec<String> = lock(&self.built)
            .iter()
            .map(|(built, _resource)| built.clone())
            .collect();
        let index = endpoints.iter().rposition(|built| built == endpoint)?;
        lock(&self.recorders).get(index).cloned()
    }
}

impl DestinationProcessorFactory for RecordingFactory {
    fn build(
        &self,
        destination: &OtelDestination,
        resource: &Resource,
    ) -> Option<Arc<dyn SpanProcessor>> {
        let endpoint = destination.as_ref().endpoint.clone();
        if self.unbuildable.contains(&endpoint) {
            return None;
        }
        let recorder = Arc::new(Recorder {
            spans: Mutex::new(Vec::new()),
            starts: Mutex::new(0),
            flushes: Mutex::new(0),
            shutdowns: Mutex::new(0),
            resources: Mutex::new(Vec::new()),
            flush_fails: self.flush_fails,
        });
        lock(&self.built).push((endpoint, resource.clone()));
        lock(&self.recorders).push(Arc::clone(&recorder));
        Some(recorder)
    }
}

#[derive(Debug, Default)]
pub(crate) struct FixedScope {
    pub(crate) current: Mutex<RequestOtelScope>,
}

impl FixedScope {
    pub(crate) fn new(destinations: Vec<OtelDestination>, additive: bool) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(RequestOtelScope {
                destinations,
                additive,
            }),
        })
    }
}

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        lock(&self.current).clone()
    }
}

#[derive(Debug)]
pub(crate) struct NoopClient;

#[async_trait::async_trait]
impl HttpClient for NoopClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the fan-out tests never export"))
    }
}

pub(crate) fn factories() -> Arc<ExporterFactories> {
    let client: Arc<dyn HttpClient> = Arc::new(NoopClient);
    Arc::new(ExporterFactories::new(
        HttpClients {
            traces: Arc::clone(&client),
            metrics: Arc::clone(&client),
            logs: client,
        },
        BTreeMap::new(),
    ))
}

pub(crate) const RELEASE_SLOTS: usize = 16;
const RELEASE_BOUND: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(crate) struct SignalCloser {
    pub(crate) signal: SyncSender<()>,
    pub(crate) fail: bool,
}

impl SpanProcessor for SignalCloser {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, _span: SpanData) {}

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        let _sent = self.signal.send(());
        if self.fail {
            Err(OTelSdkError::InternalFailure("close failed".to_owned()))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockPoint {
    End,
    Close,
}

pub(crate) struct BlockingProcessor {
    pub(crate) entered: SyncSender<()>,
    pub(crate) release: Mutex<Receiver<()>>,
    pub(crate) shutdowns: Mutex<usize>,
    pub(crate) block: BlockPoint,
}

impl Debug for BlockingProcessor {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str("BlockingProcessor")
    }
}

fn block_until_released(entered: &SyncSender<()>, release: &Mutex<Receiver<()>>) {
    let _signalled = entered.send(());
    let guard = lock(release);
    let _released = guard.recv_timeout(RELEASE_BOUND);
    drop(guard);
}

impl SpanProcessor for BlockingProcessor {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, _span: SpanData) {
        if self.block == BlockPoint::End {
            block_until_released(&self.entered, &self.release);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        let mut shutdowns = lock(&self.shutdowns);
        *shutdowns = shutdowns.saturating_add(1);
        drop(shutdowns);
        if self.block == BlockPoint::Close {
            block_until_released(&self.entered, &self.release);
        }
        Ok(())
    }
}

impl BlockingProcessor {
    pub(crate) fn shutdown_count(&self) -> usize {
        *lock(&self.shutdowns)
    }
}

fn blocking_processor(block: BlockPoint) -> (Arc<BlockingProcessor>, Receiver<()>, SyncSender<()>) {
    let (entered, entered_rx) = mpsc::sync_channel(RELEASE_SLOTS);
    let (release_tx, release_rx) = mpsc::sync_channel(RELEASE_SLOTS);
    let processor = Arc::new(BlockingProcessor {
        entered,
        release: Mutex::new(release_rx),
        shutdowns: Mutex::new(0),
        block,
    });
    (processor, entered_rx, release_tx)
}

pub(crate) fn blocking_closer() -> (Arc<BlockingProcessor>, Receiver<()>, SyncSender<()>) {
    blocking_processor(BlockPoint::Close)
}

pub(crate) fn blocking_recorder() -> (Arc<BlockingProcessor>, Receiver<()>, SyncSender<()>) {
    blocking_processor(BlockPoint::End)
}

#[derive(Debug)]
pub(crate) struct OneBlockingFactory {
    pub(crate) blocking_endpoint: String,
    pub(crate) blocking: Arc<BlockingProcessor>,
    pub(crate) others: Arc<RecordingFactory>,
}

impl DestinationProcessorFactory for OneBlockingFactory {
    fn build(
        &self,
        destination: &OtelDestination,
        resource: &Resource,
    ) -> Option<Arc<dyn SpanProcessor>> {
        if destination.as_ref().endpoint == self.blocking_endpoint {
            let concrete = Arc::clone(&self.blocking);
            let blocking: Arc<dyn SpanProcessor> = concrete;
            return Some(blocking);
        }
        self.others.build(destination, resource)
    }
}

pub(crate) fn spec(kind: &str, endpoint: Option<&str>, headers: Option<&str>) -> ExporterSpec {
    ExporterSpec {
        kind: kind.to_owned(),
        endpoint: endpoint.map(str::to_owned),
        traces_endpoint: None,
        headers: headers.map(str::to_owned),
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers: false,
    }
}

pub(crate) fn langfuse_spec(endpoint: &str) -> ExporterSpec {
    ExporterSpec {
        kind: "otlp_http".to_owned(),
        endpoint: Some(endpoint.to_owned()),
        traces_endpoint: None,
        headers: None,
        owner: Some(ExporterOwner::LangfuseOtel),
        options: None,
        use_simple_processor: None,
        requires_headers: false,
    }
}

pub(crate) fn settings(exporters: Vec<ExporterSpec>, langfuse_span_scope: SpanScope) -> Settings {
    Settings {
        exporters,
        service_name: "betterlite".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope,
        excluded_services: BTreeSet::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}
