#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use opentelemetry::trace::Status;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use rstest::rstest;

use super::{
    ArizeDynamicOtel, ArizeHealth, ArizeSampling, CALLBACK_NAME, ERROR_RATE_VAR, HEALTHY_MESSAGE,
    MISSING_API_KEY_MESSAGE, MISSING_SPACE_MESSAGE, SUCCESS_RATE_VAR, arize_lifecycle,
    health_check, otel_config, system_draw,
};
use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::legacy_lifecycle::lifecycle::{
    DEFAULT_HANDLER_CLASS_NAME, ExportGate as _, LegacyLifecycle, LegacyLifecycleOptions,
    LifecycleCall, LifecycleEvent,
};
use crate::otel::legacy_lifecycle::tracer::{BaseDynamicOtel, DynamicOtel as _, TracerChoice};
use crate::otel::legacy_policy::capture::LoggingFlags;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::SystemClock;
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel::v2_callback::live_span::RootSpans;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use crate::otel_provider_assembly::proxy_registry::{
    OtelRegistration, ProxyRegistry, ServiceCallback, register_on_proxy,
};

const START: f64 = 1_704_067_200.0;
const END: f64 = 1_704_067_201.5;

#[derive(Debug)]
struct NoServer;

#[async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no HTTP client is wired into this suite"))
    }
}

fn shared_clients() -> HttpClients {
    let shared: Arc<dyn HttpClient> = Arc::new(NoServer);
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

struct Rig {
    env: BTreeMap<String, String>,
    globals: GlobalProviders,
    proxy: ProxyRegistry,
    exporter: InMemorySpanExporter,
    roots: Arc<RootSpans>,
}

impl Rig {
    fn new() -> Self {
        Self {
            env: BTreeMap::from([("OTEL_EXPORTER".to_owned(), "in_memory".to_owned())]),
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
            roots: Arc::new(RootSpans::default()),
        }
    }

    fn arize(&self) -> LegacyLifecycle {
        arize_lifecycle(
            self.init(
                OpenTelemetryConfig::from_env(&self.env),
                Some(CALLBACK_NAME),
            ),
            self.options(),
        )
    }

    fn options(&self) -> LegacyLifecycleOptions {
        LegacyLifecycleOptions {
            dynamic: Arc::new(BaseDynamicOtel),
            live: Arc::clone(&self.roots),
            metrics: None,
            events: None,
            failure_counters: Vec::new(),
            handler_class_name: DEFAULT_HANDLER_CLASS_NAME.to_owned(),
            use_request_span: false,
            clock: Box::new(SystemClock),
        }
    }

    fn init(&self, config: OpenTelemetryConfig, callback_name: Option<&str>) -> LegacyOtel {
        init_legacy_otel(
            LegacyOtelInit {
                config: Some(config),
                callback_name: callback_name.map(str::to_owned),
                overrides: MetadataKeyOverrides::default(),
                injected: InjectedProviders::default(),
                max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
            },
            &LegacyOtelEnvironment {
                env: &self.env,
                clients: shared_clients(),
                shared_exporter: self.exporter.clone(),
                globals: &self.globals,
                proxy: Some(&self.proxy),
            },
        )
        .expect("legacy otel init")
    }

    fn spans(&self) -> Vec<SpanData> {
        self.exporter.get_finished_spans().expect("finished spans")
    }
}

fn kwargs_fixture(json: &str) -> MetadataMap {
    serde_json::from_str(json).expect("kwargs fixture")
}

fn base_kwargs(call_id: &str) -> MetadataMap {
    kwargs_fixture(&format!(
        r#"{{
            "model": "gpt-4o",
            "messages": [{{"role": "user", "content": "hello"}}],
            "call_type": "completion",
            "optional_params": {{}},
            "litellm_params": {{"custom_llm_provider": "openai", "metadata": {{"litellm_call_id": "{call_id}"}}}},
            "standard_logging_object": {{
                "id": "chatcmpl-1",
                "call_type": "completion",
                "status": "success",
                "metadata": {{"litellm_call_id": "{call_id}"}},
                "hidden_params": {{}}
            }}
        }}"#
    ))
}

fn with_dynamic(kwargs: &mut MetadataMap, json: &str) {
    kwargs.insert(
        "standard_callback_dynamic_params".to_owned(),
        serde_json::from_str(json).expect("dynamic params fixture"),
    );
}

fn dynamic_kwargs(call_id: &str, json: &str) -> MetadataMap {
    let mut kwargs = base_kwargs(call_id);
    with_dynamic(&mut kwargs, json);
    kwargs
}

fn call(kwargs: &MetadataMap) -> LifecycleCall<'_> {
    LifecycleCall {
        kwargs,
        response: None,
        start_time: SpanTime::Seconds(START),
        end_time: SpanTime::Seconds(END),
        exception: None,
        flags: LoggingFlags {
            turn_off_message_logging: false,
            message_logging: false,
        },
    }
}

fn gate_with(draw: f64) -> ArizeSampling {
    ArizeSampling::new(move || draw)
}

fn attribute(spans: &[SpanData], name: &str) -> Option<String> {
    let span = spans.first()?;
    span.attributes
        .iter()
        .find(|pair| pair.key.as_str() == name)
        .map(|pair| pair.value.as_str().into_owned())
}

fn env_map(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn arize_answers_to_the_arize_callback_name() {
    let lifecycle = Rig::new().arize();
    assert_eq!(lifecycle.callback_name.as_deref(), Some(CALLBACK_NAME));
    assert_eq!(CALLBACK_NAME, "arize");
    assert!(
        lifecycle.export_gate.is_some(),
        "the sampling gate is installed"
    );
}

#[test]
fn arize_coexists_with_the_primary_otel_registration() {
    let rig = Rig::new();
    register_on_proxy(
        &rig.proxy,
        &OtelRegistration {
            callback_name: Some("otel".to_owned()),
        },
    );
    let _lifecycle = rig.arize();
    let slot = rig.proxy.open_telemetry_logger.lock().expect("the slot");
    assert_eq!(
        slot.as_ref()
            .and_then(|entry| entry.callback_name.as_deref()),
        Some("otel")
    );
    drop(slot);
    let callbacks = rig.proxy.service_callbacks.lock().expect("the callbacks");
    assert_eq!(callbacks.len(), 1);
    assert!(matches!(
        callbacks.first(),
        Some(ServiceCallback::OpenTelemetry(entry)) if entry.callback_name.as_deref() == Some("otel")
    ));
}

#[test]
fn arize_sampled_out_success_writes_no_span() {
    let rig = Rig::new();
    let mut lifecycle = rig.arize();
    lifecycle.export_gate = Some(Arc::new(gate_with(0.0)));
    let kwargs = dynamic_kwargs("call-out", r#"{"arize_success_sampling_rate": 0.0}"#);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the sampled-out success");
    assert!(rig.spans().is_empty());
}

#[test]
fn arize_sampled_in_success_writes_one_span() {
    let rig = Rig::new();
    let mut lifecycle = rig.arize();
    lifecycle.export_gate = Some(Arc::new(gate_with(1.0)));
    let kwargs = dynamic_kwargs("call-in", r#"{"arize_success_sampling_rate": 1.0}"#);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the sampled-in success");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert!(matches!(spans[0].status, Status::Ok));
}

#[test]
fn arize_draw_above_the_rate_drops_the_success() {
    let rig = Rig::new();
    let mut lifecycle = rig.arize();
    lifecycle.export_gate = Some(Arc::new(gate_with(0.6)));
    let kwargs = dynamic_kwargs("call-above", r#"{"arize_success_sampling_rate": 0.5}"#);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the rejected success");
    assert!(rig.spans().is_empty());
}

#[test]
fn arize_draw_at_the_rate_keeps_the_success() {
    let rig = Rig::new();
    let mut lifecycle = rig.arize();
    lifecycle.export_gate = Some(Arc::new(gate_with(0.5)));
    let kwargs = dynamic_kwargs("call-at", r#"{"arize_success_sampling_rate": 0.5}"#);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the accepted success");
    assert_eq!(rig.spans().len(), 1);
}

#[test]
fn arize_failure_reads_the_error_rate() {
    let rig = Rig::new();
    let mut lifecycle = rig.arize();
    lifecycle.export_gate = Some(Arc::new(gate_with(0.1)));
    let kwargs = dynamic_kwargs(
        "call-failure",
        r#"{"arize_success_sampling_rate": 0.0, "arize_error_sampling_rate": 1.0}"#,
    );
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Failure)
        .expect("the sampled-in failure");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert!(matches!(spans[0].status, Status::Error { .. }));
}

#[test]
fn arize_sampled_out_failure_writes_no_span() {
    let rig = Rig::new();
    let mut lifecycle = rig.arize();
    lifecycle.export_gate = Some(Arc::new(gate_with(0.1)));
    let kwargs = dynamic_kwargs(
        "call-error-out",
        r#"{"arize_success_sampling_rate": 1.0, "arize_error_sampling_rate": 0.0}"#,
    );
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Failure)
        .expect("the sampled-out failure");
    assert!(rig.spans().is_empty());
}

#[test]
fn arize_exports_when_no_rate_is_configured() {
    let rig = Rig::new();
    let mut lifecycle = rig.arize();
    lifecycle.export_gate = Some(Arc::new(gate_with(0.99)));
    let kwargs = base_kwargs("call-no-rate");
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the unsampled success");
    assert_eq!(rig.spans().len(), 1);
}

#[test]
fn arize_zero_rate_never_draws() {
    let draws = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&draws);
    let sampling = ArizeSampling::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        0.0
    });
    let kwargs = dynamic_kwargs("call-zero", r#"{"arize_success_sampling_rate": 0.0}"#);
    assert!(
        !sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .expect("the decision")
    );
    assert_eq!(draws.load(Ordering::SeqCst), 0);
}

#[test]
fn arize_caches_the_decision_per_request() {
    let draws = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&draws);
    let sampling = ArizeSampling::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        0.9
    });
    let kwargs = dynamic_kwargs("call-cached", r#"{"arize_success_sampling_rate": 0.5}"#);
    assert!(
        !sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .expect("first")
    );
    assert!(
        !sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .expect("cached")
    );
    assert_eq!(draws.load(Ordering::SeqCst), 1);
}

#[test]
fn arize_success_and_error_decisions_are_separate() {
    let sampling = gate_with(0.9);
    let kwargs = dynamic_kwargs(
        "call-separate",
        r#"{"arize_success_sampling_rate": 1.0, "arize_error_sampling_rate": 0.5}"#,
    );
    assert!(
        sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .expect("success")
    );
    assert!(
        !sampling
            .should_export(&kwargs, LifecycleEvent::Failure)
            .expect("failure")
    );
}

#[test]
fn arize_keeps_both_decisions_of_a_request() {
    let draws = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&draws);
    let sampling = ArizeSampling::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        0.9
    });
    let kwargs = dynamic_kwargs(
        "call-both-decisions",
        r#"{"arize_success_sampling_rate": 1.0, "arize_error_sampling_rate": 1.0}"#,
    );
    assert!(
        sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .expect("the success")
    );
    assert!(
        sampling
            .should_export(&kwargs, LifecycleEvent::Failure)
            .expect("the failure")
    );
    assert!(
        sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .expect("the cached success")
    );
    assert!(
        sampling
            .should_export(&kwargs, LifecycleEvent::Failure)
            .expect("the cached failure")
    );
    assert_eq!(draws.load(Ordering::SeqCst), 2);
}

fn draws(count: usize) -> (f64, f64, bool) {
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut nonzero = false;
    for _ in 0..count {
        let draw = system_draw();
        low = low.min(draw);
        high = high.max(draw);
        nonzero = nonzero || draw > 0.0;
    }
    (low, high, nonzero)
}

#[test]
fn arize_default_draw_stays_inside_the_unit_interval() {
    let (low, high, nonzero) = draws(64);
    assert!(low >= 0.0, "the lowest draw {low} is below zero");
    assert!(high < 1.0, "the highest draw {high} reaches one");
    assert!(nonzero, "every draw was exactly zero");
}

#[rstest]
#[case::above_one(1.5)]
#[case::below_zero(-0.5)]
fn arize_out_of_range_rate_still_exports(#[case] rate: f64) {
    let sampling = gate_with(1.0);
    let mut kwargs = base_kwargs("call-range");
    with_dynamic(&mut kwargs, &format!(r#"{{"{SUCCESS_RATE_VAR}": {rate}}}"#));
    assert!(
        sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .expect("the decision")
    );
}

#[test]
fn arize_wrong_rate_type_is_an_error() {
    let sampling = gate_with(0.0);
    let kwargs = dynamic_kwargs("call-type", r#"{"arize_success_sampling_rate": "0.5"}"#);
    assert!(
        sampling
            .should_export(&kwargs, LifecycleEvent::Success)
            .is_err()
    );
}

#[test]
fn arize_writes_openinference_attributes() {
    let rig = Rig::new();
    let lifecycle = rig.arize();
    let kwargs = base_kwargs("call-attributes");
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the arize success");
    let spans = rig.spans();
    assert_eq!(
        attribute(&spans, "openinference.span.kind").as_deref(),
        Some("LLM")
    );
    assert_eq!(
        attribute(&spans, "llm.model_name").as_deref(),
        Some("gpt-4o")
    );
}

#[test]
fn arize_dynamic_headers_prefer_the_space_key() {
    let headers = ArizeDynamicOtel
        .construct_dynamic_otel_headers(&dynamic_params(&[
            ("arize_space_id", "space-id"),
            ("arize_space_key", "space-key"),
            ("arize_api_key", "api-key"),
        ]))
        .expect("the dynamic headers");
    assert_eq!(
        headers.get("arize-space-id").map(String::as_str),
        Some("space-key")
    );
    assert_eq!(headers.get("api_key").map(String::as_str), Some("api-key"));
}

#[test]
fn arize_dynamic_headers_fall_back_to_the_space_id() {
    let headers = ArizeDynamicOtel
        .construct_dynamic_otel_headers(&dynamic_params(&[("arize_space_id", "space-id")]))
        .expect("the dynamic headers");
    assert_eq!(
        headers.get("arize-space-id").map(String::as_str),
        Some("space-id")
    );
    assert!(!headers.contains_key("api_key"));
}

#[test]
fn arize_dynamic_headers_are_absent_without_credentials() {
    assert!(
        ArizeDynamicOtel
            .construct_dynamic_otel_headers(&MetadataMap::new())
            .is_none()
    );
    assert!(
        ArizeDynamicOtel
            .construct_dynamic_otel_headers(&dynamic_params(&[
                ("arize_space_key", ""),
                ("arize_api_key", ""),
            ]))
            .is_none()
    );
}

fn dynamic_params(pairs: &[(&str, &str)]) -> MetadataMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                OtelModelValue::String((*value).to_owned()),
            )
        })
        .collect()
}

#[test]
fn arize_reads_the_dynamic_headers_from_the_request_params() {
    let lifecycle = Rig::new().arize();
    let kwargs = dynamic_kwargs(
        "call-dynamic",
        r#"{"arize_space_key": "space-key", "arize_api_key": "api-key"}"#,
    );
    let TracerChoice::DynamicHeaders(headers) =
        lifecycle.tracer_choice(&kwargs).expect("the tracer choice")
    else {
        panic!("the arize callback builds dynamic headers");
    };
    assert_eq!(
        headers.get("arize-space-id").map(String::as_str),
        Some("space-key")
    );
    assert_eq!(headers.get("api_key").map(String::as_str), Some("api-key"));
}

#[test]
fn arize_otel_config_points_at_the_configured_endpoint() {
    let config = otel_config(&env_map(&[
        ("ARIZE_ENDPOINT", "https://grpc.arize.test:443"),
        ("ARIZE_SPACE_KEY", "space-key"),
        ("ARIZE_API_KEY", "api-key"),
        ("ARIZE_PROJECT_NAME", "project"),
    ]));
    assert_eq!(config.exporter, Exporter::Named("otlp_grpc".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://grpc.arize.test:443")
    );
    assert_eq!(
        config.headers.as_deref(),
        Some("space_id=space-key,api_key=api-key")
    );
    assert_eq!(config.service_name, "project");
    assert_eq!(config.model_id, "project");
}

#[test]
fn arize_otel_config_defaults_the_grpc_endpoint_and_the_service_name() {
    let config = otel_config(&env_map(&[]));
    assert_eq!(config.exporter, Exporter::Named("otlp_grpc".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://otlp.arize.com/v1")
    );
    assert!(config.headers.is_none());
    assert_eq!(config.service_name, "litellm");
}

#[test]
fn arize_otel_config_takes_the_http_endpoint_when_it_is_the_only_one() {
    let config = otel_config(&env_map(&[(
        "ARIZE_HTTP_ENDPOINT",
        "https://otlp.arize.test/v1/traces",
    )]));
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://otlp.arize.test/v1/traces")
    );
}

#[test]
fn arize_health_check_accepts_a_space_id_and_an_api_key() {
    assert_eq!(
        health_check(&env_map(&[
            ("ARIZE_SPACE_ID", "space-id"),
            ("ARIZE_API_KEY", "api-key"),
        ])),
        ArizeHealth::Healthy {
            message: HEALTHY_MESSAGE
        }
    );
}

#[test]
fn arize_health_check_reports_the_missing_space() {
    assert_eq!(
        health_check(&env_map(&[("ARIZE_API_KEY", "api-key")])),
        ArizeHealth::Unhealthy {
            error_message: MISSING_SPACE_MESSAGE.to_owned()
        }
    );
}

#[test]
fn arize_health_check_reports_the_missing_api_key() {
    assert_eq!(
        health_check(&env_map(&[("ARIZE_SPACE_KEY", "space-key")])),
        ArizeHealth::Unhealthy {
            error_message: MISSING_API_KEY_MESSAGE.to_owned()
        }
    );
}

#[test]
fn arize_health_check_reads_an_empty_space_as_unset() {
    assert_eq!(
        health_check(&env_map(&[
            ("ARIZE_SPACE_ID", ""),
            ("ARIZE_API_KEY", "api-key"),
        ])),
        ArizeHealth::Unhealthy {
            error_message: MISSING_SPACE_MESSAGE.to_owned()
        }
    );
}

#[test]
fn arize_default_sampling_draws_between_zero_and_one() {
    let sampling = ArizeSampling::default();
    let always = dynamic_kwargs("call-default-in", r#"{"arize_success_sampling_rate": 1.0}"#);
    assert!(
        sampling
            .should_export(&always, LifecycleEvent::Success)
            .expect("the decision")
    );
    let never = dynamic_kwargs(
        "call-default-out",
        r#"{"arize_success_sampling_rate": 0.0}"#,
    );
    assert!(
        !sampling
            .should_export(&never, LifecycleEvent::Success)
            .expect("the decision")
    );
}

#[test]
fn arize_rate_variables_name_the_dynamic_params() {
    assert_eq!(SUCCESS_RATE_VAR, "arize_success_sampling_rate");
    assert_eq!(ERROR_RATE_VAR, "arize_error_sampling_rate");
}
