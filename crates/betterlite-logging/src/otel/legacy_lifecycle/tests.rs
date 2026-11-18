#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};

use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::legacy_lifecycle::lifecycle::{LegacyLifecycle, LegacyLifecycleOptions};
use crate::otel::legacy_lifecycle::logger::LegacyOtelLogger;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use crate::otel_provider_assembly::proxy_registry::ProxyRegistry;

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no HTTP client is wired into this test"))
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
}

impl Rig {
    fn new() -> Self {
        let env = BTreeMap::from([("OTEL_EXPORTER".to_owned(), "in_memory".to_owned())]);
        Self {
            env,
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
        }
    }

    fn lifecycle(&self, callback_name: Option<&str>) -> LegacyLifecycle {
        let init = LegacyOtelInit {
            config: Some(OpenTelemetryConfig::from_env(&self.env)),
            callback_name: callback_name.map(str::to_owned),
            overrides: MetadataKeyOverrides::default(),
            injected: InjectedProviders::default(),
            max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
        };
        let otel = init_legacy_otel(
            init,
            &LegacyOtelEnvironment {
                env: &self.env,
                clients: shared_clients(),
                shared_exporter: self.exporter.clone(),
                globals: &self.globals,
                proxy: Some(&self.proxy),
            },
        )
        .expect("legacy otel init");
        LegacyLifecycle::from_otel(otel, LegacyLifecycleOptions::default())
    }

    fn spans(&self) -> Vec<SpanData> {
        self.exporter.get_finished_spans().expect("finished spans")
    }

    fn names(&self) -> Vec<String> {
        self.spans()
            .iter()
            .map(|span| span.name.to_string())
            .collect()
    }
}

fn event(id: &str) -> LoggerEvent {
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(PAYLOAD).expect("payload fixture");
    id.clone_into(&mut payload.id);
    LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

#[test]
fn callback_logger_exposes_its_name_and_logging_flags() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.lifecycle(None)));
    assert_eq!(logger.callback_name(), "opentelemetry");
    assert!(logger.message_logging());
    assert!(!logger.turn_off_message_logging());
    assert!(!logger.redacts_messages_itself());
    let named = LegacyOtelLogger::new(Arc::new(rig.lifecycle(Some("arize"))));
    assert_eq!(named.callback_name(), "arize");
}

#[test]
fn callback_logger_success_event_writes_the_primary_and_raw_spans() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.lifecycle(None)));
    logger
        .log_success_event(&event("call-1"))
        .expect("success event");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
    let spans = rig.spans();
    assert_eq!(spans[0].status, opentelemetry::trace::Status::Ok);
    assert_eq!(
        spans[1].parent_span_id,
        spans[0].span_context.span_id(),
        "the raw request span is a child of the primary span"
    );
}

#[test]
fn callback_logger_failure_event_writes_an_error_span() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.lifecycle(None)));
    logger
        .log_failure_event(&event("call-1"))
        .expect("failure event");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "litellm_request");
    assert!(matches!(
        spans[0].status,
        opentelemetry::trace::Status::Error { .. }
    ));
}

#[tokio::test]
async fn callback_logger_async_events_route_to_the_same_handlers() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.lifecycle(None)));
    logger
        .async_log_success_event(Arc::new(event("call-1")))
        .await
        .expect("async success event");
    logger
        .async_log_failure_event(Arc::new(event("call-2")))
        .await
        .expect("async failure event");
    assert_eq!(
        rig.names(),
        vec!["litellm_request", "raw_gen_ai_request", "litellm_request"]
    );
}

#[test]
fn callback_logger_success_event_dedupes_the_sync_and_async_fire() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.lifecycle(None)));
    let first = event("call-1");
    logger.log_success_event(&first).expect("sync success");
    logger.log_success_event(&first).expect("duplicate success");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
}

#[test]
fn callback_logger_failure_event_dedupes_the_sync_and_async_fire() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.lifecycle(None)));
    let first = event("call-1");
    logger.log_failure_event(&first).expect("sync failure");
    logger.log_failure_event(&first).expect("duplicate failure");
    assert_eq!(rig.names(), vec!["litellm_request"]);
}
