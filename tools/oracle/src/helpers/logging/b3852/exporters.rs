use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::otel::config::{ExporterOwner, ExporterSpec, OTelConfigError};
use betterlite_logging::otel::plumbing::exporters::{
    ExportResult, ExporterFactories, ExporterFactory, HttpClients,
};
use betterlite_logging::otel::plumbing::sinks::SpanSink;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer};

// the custom exporter kind both sides register, so a routed provider builds an in-memory sink
const KIND: &str = "b3852_memory";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExporterInput {
    kind: String,
    endpoint: Option<String>,
    headers: Option<String>,
    owner: Option<String>,
    use_simple_processor: bool,
}

#[derive(Clone, Default)]
pub(super) struct Built(Arc<BuiltInner>);

#[derive(Default)]
struct BuiltInner {
    specs: Mutex<Vec<ExporterSpec>>,
    sinks: Mutex<Vec<InMemorySpanExporter>>,
}

impl Built {
    fn record(&self, spec: &ExporterSpec, sink: &InMemorySpanExporter) {
        self.0
            .specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(spec.clone());
        self.0
            .sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sink.clone());
    }

    pub(super) fn builds(&self) -> usize {
        self.0
            .specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn specs(&self) -> Vec<ExporterSpec> {
        self.0
            .specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn sinks(&self) -> Vec<InMemorySpanExporter> {
        self.0
            .sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// counts the ERROR records the once-only metric filter log emits
#[derive(Clone, Default)]
pub(super) struct Captured(pub(super) Arc<AtomicUsize>);

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn exporter_spec(input: &ExporterInput) -> Result<ExporterSpec, Box<HelperError>> {
    Ok(ExporterSpec {
        kind: input.kind.clone(),
        endpoint: input.endpoint.clone(),
        traces_endpoint: None,
        headers: input.headers.clone(),
        owner: input.owner.as_deref().map(exporter_owner).transpose()?,
        options: None,
        use_simple_processor: Some(input.use_simple_processor),
        requires_headers: false,
    })
}

fn exporter_owner(name: &str) -> Result<ExporterOwner, Box<HelperError>> {
    let owners = [
        ("arize", ExporterOwner::ArizeAx),
        ("arize_phoenix", ExporterOwner::ArizePhoenix),
        ("langfuse_otel", ExporterOwner::LangfuseOtel),
        ("weave_otel", ExporterOwner::WeaveOtel),
        ("levo", ExporterOwner::Levo),
        ("agentops", ExporterOwner::Agentops),
        ("newrelic", ExporterOwner::Newrelic),
        ("signoz", ExporterOwner::Signoz),
    ];
    owners
        .iter()
        .find_map(|(candidate, owner)| (*candidate == name).then_some(*owner))
        .ok_or_else(|| OTelConfigError::ExporterOwner(name.to_owned()))
        .or_config_error()
}

// every provider either side builds routes the custom kind to a fresh in-memory sink, and the recorded spec is what the cases compare
pub(super) fn factories(built: &Built) -> Arc<ExporterFactories> {
    let recorder = built.clone();
    let factory: ExporterFactory = Arc::new(move |spec: &ExporterSpec| -> ExportResult<SpanSink> {
        let sink = InMemorySpanExporter::default();
        recorder.record(spec, &sink);
        Ok(SpanSink::InMemory(sink))
    });
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    Arc::new(ExporterFactories::new(
        HttpClients {
            traces: Arc::clone(&client),
            metrics: Arc::clone(&client),
            logs: client,
        },
        BTreeMap::from([(KIND.to_owned(), factory)]),
    ))
}

#[derive(Debug)]
struct NoClient;

#[async_trait::async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the lifecycle scenarios export in memory"))
    }
}
