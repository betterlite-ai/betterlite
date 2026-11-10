use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::future::{Future, ready};
use std::io::{Result as IoResult, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::langfuse_settings::Settings;
use http::HeaderMap;
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{InstrumentationScope, KeyValue, Value};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::AnyValue;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueKind;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
use prost::Message as _;
use secrecy::SecretString;
use thiserror::Error;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::langfuse_exporter::encoder::{OtlpSpanEncoder, SpanEncoder};
use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_exporter::exporter::LangfuseSpanExporter;
use crate::langfuse_exporter::options::{ExporterOptions, exporter_options};
use crate::langfuse_exporter::sleep::Sleep;
use crate::langfuse_exporter::transport::{HttpRequest, HttpResponse, Transport};
use crate::langfuse_settings::Credentials;

pub(super) const UNENCODABLE_KEY: &str = "test.unencodable";
pub(super) const CLOUD_ENDPOINT: &str = "https://cloud.langfuse.com/api/public/otel/v1/traces";

#[derive(Clone, Default)]
pub(super) struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    pub(super) fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn captured<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let value = with_default(subscriber, action);
    (value, buffer.contents())
}

#[derive(Debug, Error)]
pub(super) enum ScriptedError {
    #[error("the scripted connection failed")]
    Failed,
}

#[derive(Clone, Debug)]
pub(super) enum Step {
    Status(u16),
    Transport,
}

#[derive(Clone, Debug)]
pub(super) struct Seen {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) timeout: Duration,
}

#[derive(Clone, Debug)]
pub(super) struct ScriptedTransport {
    steps: Arc<Mutex<VecDeque<Step>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl ScriptedTransport {
    pub(super) fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(VecDeque::from(steps))),
            seen: Arc::default(),
        }
    }

    pub(super) fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Transport for ScriptedTransport {
    type Error = ScriptedError;

    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, ScriptedError>> + Send {
        self.seen.lock().unwrap().push(Seen {
            url: request.url.to_string(),
            headers: header_pairs(&request.headers),
            body: request.body.to_vec(),
            timeout: request.timeout,
        });
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Step::Status(200));
        ready(match step {
            Step::Status(status) => Ok(HttpResponse { status }),
            Step::Transport => Err(ScriptedError::Failed),
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct RecordingSleep {
    delays: Mutex<Vec<Duration>>,
}

impl RecordingSleep {
    pub(super) fn delays(&self) -> Vec<Duration> {
        self.delays.lock().unwrap().clone()
    }

    pub(super) fn seconds(&self) -> Vec<u64> {
        self.delays().iter().map(Duration::as_secs).collect()
    }
}

impl Sleep for RecordingSleep {
    fn sleep(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.delays.lock().unwrap().push(delay);
        Box::pin(ready(()))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RejectingEncoder;

impl SpanEncoder for RejectingEncoder {
    fn encode(&self, spans: &[SpanData], resource: &Resource) -> Option<Vec<u8>> {
        if spans.iter().any(|span| {
            span.attributes
                .iter()
                .any(|attribute| attribute.key.as_str() == UNENCODABLE_KEY)
        }) {
            return None;
        }
        OtlpSpanEncoder.encode(spans, resource)
    }
}

pub(super) struct Fixture {
    pub(super) exporter: LangfuseSpanExporter<ScriptedTransport>,
    pub(super) transport: ScriptedTransport,
    pub(super) sleep: Arc<RecordingSleep>,
}

pub(super) fn exporter_with(steps: Vec<Step>, environment: &[(&str, &str)]) -> Fixture {
    build(
        steps,
        environment,
        Arc::new(OtlpSpanEncoder),
        &credentials(),
    )
}

pub(super) fn rejecting_exporter_with(steps: Vec<Step>) -> Fixture {
    build(steps, &[], Arc::new(RejectingEncoder), &credentials())
}

fn build(
    steps: Vec<Step>,
    environment: &[(&str, &str)],
    encoder: Arc<dyn SpanEncoder>,
    credentials: &Credentials,
) -> Fixture {
    let lookup = lookup(environment);
    let settings = settings_from(&lookup);
    let transport = ScriptedTransport::new(steps);
    let sleep = Arc::new(RecordingSleep::default());
    let options = exporter_options(&settings, &lookup, credentials).unwrap();
    let recording = Arc::clone(&sleep);
    let sleep_dependency: Arc<dyn Sleep> = recording;
    let exporter =
        LangfuseSpanExporter::new(&options, transport.clone(), encoder, sleep_dependency).unwrap();
    Fixture {
        exporter,
        transport,
        sleep,
    }
}

pub(super) fn options_for(
    environment: &[(&str, &str)],
) -> Result<ExporterOptions, LangfuseExporterError> {
    let lookup = lookup(environment);
    let settings = settings_from(&lookup);
    exporter_options(&settings, &lookup, &credentials())
}

pub(super) fn lookup(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn settings_from(lookup: &BTreeMap<String, String>) -> Settings {
    ConfigLoader {
        lookup: lookup.clone(),
    }
    .load("")
    .unwrap()
    .sections
    .langfuse_settings
}

pub(super) fn credentials() -> Credentials {
    Credentials {
        public_key: Some("pk-lf-test".to_owned()),
        secret_key: Some(SecretString::from("sk-lf-test")),
        host: "https://cloud.langfuse.com".to_owned(),
    }
}

pub(super) fn span(name: &str, attributes: &[(&str, &str)]) -> SpanData {
    SpanData {
        span_context: SpanContext::new(
            TraceId::from(1_u128),
            SpanId::from(2_u64),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Owned(name.to_owned()),
        start_time: UNIX_EPOCH,
        end_time: UNIX_EPOCH,
        attributes: attributes
            .iter()
            .map(|(key, value)| KeyValue::new((*key).to_owned(), Value::from((*value).to_owned())))
            .collect(),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("langfuse-sdk").build(),
    }
}

pub(super) fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap().to_owned()))
        .collect();
    pairs.sort();
    pairs
}

pub(super) fn header(pairs: &[(String, String)], name: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(present, _)| present == name)
        .map(|(_, value)| value.clone())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DecodedSpan {
    pub(super) name: String,
    pub(super) attributes: Vec<(String, String)>,
}

pub(super) fn spans_in(body: &[u8]) -> Vec<DecodedSpan> {
    let request = ExportTraceServiceRequest::decode(body).unwrap();
    request
        .resource_spans
        .iter()
        .flat_map(|resource| resource.scope_spans.iter())
        .flat_map(|scope| scope.spans.iter())
        .map(|span| DecodedSpan {
            name: span.name.clone(),
            attributes: span
                .attributes
                .iter()
                .map(|attribute| {
                    (
                        attribute.key.clone(),
                        attribute
                            .value
                            .as_ref()
                            .map_or_else(String::new, any_value_text),
                    )
                })
                .collect(),
        })
        .collect()
}

pub(super) fn spans_of(records: &[Seen]) -> Vec<Vec<DecodedSpan>> {
    records.iter().map(|seen| spans_in(&seen.body)).collect()
}

pub(super) fn attribute<'span>(span: &'span DecodedSpan, name: &str) -> Option<&'span str> {
    span.attributes
        .iter()
        .find(|(present, _)| present == name)
        .map(|(_, value)| value.as_str())
}

fn any_value_text(value: &AnyValue) -> String {
    match value.value.as_ref() {
        Some(AnyValueKind::StringValue(text)) => text.clone(),
        Some(AnyValueKind::BoolValue(flag)) => flag.to_string(),
        Some(AnyValueKind::IntValue(number)) => number.to_string(),
        Some(AnyValueKind::DoubleValue(number)) => number.to_string(),
        Some(AnyValueKind::ArrayValue(array)) => array
            .values
            .iter()
            .map(any_value_text)
            .collect::<Vec<_>>()
            .join(","),
        Some(
            AnyValueKind::KvlistValue(_)
            | AnyValueKind::BytesValue(_)
            | AnyValueKind::StringValueStrindex(_),
        )
        | None => String::new(),
    }
}
