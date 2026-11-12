#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use opentelemetry::InstrumentationScope;
use opentelemetry::logs::{Logger as _, LoggerProvider as _};
use opentelemetry::trace::{Span as _, Tracer as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use rstest::rstest;

use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::headers::HeadersError;
use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::legacy_policy::capture::LoggingFlags;
use crate::otel::legacy_policy::config::{Exporter, MetadataKeys, OpenTelemetryConfig, Signals};
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::{
    GlobalProviders, ProviderSlot as _,
};
use crate::otel_provider_assembly::proxy_registry::{
    OtelRegistration, ProxyRegistry, ServiceCallback,
};

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no HTTP client is wired into this test"))
    }
}

#[derive(Debug, Default)]
struct PostedUris(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl HttpClient for PostedUris {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        self.0
            .lock()
            .expect("uri lock")
            .push(request.uri().to_string());
        Ok(Response::new(Bytes::new()))
    }
}

fn shared_clients<C>(client: &Arc<C>) -> HttpClients
where
    C: HttpClient + 'static,
{
    let shared: Arc<dyn HttpClient> = Arc::<C>::clone(client);
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

fn env_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// The in_memory env branch of `from_env` returns the plain defaults with only the exporter set.
fn in_memory_config() -> OpenTelemetryConfig {
    let mut config = OpenTelemetryConfig::from_env(&env_of(&[("OTEL_EXPORTER", "in_memory")]));
    config.service_name = "mine".to_owned();
    config.deployment_environment = "staging".to_owned();
    config.model_id = "model-7".to_owned();
    config
}

fn plain_init<'keys>(config: Option<OpenTelemetryConfig>) -> LegacyOtelInit<'keys> {
    LegacyOtelInit {
        config,
        callback_name: None,
        overrides: MetadataKeyOverrides {
            team: None,
            metadata: None,
        },
        injected: InjectedProviders {
            tracer: None,
            meter: None,
            logger: None,
        },
        max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
    }
}

struct Rig {
    env: BTreeMap<String, String>,
    globals: GlobalProviders,
    proxy: ProxyRegistry,
    exporter: InMemorySpanExporter,
}

impl Rig {
    fn new() -> Self {
        Self {
            env: BTreeMap::new(),
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
        }
    }

    fn init(&self, init: LegacyOtelInit<'_>) -> Result<LegacyOtel, LegacyExporterError> {
        self.init_on(init, shared_clients(&Arc::new(NoServer)))
    }

    fn init_on(
        &self,
        init: LegacyOtelInit<'_>,
        clients: HttpClients,
    ) -> Result<LegacyOtel, LegacyExporterError> {
        init_legacy_otel(
            init,
            &LegacyOtelEnvironment {
                env: &self.env,
                clients,
                shared_exporter: self.exporter.clone(),
                globals: &self.globals,
                proxy: Some(&self.proxy),
            },
        )
    }

    fn init_with(&self, config: OpenTelemetryConfig, callback: Option<&str>) -> LegacyOtel {
        let mut init = plain_init(Some(config));
        init.callback_name = callback.map(str::to_owned);
        self.init(init).expect("legacy init")
    }
}

fn record_span(otel: &LegacyOtel, name: &'static str) {
    otel.tracer.start(name).end();
}

fn exported_names(exporter: &InMemorySpanExporter) -> Vec<String> {
    exporter
        .get_finished_spans()
        .expect("finished spans")
        .iter()
        .map(|span| span.name.to_string())
        .collect()
}

#[test]
fn no_config_is_read_from_the_environment_and_normalized() {
    let mut rig = Rig::new();
    rig.env = env_of(&[
        ("OTEL_EXPORTER", "console"),
        ("OTEL_SERVICE_NAME", "from-env"),
        ("OTEL_IGNORE_CONTEXT_PROPAGATION", "true"),
    ]);
    let otel = rig.init(LegacyOtelInit::default()).expect("legacy init");
    assert_eq!(otel.config.exporter, Exporter::Console);
    assert_eq!(otel.config.service_name, "from-env");
    assert!(otel.config.ignore_context_propagation);
}

#[test]
fn a_supplied_config_is_kept_and_the_environment_is_not_read() {
    let mut rig = Rig::new();
    rig.env = env_of(&[
        ("OTEL_EXPORTER", "console"),
        ("OTEL_SERVICE_NAME", "ignored"),
    ]);
    let otel = rig.init_with(in_memory_config(), None);
    assert_eq!(otel.config.exporter, Exporter::InMemory);
    assert_eq!(otel.config.service_name, "mine");
    assert!(!otel.config.ignore_context_propagation);
}

#[test]
fn the_callback_name_is_kept() {
    let rig = Rig::new();
    let otel = rig.init_with(in_memory_config(), Some("arize"));
    assert_eq!(otel.callback_name.as_deref(), Some("arize"));
}

#[rstest]
#[case::text(Some(MetadataKeys::Text(" a, b ,,c")), vec!["a", "b", "c"])]
#[case::list(Some(MetadataKeys::List(&[])), Vec::<&str>::new())]
#[case::absent(None, vec!["kept"])]
fn the_baggage_overrides_replace_the_configured_keys(
    #[case] keys: Option<MetadataKeys<'static>>,
    #[case] expected: Vec<&str>,
) {
    let rig = Rig::new();
    let mut config = in_memory_config();
    config.baggage_team_metadata_keys = vec!["kept".to_owned()];
    config.baggage_metadata_keys = vec!["kept".to_owned()];
    let mut init = plain_init(Some(config));
    init.overrides = MetadataKeyOverrides {
        team: keys,
        metadata: keys,
    };
    let otel = rig.init(init).expect("legacy init");
    assert_eq!(otel.config.baggage_team_metadata_keys, expected);
    assert_eq!(otel.config.baggage_metadata_keys, expected);
}

#[test]
fn the_team_and_metadata_overrides_apply_to_their_own_key_lists() {
    let rig = Rig::new();
    let metadata = ["m1".to_owned(), " m2 ".to_owned()];
    let mut init = plain_init(Some(in_memory_config()));
    init.overrides = MetadataKeyOverrides {
        team: Some(MetadataKeys::Text("t1,t2")),
        metadata: Some(MetadataKeys::List(&metadata)),
    };
    let otel = rig.init(init).expect("legacy init");
    assert_eq!(otel.config.baggage_team_metadata_keys, vec!["t1", "t2"]);
    assert_eq!(otel.config.baggage_metadata_keys, vec!["m1", "m2"]);
}

#[test]
fn the_resource_carries_the_service_environment_and_model() {
    let rig = Rig::new();
    let otel = rig.init_with(in_memory_config(), None);
    let attributes: BTreeMap<String, String> = otel
        .resource
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    assert_eq!(attributes["service.name"], "mine");
    assert_eq!(attributes["deployment.environment"], "staging");
    assert_eq!(attributes["model_id"], "model-7");
}

#[rstest]
#[case::default_name(&[], "litellm")]
#[case::env_name(&[("OTEL_TRACER_NAME", "custom")], "custom")]
fn the_tracer_is_named_for_the_environment_and_exports_to_the_shared_exporter(
    #[case] env: &[(&str, &str)],
    #[case] expected: &str,
) {
    let mut rig = Rig::new();
    rig.env = env_of(env);
    let otel = rig.init_with(in_memory_config(), None);
    record_span(&otel, "work");
    let spans = rig.exporter.get_finished_spans().expect("finished spans");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].instrumentation_scope.name(), expected);
}

#[test]
fn a_new_tracer_provider_is_published_and_the_next_logger_reuses_it() {
    let rig = Rig::new();
    assert!(rig.globals.tracer.existing().expect("lookup").is_none());
    let first = rig.init_with(in_memory_config(), None);
    assert!(rig.globals.tracer.existing().expect("lookup").is_some());
    let other_exporter = InMemorySpanExporter::default();
    let second = init_legacy_otel(
        plain_init(Some(in_memory_config())),
        &LegacyOtelEnvironment {
            env: &rig.env,
            clients: shared_clients(&Arc::new(NoServer)),
            shared_exporter: other_exporter.clone(),
            globals: &rig.globals,
            proxy: Some(&rig.proxy),
        },
    )
    .expect("second init");
    record_span(&first, "from-first");
    record_span(&second, "from-second");
    assert_eq!(
        exported_names(&rig.exporter),
        vec!["from-first", "from-second"]
    );
    assert!(exported_names(&other_exporter).is_empty());
}

#[rstest]
#[case::config_flag(true, None)]
#[case::langfuse_callback(false, Some("langfuse_otel"))]
fn skipping_the_global_builds_a_private_provider_and_publishes_nothing(
    #[case] skip_flag: bool,
    #[case] callback: Option<&str>,
) {
    let rig = Rig::new();
    let mut config = in_memory_config();
    config.skip_set_global = skip_flag;
    let otel = rig.init_with(config, callback);
    assert!(rig.globals.tracer.existing().expect("lookup").is_none());
    record_span(&otel, "private");
    assert_eq!(exported_names(&rig.exporter), vec!["private"]);
}

#[test]
fn another_callback_name_still_publishes_the_global() {
    let rig = Rig::new();
    rig.init_with(in_memory_config(), Some("arize"));
    assert!(rig.globals.tracer.existing().expect("lookup").is_some());
}

#[test]
fn an_injected_tracer_provider_is_used_and_never_published() {
    let rig = Rig::new();
    let injected_exporter = InMemorySpanExporter::default();
    let injected = SdkTracerProvider::builder()
        .with_simple_exporter(injected_exporter.clone())
        .build();
    let mut init = plain_init(Some(in_memory_config()));
    init.injected.tracer = Some(injected);
    let otel = rig.init(init).expect("legacy init");
    record_span(&otel, "injected");
    assert_eq!(exported_names(&injected_exporter), vec!["injected"]);
    assert!(exported_names(&rig.exporter).is_empty());
    assert!(rig.globals.tracer.existing().expect("lookup").is_none());
}

#[test]
fn metrics_stay_off_unless_enabled_even_with_an_injected_provider() {
    let rig = Rig::new();
    let injected = SdkMeterProvider::builder().build();
    let mut init = plain_init(Some(in_memory_config()));
    init.injected.meter = Some(injected);
    let otel = rig.init(init).expect("legacy init");
    assert!(otel.metrics.is_none());
    assert!(rig.globals.meter.existing().expect("lookup").is_none());
}

#[test]
fn enabled_metrics_build_the_instruments_on_the_litellm_meter() {
    let rig = Rig::new();
    let exporter = InMemoryMetricExporter::default();
    let injected = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let mut config = in_memory_config();
    config.signals = Signals {
        enable_metrics: true,
        enable_events: false,
    };
    let mut init = plain_init(Some(config));
    init.injected.meter = Some(injected);
    let otel = rig.init(init).expect("legacy init");
    let metrics = otel.metrics.expect("metrics signal");
    metrics.instruments.operation_duration.record(1.0_f64, &[]);
    metrics.meter_provider.force_flush().expect("flush");
    let finished = exporter.get_finished_metrics().expect("finished metrics");
    let scopes: Vec<String> = finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .map(|scope| scope.scope().name().to_owned())
        .collect();
    assert_eq!(scopes, vec!["litellm.integrations.opentelemetry"]);
    assert!(rig.globals.meter.existing().expect("lookup").is_none());
}

#[test]
fn enabled_metrics_without_an_injected_provider_build_and_publish_one() {
    let rig = Rig::new();
    let mut config = in_memory_config();
    config.signals.enable_metrics = true;
    let otel = rig.init_with(config, None);
    assert!(otel.metrics.is_some());
    assert!(rig.globals.meter.existing().expect("lookup").is_some());
    assert!(rig.globals.logger.existing().expect("lookup").is_none());
}

fn otlp_config() -> OpenTelemetryConfig {
    let mut config = in_memory_config();
    config.exporter = Exporter::Named("otlp_http".to_owned());
    config.endpoint = Some("http://collector:4318".to_owned());
    config
}

#[test]
fn a_created_meter_provider_exports_to_the_metrics_endpoint() {
    let rig = Rig::new();
    let posted = Arc::new(PostedUris::default());
    let mut config = otlp_config();
    config.signals.enable_metrics = true;
    let otel = rig
        .init_on(plain_init(Some(config)), shared_clients(&posted))
        .expect("legacy init");
    let metrics = otel.metrics.expect("metrics signal");
    metrics.instruments.token_usage.record(3.0_f64, &[]);
    metrics.meter_provider.force_flush().expect("flush");
    assert_eq!(
        posted.0.lock().expect("uri lock").clone(),
        vec!["http://collector:4318/v1/metrics"]
    );
}

#[test]
fn a_created_logger_provider_exports_to_the_logs_endpoint() {
    let rig = Rig::new();
    let posted = Arc::new(PostedUris::default());
    let mut config = otlp_config();
    config.signals.enable_events = true;
    let otel = rig
        .init_on(plain_init(Some(config)), shared_clients(&posted))
        .expect("legacy init");
    let provider = otel.logger_provider.expect("logger provider");
    let logger = provider.logger_with_scope(InstrumentationScope::builder("init-test").build());
    logger.emit(logger.create_log_record());
    provider.force_flush().expect("flush");
    assert_eq!(
        posted.0.lock().expect("uri lock").clone(),
        vec!["http://collector:4318/v1/logs"]
    );
}

#[test]
fn logs_stay_off_unless_events_are_enabled() {
    let rig = Rig::new();
    let otel = rig.init_with(in_memory_config(), None);
    assert!(otel.logger_provider.is_none());
    assert!(rig.globals.logger.existing().expect("lookup").is_none());
}

#[test]
fn enabled_events_build_and_publish_a_logger_provider() {
    let rig = Rig::new();
    let mut config = in_memory_config();
    config.signals.enable_events = true;
    let otel = rig.init_with(config, None);
    assert!(otel.logger_provider.is_some());
    assert!(rig.globals.logger.existing().expect("lookup").is_some());
    assert!(rig.globals.meter.existing().expect("lookup").is_none());
}

#[test]
fn malformed_headers_fail_the_init() {
    let rig = Rig::new();
    let mut config = in_memory_config();
    config.headers = Some("oops".to_owned());
    let error = rig
        .init(plain_init(Some(config)))
        .err()
        .expect("init error");
    assert!(matches!(
        error,
        LegacyExporterError::Headers(HeadersError::MissingSeparator { position: 1 })
    ));
}

#[rstest]
#[case::legacy_flag(None, &[], "SPAN_AND_EVENT")]
#[case::configured(Some("SPAN_ONLY"), &[], "SPAN_ONLY")]
#[case::from_env(None, &[("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", "EVENT_ONLY")], "EVENT_ONLY")]
fn the_capture_policy_is_sampled_at_init(
    #[case] configured: Option<&str>,
    #[case] env: &[(&str, &str)],
    #[case] expected: &str,
) {
    let mut rig = Rig::new();
    rig.env = env_of(env);
    let mut config = in_memory_config();
    config.capture_message_content = configured.map(str::to_owned);
    let otel = rig.init_with(config, None);
    let flags = LoggingFlags {
        turn_off_message_logging: false,
        message_logging: true,
    };
    assert_eq!(otel.capture_policy.resolve(flags).as_str(), expected);
}

#[rstest]
#[case::default_limit(None, 2)]
#[case::one_provider(Some(1), 1)]
fn the_dynamic_provider_cap_reaches_the_tracer_cache(
    #[case] cap: Option<usize>,
    #[case] cached: usize,
) {
    let rig = Rig::new();
    let mut init = plain_init(Some(in_memory_config()));
    if let Some(limit) = cap {
        init.max_dynamic_tracer_providers = limit;
    }
    let otel = rig.init(init).expect("legacy init");
    for name in ["one", "two"] {
        let headers = BTreeMap::from([("x-key".to_owned(), name.to_owned())]);
        otel.tracer_cache
            .tracer_for_headers(&headers)
            .expect("dynamic tracer");
    }
    assert_eq!(otel.tracer_cache.cached_keys().len(), cached);
}

#[test]
fn the_dynamic_cache_inherits_the_callback_name_and_tracer_name() {
    let mut rig = Rig::new();
    rig.env = env_of(&[("OTEL_TRACER_NAME", "dynamic-name")]);
    let otel = rig.init_with(in_memory_config(), None);
    let headers = BTreeMap::from([("x-key".to_owned(), "value".to_owned())]);
    let tracer = otel
        .tracer_cache
        .tracer_for_headers(&headers)
        .expect("dynamic tracer");
    tracer.start("dynamic").end();
    let spans = rig.exporter.get_finished_spans().expect("finished spans");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].instrumentation_scope.name(), "dynamic-name");
}

#[test]
fn the_logger_registers_on_the_proxy() {
    let rig = Rig::new();
    rig.init_with(in_memory_config(), Some("first"));
    rig.init_with(in_memory_config(), Some("second"));
    let registration = OtelRegistration {
        callback_name: Some("first".to_owned()),
    };
    let callbacks = rig
        .proxy
        .service_callbacks
        .lock()
        .expect("callbacks")
        .clone();
    assert_eq!(
        callbacks,
        vec![ServiceCallback::OpenTelemetry(registration.clone())]
    );
    let slot = rig
        .proxy
        .open_telemetry_logger
        .lock()
        .expect("slot")
        .clone();
    assert_eq!(slot, Some(registration));
}

#[test]
fn init_without_a_proxy_registry_still_succeeds() {
    let rig = Rig::new();
    let otel = init_legacy_otel(
        plain_init(Some(in_memory_config())),
        &LegacyOtelEnvironment {
            env: &rig.env,
            clients: shared_clients(&Arc::new(NoServer)),
            shared_exporter: rig.exporter.clone(),
            globals: &rig.globals,
            proxy: None,
        },
    )
    .expect("legacy init");
    record_span(&otel, "no-proxy");
    assert_eq!(exported_names(&rig.exporter), vec!["no-proxy"]);
}

#[test]
fn the_default_init_caps_dynamic_providers_at_the_litellm_limit() {
    assert_eq!(LegacyOtelInit::default().max_dynamic_tracer_providers, 256);
}
