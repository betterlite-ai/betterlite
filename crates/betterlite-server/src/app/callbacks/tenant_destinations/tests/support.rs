use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::auth_admission::ports::TenantDestinations as _;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_core::api::request_setup_options::{
    CallbackVarsDecryption, RequestSetupOptions, RequestSetupSettings,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::otel::config::ExporterSpec;
use betterlite_logging::otel::config::destination::{
    DestinationFields, OtelDestination, SpanScope,
};
use betterlite_logging::otel::content_projections::StreamOutputAssembler;
use betterlite_logging::otel::fan_out::attach::attach_tenant_fan_out;
use betterlite_logging::otel::fan_out::overridden::OverriddenBackendFilter;
use betterlite_logging::otel::fan_out::processor::TenantFanOutSpanProcessor;
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger_runtime::LoggerRuntime;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::exporters::{
    ExportResult, ExporterError, ExporterFactories, ExporterFactory, HttpClients,
};
use betterlite_logging::otel::plumbing::sinks::SpanSink;
use betterlite_logging::otel::proxy_runtime::runtime::{ProxyRuntimeInputs, build_proxy_runtime};
use betterlite_logging::otel::proxy_runtime::slot::request_slot_scope;
use betterlite_logging::otel::proxy_runtime::sources::ProxyTraceSources;
use chrono::FixedOffset;
use indexmap::IndexMap;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor, SpanData, SpanProcessor as _,
};

use crate::app::callbacks::tenant_destinations::TenantDestinationAdapter;

pub(crate) const TENANT_A: &str = "tenant-a.example";
pub(crate) const UNBUILDABLE_HOST: &str = "unbuildable.example";
pub(crate) const NEWRELIC_US_ENDPOINT: &str = "https://otlp.nr-data.net";
pub(crate) const OPERATOR_ENDPOINT: &str = "https://operator.example/v1/traces";
const UNBUILDABLE: &str = "unbuildable";

pub(crate) type Adapter = TenantDestinationAdapter<(), Plaintext, BTreeMap<String, String>>;

pub(crate) struct Plaintext;

impl CallbackVarsDecryption for Plaintext {
    fn decrypt(&self, metadata: &Metadata) -> Metadata {
        metadata.clone()
    }
}

pub(crate) struct NoSettings;

impl CallbackSettingsView for NoSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}

pub(crate) struct NullAssembler;

impl StreamOutputAssembler for NullAssembler {
    fn assemble_sse(&self, _frames: &[Vec<u8>]) -> Option<OtelModelValue> {
        None
    }

    fn assemble_chat(
        &self,
        _chunks: &[OtelModelValue],
        _messages: Option<&[OtelModelValue]>,
    ) -> Option<OtelModelValue> {
        None
    }
}

#[derive(Debug)]
struct NoClient;

#[async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from(
            "the tenant destination tests record spans in memory",
        ))
    }
}

fn clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

pub(crate) fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    match value.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn build_runtime() -> Arc<LoggerRuntime> {
    let loaded = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .expect("the empty config loads");
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> =
        Arc::new(BTreeMap::<String, String>::new());
    let sources = Arc::new(ProxyTraceSources::new(environment.as_ref()));
    build_proxy_runtime(ProxyRuntimeInputs {
        config: &loaded,
        environment,
        http_clients: clients(),
        callback_settings: Arc::new(NoSettings),
        configured_exclusions: None,
        assembler: Arc::new(NullAssembler),
        utc_offset: FixedOffset::east_opt(0).expect("utc"),
        sources,
    })
    .expect("the proxy runtime builds")
}

pub(crate) struct RecordingSinks {
    built: Mutex<BTreeMap<String, BuiltSink>>,
}

#[derive(Debug)]
struct BuiltSink {
    exporter: InMemorySpanExporter,
    headers: String,
}

impl RecordingSinks {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            built: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn factories(self: &Arc<Self>) -> Arc<ExporterFactories> {
        let recorder = Arc::clone(self);
        let factory: ExporterFactory =
            Arc::new(move |spec: &ExporterSpec| build_sink(&recorder, spec));
        let custom = BTreeMap::from([("otlp_http".to_owned(), factory)]);
        Arc::new(ExporterFactories::new(clients(), custom))
    }

    pub(crate) fn spans(&self, endpoint: &str) -> Vec<SpanData> {
        lock(&self.built)
            .get(endpoint)
            .map_or_else(Vec::new, |sink| {
                sink.exporter
                    .get_finished_spans()
                    .expect("the in-memory sink keeps its spans")
            })
    }

    pub(crate) fn headers(&self, endpoint: &str) -> Option<String> {
        lock(&self.built)
            .get(endpoint)
            .map(|sink| sink.headers.clone())
    }
}

// a destination whose endpoint names the unbuildable host cannot become an exporter, like a backend without a transport
fn build_sink(recorder: &RecordingSinks, spec: &ExporterSpec) -> ExportResult<SpanSink> {
    let endpoint = spec.endpoint.clone().unwrap_or_default();
    if endpoint.contains(UNBUILDABLE) {
        return Err(ExporterError::UnknownKind(spec.kind.clone()));
    }
    let exporter = InMemorySpanExporter::default();
    lock(&recorder.built).insert(
        endpoint,
        BuiltSink {
            exporter: exporter.clone(),
            headers: spec.headers.clone().unwrap_or_default(),
        },
    );
    Ok(SpanSink::InMemory(exporter))
}

pub(crate) fn attach(
    runtime: &LoggerRuntime,
    sinks: &Arc<RecordingSinks>,
) -> TenantFanOutSpanProcessor {
    let mut registry = lock(&runtime.fan_out);
    let _attached = attach_tenant_fan_out(
        &mut registry,
        sinks.factories(),
        Arc::clone(&runtime.scope),
        &[],
        BTreeSet::new(),
    );
    registry
        .fan_out
        .clone()
        .expect("the runtime registers the fan-out")
}

pub(crate) fn build_provider(fan_out: TenantFanOutSpanProcessor) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(fan_out)
        .build()
}

pub(crate) fn operator_filter(
    runtime: &LoggerRuntime,
    sink: &InMemorySpanExporter,
) -> OverriddenBackendFilter {
    OverriddenBackendFilter {
        inner: Box::new(SimpleSpanProcessor::new(sink.clone())),
        owner: Some("langfuse_otel".to_owned()),
        scope: SpanScope::Full,
        sink: None,
        scope_source: Arc::clone(&runtime.scope),
    }
}

pub(crate) fn operator_provider(
    operator: OverriddenBackendFilter,
    fan_out: &TenantFanOutSpanProcessor,
) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(operator)
        .with_span_processor(fan_out.clone())
        .build()
}

pub(crate) fn end_span(provider: &SdkTracerProvider, name: &str) {
    let mut span = provider
        .tracer("tenant_destination_test")
        .start(name.to_owned());
    span.end();
}

pub(crate) fn end_database_span(provider: &SdkTracerProvider) {
    let mut span = provider
        .tracer("tenant_destination_test")
        .start("db_lookup");
    span.set_attribute(KeyValue::new("db.system", "postgresql"));
    span.end();
}

pub(crate) fn finished(sink: &InMemorySpanExporter) -> usize {
    sink.get_finished_spans()
        .expect("the sink keeps its spans")
        .len()
}

// tenant destinations batch their exports, so a test reads the recorder after the fan-out flushes
pub(crate) fn flush(fan_out: &TenantFanOutSpanProcessor) {
    fan_out
        .force_flush()
        .expect("the fan-out flushes its destination processors");
}

pub(crate) fn build_adapter(
    runtime: &Arc<LoggerRuntime>,
    settings: RequestSetupSettings,
) -> Arc<Adapter> {
    Arc::new(TenantDestinationAdapter {
        setup: RequestSetupOptions {
            settings,
            session_ids: (),
            decryption: Plaintext,
            environment: BTreeMap::new(),
        },
        runtime: Arc::clone(runtime),
    })
}

pub(crate) fn setup(allowed_hosts: Vec<AllowedDestination>) -> RequestSetupSettings {
    RequestSetupSettings {
        callback_disabling: true,
        missing_session_id: None,
        otel_v2_enabled: true,
        default_team_settings: Vec::new(),
        allowed_hosts,
    }
}

pub(crate) fn allowed_hosts(hosts: &[&str]) -> Vec<AllowedDestination> {
    hosts
        .iter()
        .map(|host| AllowedDestination::parse(host).expect("the allowed host parses"))
        .collect()
}

pub(crate) fn build_auth(json: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(json).expect("the key fixture parses");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture constructs")
}

pub(crate) fn build_key(metadata: &str) -> UserAPIKeyAuth {
    build_auth(&format!(
        r#"{{"api_key": "sk-test", "metadata": {metadata}}}"#
    ))
}

pub(crate) fn build_key_with_team(metadata: &str, team_metadata: &str) -> UserAPIKeyAuth {
    build_auth(&format!(
        r#"{{"api_key": "sk-test", "metadata": {metadata}, "team_metadata": {team_metadata}}}"#
    ))
}

pub(crate) fn build_key_with_disable_list(names: &str, entries: &str) -> UserAPIKeyAuth {
    build_key(&format!(
        r#"{{"litellm_disabled_callbacks": [{names}], "logging": [{entries}]}}"#
    ))
}

pub(crate) fn logging(entries: &str) -> String {
    format!(r#"{{"logging": [{entries}]}}"#)
}

pub(crate) fn langfuse_entry(host: &str, public: &str, secret: &str) -> String {
    format!(
        r#"{{"callback_name": "langfuse_otel", "callback_vars": {{"langfuse_host": "{host}", "langfuse_public_key": "{public}", "langfuse_secret_key": "{secret}"}}}}"#
    )
}

pub(crate) fn langfuse_host_entry(host: &str) -> String {
    format!(
        r#"{{"callback_name": "langfuse_otel", "callback_vars": {{"langfuse_host": "{host}"}}}}"#
    )
}

pub(crate) fn failure_langfuse_entry(public: &str, secret: &str) -> String {
    format!(
        r#"{{"callback_name": "langfuse_otel", "callback_type": "failure", "callback_vars": {{"langfuse_public_key": "{public}", "langfuse_secret_key": "{secret}"}}}}"#
    )
}

pub(crate) fn newrelic_entry(api_key: &str) -> String {
    format!(
        r#"{{"callback_name": "newrelic", "callback_vars": {{"newrelic_api_key": "{api_key}"}}}}"#
    )
}

pub(crate) fn langfuse_endpoint(host: &str) -> String {
    format!("https://{host}/api/public/otel")
}

pub(crate) fn basic(public: &str, secret: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{public}:{secret}")))
}

pub(crate) fn authorization(destination: &OtelDestination) -> Option<&str> {
    destination
        .as_ref()
        .headers
        .get("Authorization")
        .map(String::as_str)
}

pub(crate) fn callback_names(destinations: &[OtelDestination]) -> Vec<String> {
    destinations
        .iter()
        .map(|destination| {
            destination
                .as_ref()
                .callback_name
                .clone()
                .expect("a resolved destination names its backend")
        })
        .collect()
}

pub(crate) fn endpoints(destinations: &[OtelDestination]) -> Vec<String> {
    destinations
        .iter()
        .map(|destination| destination.as_ref().endpoint.clone())
        .collect()
}

pub(crate) fn slot(runtime: &LoggerRuntime) -> Vec<OtelDestination> {
    runtime.scope.scope().destinations
}

pub(crate) async fn seeded_names(
    runtime: &LoggerRuntime,
    adapter: &Adapter,
    key: &UserAPIKeyAuth,
    headers: &IndexMap<String, String>,
) -> Vec<String> {
    let (_, names) = request_slot_scope(async {
        adapter.seed(key, headers).expect("the key seeds");
        callback_names(&slot(runtime))
    })
    .await;
    names
}

pub(crate) fn prior_destination() -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: OPERATOR_ENDPOINT.to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: Some("operator".to_owned()),
        protocol: None,
        span_scope: SpanScope::Full,
    })
}

pub(crate) fn no_headers() -> IndexMap<String, String> {
    IndexMap::new()
}

pub(crate) fn disable_header(names: &str) -> IndexMap<String, String> {
    let mut headers = IndexMap::new();
    headers.insert("x-litellm-disable-callbacks".to_owned(), names.to_owned());
    headers
}

pub(crate) fn headers_of(sinks: &RecordingSinks, endpoint: &str) -> String {
    sinks.headers(endpoint).expect("the tenant exporter builds")
}

// the Authorization pair exactly as the built exporter receives it, percent-encoding included
pub(crate) fn authorization_pair(destinations: &[OtelDestination]) -> String {
    let headers = destinations
        .first()
        .expect("a tenant destination resolves")
        .header_string();
    headers
        .split(',')
        .find(|pair| pair.starts_with("Authorization="))
        .map(str::to_owned)
        .expect("the destination carries its authorization")
}
