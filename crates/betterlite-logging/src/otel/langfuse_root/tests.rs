#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_models::decisions::Json;
use futures_util::StreamExt as _;
use futures_util::stream;
use indexmap::IndexMap;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracer, SdkTracerProvider};

use super::{LangfuseContentOtel, LangfuseOtel, RootSpanSink, RootSpanSource};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::{GuardrailStatus, LoggingStatus, StatusFields};
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{ApiCallLogger as _, PreCallEvent};
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterSpec, Settings};
use crate::otel::content_projections::{OutputResponse, RelayedChunk, StreamOutputAssembler};
use crate::otel::emitter::span_emission::{DedupCache, SpanEmitter};
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use crate::otel::logger::v2::{OpenTelemetryV2, OpenTelemetryV2Parts, RequestTraceSource};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::exporters::{ExporterFactories, HttpClients};
use crate::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};

type Log = Arc<Mutex<Vec<String>>>;

#[derive(Debug)]
struct RecordingSink {
    recording: bool,
    log: Log,
}

impl RootSpanSink for RecordingSink {
    fn is_recording(&self) -> bool {
        self.recording
    }

    fn set_attribute(&self, key: &str, value: String) {
        self.log.lock().expect("log").push(format!("{key}={value}"));
    }
}

struct FixedRoots {
    sink: Option<Arc<dyn RootSpanSink>>,
}

impl RootSpanSource for FixedRoots {
    fn request_root_span(&self) -> Option<Arc<dyn RootSpanSink>> {
        self.sink.clone()
    }
}

// the wrapped pre-call reaches its trace source only after it parsed a real call, so the marker proves it ran
struct RecordingTrace {
    log: Log,
}

impl RequestTraceSource for RecordingTrace {
    fn trace_context(&self) -> RequestTraceContext {
        self.log.lock().expect("log").push("wrapped".to_owned());
        RequestTraceContext::default()
    }
}

#[derive(Debug)]
struct NoClient;

#[async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the langfuse root tests export in memory"))
    }
}

#[derive(Debug)]
struct NoScope;

impl RequestScopeSource for NoScope {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
    }
}

struct FakeAssembler {
    assembled: Option<OtelModelValue>,
}

impl StreamOutputAssembler for FakeAssembler {
    fn assemble_sse(&self, _frames: &[Vec<u8>]) -> Option<OtelModelValue> {
        self.assembled.clone()
    }

    fn assemble_chat(
        &self,
        _chunks: &[OtelModelValue],
        _messages: Option<&[OtelModelValue]>,
    ) -> Option<OtelModelValue> {
        self.assembled.clone()
    }
}

fn settings(exporters: Vec<ExporterSpec>) -> Settings {
    Settings {
        exporters,
        service_name: "betterlite".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn inner_logger(log: &Log) -> OpenTelemetryV2 {
    inner_logger_exporting(log, InMemorySpanExporter::default())
}

fn inner_logger_exporting(log: &Log, spans: InMemorySpanExporter) -> OpenTelemetryV2 {
    let config = settings(Vec::new());
    let tracer = SdkTracerProvider::builder()
        .with_simple_exporter(spans)
        .build()
        .tracer("litellm");
    let emitter = emitter_for(&tracer);
    let tenants = tenants_for(&config);
    OpenTelemetryV2::with_parts(OpenTelemetryV2Parts {
        name: "langfuse_custom".to_owned(),
        config,
        tracer,
        emitter,
        metrics: None,
        tenants,
        trace: Arc::new(RecordingTrace {
            log: Arc::clone(log),
        }),
    })
}

fn emitter_for(tracer: &SdkTracer) -> SpanEmitter<SdkTracer> {
    SpanEmitter {
        tracer: tracer.clone(),
        environment: Arc::new(BTreeMap::<String, String>::new()),
        mappers: Vec::new(),
        event_recorder: None,
        attribute_limit: None,
        emitted: DedupCache::default(),
    }
}

fn tenants_for(config: &Settings) -> TenantTracerCache {
    TenantTracerCache::new(
        config.clone(),
        Some("langfuse_custom".to_owned()),
        "litellm",
        TenantSources {
            factories: Arc::new(ExporterFactories::new(
                HttpClients {
                    traces: Arc::new(NoClient),
                    metrics: Arc::new(NoClient),
                    logs: Arc::new(NoClient),
                },
                BTreeMap::new(),
            )),
            scope_source: Arc::new(NoScope),
            allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
        },
    )
}

fn root(log: &Log, recording: bool) -> Arc<dyn RootSpanSink> {
    Arc::new(RecordingSink {
        recording,
        log: Arc::clone(log),
    })
}

fn details() -> Json {
    serde_json::from_str(
        r#"{
            "litellm_call_id": "call-1",
            "model": "gpt-4o",
            "litellm_params": {
                "metadata": {
                    "trace_name": "checkout",
                    "trace_user_id": "user-1",
                    "session_id": "session-1",
                    "tags": ["env:test", "beta"]
                }
            }
        }"#,
    )
    .expect("details")
}

fn data(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("request data")
}

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn entries(log: &Log) -> Vec<String> {
    log.lock().expect("log").clone()
}

fn render(chunk: &RelayedChunk) -> String {
    match chunk {
        RelayedChunk::Raw(frame) => str::from_utf8(frame).expect("raw chunk").to_owned(),
        RelayedChunk::Chat(item) | RelayedChunk::Completed(item) | RelayedChunk::Other(item) => {
            serde_json::to_string(item).expect("json")
        }
    }
}

fn key_dict() -> MetadataMap {
    MetadataMap::from([(
        "api_key".to_owned(),
        OtelModelValue::String("sk-test".to_owned()),
    )])
}

fn pre_call(logger: &LangfuseOtel) -> Result<(), PrePostError> {
    let details = details();
    logger.log_pre_api_call(&PreCallEvent {
        model: "gpt-4o",
        messages: &Json::Null,
        details: &details,
    })
}

const REPLY: &str = r#"{"choices": [{"message": {"role": "assistant", "content": "done"}}]}"#;
const REPLY_OUTPUT: &str = r#"[{"content":"done","role":"assistant"}]"#;
const INPUT: &str = r#"[{"content":"hello","role":"user"}]"#;
const REQUEST: &str = r#"{"messages": [{"role": "user", "content": "hello"}]}"#;

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

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("payload")
}

// a failed call with its own id, so the second close is not deduped against the first
fn failure_payload() -> StandardLoggingPayload {
    let mut payload = payload();
    payload.id = "chatcmpl-2".to_owned();
    payload.status = LoggingStatus::Failure;
    payload.status_fields = StatusFields {
        llm_api_status: LoggingStatus::Failure,
        guardrail_status: GuardrailStatus::NotRun,
    };
    payload
}

fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

#[test]
fn the_pre_call_stamps_the_trace_controls_before_the_wrapped_hook() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let logger = LangfuseOtel {
        inner: inner_logger(&log),
        roots: Arc::new(FixedRoots { sink: Some(sink) }),
    };
    pre_call(&logger).expect("pre call");
    assert_eq!(
        entries(&log),
        [
            "langfuse.trace.name=checkout",
            "user.id=user-1",
            "session.id=session-1",
            "langfuse.trace.tags=[\"env:test\",\"beta\"]",
            "wrapped",
        ]
    );
}

#[test]
fn a_missing_root_stamps_nothing_and_still_calls_the_wrapped_hook() {
    let log: Log = Arc::default();
    let logger = LangfuseOtel {
        inner: inner_logger(&log),
        roots: Arc::new(FixedRoots { sink: None }),
    };
    pre_call(&logger).expect("pre call");
    assert_eq!(entries(&log), ["wrapped"]);
}

#[test]
fn a_non_recording_root_stamps_nothing_and_still_calls_the_wrapped_hook() {
    let log: Log = Arc::default();
    let sink = root(&log, false);
    let logger = LangfuseOtel {
        inner: inner_logger(&log),
        roots: Arc::new(FixedRoots { sink: Some(sink) }),
    };
    pre_call(&logger).expect("pre call");
    assert_eq!(entries(&log), ["wrapped"]);
}

#[test]
fn the_pre_call_omits_the_controls_the_caller_did_not_send() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let logger = LangfuseOtel {
        inner: inner_logger(&log),
        roots: Arc::new(FixedRoots { sink: Some(sink) }),
    };
    let details: Json = serde_json::from_str(r#"{"litellm_call_id": "call-1", "model": "gpt-4o"}"#)
        .expect("details");
    logger
        .log_pre_api_call(&PreCallEvent {
            model: "gpt-4o",
            messages: &Json::Null,
            details: &details,
        })
        .expect("pre call");
    assert_eq!(entries(&log), ["wrapped"]);
}

#[test]
fn the_content_wrapper_forwards_the_pre_call_stamp() {
    let log: Log = Arc::default();
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots {
                sink: Some(root(&log, true)),
            }),
        },
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    let details = details();
    content
        .log_pre_api_call(&PreCallEvent {
            model: "gpt-4o",
            messages: &Json::Null,
            details: &details,
        })
        .expect("pre call");
    assert_eq!(
        entries(&log),
        [
            "langfuse.trace.name=checkout",
            "user.id=user-1",
            "session.id=session-1",
            "langfuse.trace.tags=[\"env:test\",\"beta\"]",
            "wrapped",
        ]
    );
}

#[test]
fn both_wrappers_report_the_wrapped_callback_name() {
    let log: Log = Arc::default();
    let otel = LangfuseOtel {
        inner: inner_logger(&log),
        roots: Arc::new(FixedRoots {
            sink: Some(root(&log, true)),
        }),
    };
    assert_eq!(otel.callback_name(), "langfuse_custom");
    let content = LangfuseContentOtel {
        inner: otel,
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    assert_eq!(content.callback_name(), "langfuse_custom");
}

#[tokio::test]
async fn the_success_and_failure_events_reach_the_wrapped_logger() {
    let log: Log = Arc::default();
    let spans = InMemorySpanExporter::default();
    let logger = LangfuseOtel {
        inner: inner_logger_exporting(&log, spans.clone()),
        roots: Arc::new(FixedRoots {
            sink: Some(root(&log, true)),
        }),
    };
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("success event");
    logger
        .async_log_failure_event(event(failure_payload()))
        .await
        .expect("failure event");
    assert_eq!(spans.get_finished_spans().expect("spans").len(), 2);
}

#[tokio::test]
async fn the_content_wrapper_forwards_the_success_and_failure_events() {
    let log: Log = Arc::default();
    let spans = InMemorySpanExporter::default();
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger_exporting(&log, spans.clone()),
            roots: Arc::new(FixedRoots {
                sink: Some(root(&log, true)),
            }),
        },
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    content
        .async_log_success_event(event(payload()))
        .await
        .expect("success event");
    content
        .async_log_failure_event(event(failure_payload()))
        .await
        .expect("failure event");
    assert_eq!(spans.get_finished_spans().expect("spans").len(), 2);
}

#[test]
fn the_success_hook_stamps_the_output_then_the_input() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    let response = value(REPLY);
    content.async_post_call_success_hook(
        &data(REQUEST),
        &key_dict(),
        &OutputResponse::ModelResponse(&response),
    );
    assert_eq!(
        entries(&log),
        [
            format!("langfuse.observation.output={REPLY_OUTPUT}"),
            format!("langfuse.observation.input={INPUT}"),
        ]
    );
}

#[test]
fn the_success_hook_stamps_nothing_when_the_output_does_not_render() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    let response = value(r#"{"type": "not-a-message"}"#);
    content.async_post_call_success_hook(
        &data(REQUEST),
        &key_dict(),
        &OutputResponse::Other(&response),
    );
    assert!(entries(&log).is_empty());
}

#[test]
fn the_success_hook_keeps_the_output_when_the_input_does_not_render() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    let response = value(REPLY);
    content.async_post_call_success_hook(
        &data("{}"),
        &key_dict(),
        &OutputResponse::ModelResponse(&response),
    );
    assert_eq!(
        entries(&log),
        [format!("langfuse.observation.output={REPLY_OUTPUT}")]
    );
}

#[test]
fn the_success_hook_stamps_nothing_on_a_non_recording_root() {
    let log: Log = Arc::default();
    let sink = root(&log, false);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    let response = value(REPLY);
    content.async_post_call_success_hook(
        &data(REQUEST),
        &key_dict(),
        &OutputResponse::ModelResponse(&response),
    );
    assert!(entries(&log).is_empty());
}

#[test]
fn the_success_hook_stamps_nothing_when_the_request_has_no_root() {
    let log: Log = Arc::default();
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: None }),
        },
        stream_assembler: Arc::new(FakeAssembler { assembled: None }),
    };
    let response = value(REPLY);
    content.async_post_call_success_hook(
        &data(REQUEST),
        &key_dict(),
        &OutputResponse::ModelResponse(&response),
    );
    assert!(entries(&log).is_empty());
}

#[tokio::test]
async fn the_streaming_hook_relays_every_chunk_in_order_and_stamps_once_after_the_last() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler {
            assembled: Some(value(REPLY)),
        }),
    };
    let chunks = [
        RelayedChunk::Chat(value(r#"{"id": "1"}"#)),
        RelayedChunk::Chat(value(r#"{"id": "2"}"#)),
    ];
    let request_data = data(REQUEST);
    let mut relayed = content.async_post_call_streaming_iterator_hook(
        &key_dict(),
        stream::iter(chunks),
        &request_data,
    );
    let mut seen = Vec::new();
    while let Some(chunk) = relayed.next().await {
        seen.push(render(&chunk));
    }
    assert_eq!(seen, [r#"{"id":"1"}"#, r#"{"id":"2"}"#]);
    assert_eq!(
        entries(&log),
        [
            format!("langfuse.observation.output={REPLY_OUTPUT}"),
            format!("langfuse.observation.input={INPUT}"),
        ]
    );
}

#[tokio::test]
async fn a_raw_stream_reaches_the_sse_assembler() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler {
            assembled: Some(value(REPLY)),
        }),
    };
    let chunks = [
        RelayedChunk::Raw(b"event: message_start\n".to_vec()),
        RelayedChunk::Raw(b"data: {}\n\n".to_vec()),
    ];
    let request_data = data("{}");
    let mut relayed = content.async_post_call_streaming_iterator_hook(
        &key_dict(),
        stream::iter(chunks),
        &request_data,
    );
    let mut seen = Vec::new();
    while let Some(chunk) = relayed.next().await {
        seen.push(render(&chunk));
    }
    assert_eq!(seen, ["event: message_start\n", "data: {}\n\n"]);
    assert_eq!(
        entries(&log),
        [format!("langfuse.observation.output={REPLY_OUTPUT}")]
    );
}

#[tokio::test]
async fn a_stream_dropped_before_its_end_stamps_nothing() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler {
            assembled: Some(value(REPLY)),
        }),
    };
    let chunks = [
        RelayedChunk::Chat(value(r#"{"id": "1"}"#)),
        RelayedChunk::Chat(value(r#"{"id": "2"}"#)),
    ];
    let request_data = data(REQUEST);
    {
        let mut relayed = content.async_post_call_streaming_iterator_hook(
            &key_dict(),
            stream::iter(chunks),
            &request_data,
        );
        assert!(relayed.next().await.is_some());
    }
    assert!(entries(&log).is_empty());
}

#[tokio::test]
async fn an_empty_stream_stamps_nothing() {
    let log: Log = Arc::default();
    let sink = root(&log, true);
    let content = LangfuseContentOtel {
        inner: LangfuseOtel {
            inner: inner_logger(&log),
            roots: Arc::new(FixedRoots { sink: Some(sink) }),
        },
        stream_assembler: Arc::new(FakeAssembler {
            assembled: Some(value(REPLY)),
        }),
    };
    let request_data = data("{}");
    let mut relayed = content.async_post_call_streaming_iterator_hook(
        &key_dict(),
        stream::iter(Vec::<RelayedChunk>::new()),
        &request_data,
    );
    assert!(relayed.next().await.is_none());
    assert!(entries(&log).is_empty());
}
