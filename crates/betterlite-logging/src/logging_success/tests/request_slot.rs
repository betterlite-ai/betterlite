#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use indexmap::IndexMap;
use opentelemetry::Context;
use opentelemetry::Value;
use opentelemetry::trace::{
    Span as _, SpanContext, TraceContextExt as _, Tracer as _, TracerProvider as _,
};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tokio::task::JoinSet;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::langfuse_observations::attributes::keys::TRACE_NAME;
use crate::logging_state::LoggingState;
use crate::logging_success::handler::{async_success_handler, success_handler};
use crate::otel::langfuse_root::{RootSpanSink, RootSpanSource as _};
use crate::otel::logger::v2::RequestTraceSource as _;
use crate::otel::logger_runtime::tests::support::provider_exporting;
use crate::otel::plumbing::context::context_slots::AnchoredSpan;
use crate::otel::proxy_runtime::slot::{RequestSlot, anchor_request_root, request_slot_scope};
use crate::otel::proxy_runtime::sources::ProxyTraceSources;
use crate::registry::Registration;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;

use super::support::{Fixture, chat_result, correlation, request, state};

const HANDLER_NAME: &str = "slot_reader";
const SERVER_SPAN: &str = "SERVER";

#[derive(Default)]
struct Observation {
    root: Option<SpanContext>,
    recording: bool,
    stamped: bool,
}

struct SlotReadingLogger {
    sources: Arc<ProxyTraceSources>,
    seen: Arc<Mutex<Observation>>,
}

impl SlotReadingLogger {
    fn observe(&self) {
        let mut seen = self.seen.lock().unwrap();
        seen.root = self.sources.trace_context().root.map(|root| root.context);
        let root: Option<Arc<dyn RootSpanSink>> = self.sources.request_root_span();
        seen.recording = root.as_ref().is_some_and(|sink| sink.is_recording());
        if let Some(sink) = root {
            sink.set_attribute(TRACE_NAME, SERVER_SPAN.to_owned());
            seen.stamped = true;
        }
    }
}

#[async_trait]
impl CallbackLogger for SlotReadingLogger {
    fn callback_name(&self) -> &str {
        HANDLER_NAME
    }

    fn log_success_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.observe();
        Ok(())
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.observe();
        Ok(())
    }
}

fn add_logger(fixture: &mut Fixture, logger: &Arc<SlotReadingLogger>) {
    let identity = CallbackIdentity {
        class_name: HANDLER_NAME.to_owned(),
        attributes: IndexMap::new(),
    };
    let erased = Arc::clone(logger);
    let sink = CallbackSink::Logger(erased);
    let registration = fixture.register(identity, sink, CallbackList::Callbacks);
    assert_ne!(registration, Registration::SkippedCapacity);
}

struct AnchoredRequest {
    exporter: InMemorySpanExporter,
    // the provider outlives the span: its shutdown resets the in-memory exporter
    provider: SdkTracerProvider,
    sources: Arc<ProxyTraceSources>,
    seen: Arc<Mutex<Observation>>,
    slot: Arc<RequestSlot>,
    request_state: LoggingState,
    root_context: SpanContext,
}

async fn anchored_request() -> AnchoredRequest {
    let exporter = InMemorySpanExporter::default();
    let provider = provider_exporting(&exporter);
    let sources = Arc::new(ProxyTraceSources::new(&BTreeMap::<String, String>::new()));
    let seen = Arc::new(Mutex::new(Observation::default()));
    let (slot, (request_state, root_context)) = request_slot_scope(async {
        let span = provider.tracer("request_slot_test").start(SERVER_SPAN);
        let context = span.span_context().clone();
        anchor_request_root(
            AnchoredSpan {
                context: context.clone(),
                http_route: None,
            },
            Context::current_with_span(span),
        );
        (state(Some(false)), context)
    })
    .await;
    AnchoredRequest {
        exporter,
        provider,
        sources,
        seen,
        slot,
        request_state,
        root_context,
    }
}

fn assert_handler_kept_the_root(seen: &Mutex<Observation>, root_context: &SpanContext) {
    let observed = seen.lock().unwrap();
    let root = observed.root.clone();
    let recording = observed.recording;
    let stamped = observed.stamped;
    drop(observed);
    assert_eq!(root, Some(root_context.clone()));
    assert!(recording, "the handler reaches the live root span");
    assert!(stamped, "the handler stamps the live root span");
}

fn assert_root_exported(exporter: &InMemorySpanExporter, root_context: &SpanContext) {
    let finished = exporter
        .get_finished_spans()
        .expect("the root span was exported");
    assert_eq!(finished.len(), 1);
    assert_eq!(&finished[0].span_context, root_context);
    assert!(finished[0].attributes.iter().any(|attribute| {
        attribute.key.as_str() == TRACE_NAME && attribute.value == Value::from(SERVER_SPAN)
    }));
}

#[tokio::test]
async fn a_handler_spawned_off_the_request_keeps_the_parent_span() {
    let AnchoredRequest {
        exporter,
        provider: _provider,
        sources,
        seen,
        slot,
        mut request_state,
        root_context,
    } = anchored_request().await;
    let logger = Arc::new(SlotReadingLogger {
        sources: Arc::clone(&sources),
        seen: Arc::clone(&seen),
    });
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let mut fixture = Fixture::new();
        add_logger(&mut fixture, &logger);
        let handler = fixture.handler();
        let mut correlation = correlation();
        async_success_handler(
            &handler,
            &mut request_state,
            &mut correlation,
            &request(chat_result()),
        )
        .await;
    });
    tasks.join_all().await;
    assert_handler_kept_the_root(&seen, &root_context);
    drop(slot);
    assert_root_exported(&exporter, &root_context);
}

#[tokio::test]
async fn a_sync_handler_on_a_blocking_task_keeps_the_parent_span() {
    let AnchoredRequest {
        exporter,
        provider: _provider,
        sources,
        seen,
        slot,
        mut request_state,
        root_context,
    } = anchored_request().await;
    let logger = Arc::new(SlotReadingLogger {
        sources: Arc::clone(&sources),
        seen: Arc::clone(&seen),
    });
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || {
        let mut fixture = Fixture::new();
        add_logger(&mut fixture, &logger);
        let handler = fixture.handler();
        let mut correlation = correlation();
        success_handler(
            &handler,
            &mut request_state,
            &mut correlation,
            &request(chat_result()),
        );
    });
    tasks.join_all().await;
    assert_handler_kept_the_root(&seen, &root_context);
    drop(slot);
    assert_root_exported(&exporter, &root_context);
}
