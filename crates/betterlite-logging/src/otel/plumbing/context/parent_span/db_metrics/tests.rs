#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::metrics::{
    DbMetricCall, DbMetricContext, DbMetricFuture, DbService, DbServiceEvent, DbServiceEventSink,
    DbServiceFailureEvent, SystemWallClock, log_db_metrics,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use tokio_util::task::TaskTracker;

use super::KwargsParentSpanResolver;

const PARENT_SPAN_KEY: &str = "litellm_parent_otel_span";
const METADATA_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const PARAMS_TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
const TOP_LEVEL_TRACE: &str = "11111111111111111111111111111111";

#[derive(Debug, thiserror::Error)]
#[error("the sink refused the event")]
struct SinkFault;

#[derive(Default)]
struct RecordingSink {
    successes: Mutex<Vec<DbServiceEvent>>,
}

impl DbServiceEventSink for RecordingSink {
    type Failure = SinkFault;

    fn success(&self, event: DbServiceEvent) -> DbMetricFuture<'_, SinkFault> {
        self.successes
            .lock()
            .expect("the recording sink is not poisoned")
            .push(event);
        Box::pin(async { Ok(()) })
    }

    fn failure(&self, _event: DbServiceFailureEvent) -> DbMetricFuture<'_, SinkFault> {
        Box::pin(async { Ok(()) })
    }
}

fn span(byte: u8) -> SpanContext {
    SpanContext::new(
        TraceId::from_bytes([byte; 16]),
        SpanId::from_bytes([byte; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    )
}

fn traceparent(value: &str) -> Json {
    Json::Text(format!("00-{value}-b7ad6b7169203331-01"))
}

fn spanned(value: &str) -> Json {
    Json::Object(IndexMap::from([(
        PARENT_SPAN_KEY.to_owned(),
        traceparent(value),
    )]))
}

fn proxy_kwargs(metadata: Option<&str>, params: Option<&str>, top_level: Option<&str>) -> Json {
    let mut kwargs = IndexMap::new();
    if let Some(trace) = metadata {
        kwargs.insert("metadata".to_owned(), spanned(trace));
    }
    if let Some(trace) = params {
        kwargs.insert(
            "litellm_params".to_owned(),
            Json::Object(IndexMap::from([("metadata".to_owned(), spanned(trace))])),
        );
    }
    if let Some(trace) = top_level {
        kwargs.insert(PARENT_SPAN_KEY.to_owned(), traceparent(trace));
    }
    Json::Object(kwargs)
}

fn proxy_call(kwargs: &Json) -> DbMetricCall<'_> {
    DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(7)),
        table_name: None,
        second_positional: Some(kwargs),
    }
}

fn resolved_traces(recorded: &[DbServiceEvent]) -> Vec<String> {
    let mut resolved: Vec<String> = recorded
        .iter()
        .map(|event| {
            event
                .parent_otel_span
                .as_ref()
                .expect("the shared helper resolves a parent")
                .trace_id()
                .to_string()
        })
        .collect();
    resolved.sort_unstable();
    resolved
}

fn sorted_traces() -> Vec<String> {
    let mut expected = vec![
        METADATA_TRACE.to_owned(),
        PARAMS_TRACE.to_owned(),
        TOP_LEVEL_TRACE.to_owned(),
    ];
    expected.sort_unstable();
    expected
}

async fn record_success(call: &DbMetricCall<'_>, sink: &Arc<RecordingSink>, tasks: &TaskTracker) {
    let resolver = KwargsParentSpanResolver;
    let clock = SystemWallClock;
    let context = DbMetricContext {
        sink: Arc::clone(sink),
        resolver: &resolver,
        clock: &clock,
        tasks,
    };
    log_db_metrics(async { Ok::<i32, DbLookupError>(1_i32) }, call, context)
        .await
        .expect("the successful call returns");
}

#[tokio::test]
async fn proxy_span_uses_shared_resolver() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let both = proxy_kwargs(
        Some(METADATA_TRACE),
        Some(PARAMS_TRACE),
        Some(TOP_LEVEL_TRACE),
    );
    let params_only = proxy_kwargs(None, Some(PARAMS_TRACE), Some(TOP_LEVEL_TRACE));
    let top_level_only = proxy_kwargs(None, None, Some(TOP_LEVEL_TRACE));
    record_success(&proxy_call(&both), &sink, &tasks).await;
    record_success(&proxy_call(&params_only), &sink, &tasks).await;
    record_success(&proxy_call(&top_level_only), &sink, &tasks).await;
    tasks.close();
    tasks.wait().await;
    let recorded = sink
        .successes
        .lock()
        .expect("the recording sink is not poisoned")
        .clone();
    assert_eq!(recorded.len(), 3);
    assert_eq!(resolved_traces(&recorded), sorted_traces());
    assert!(recorded.iter().all(|event| {
        event.parent_otel_span != Some(span(7))
            && event.service == DbService::BatchWriteToDb
            && event.call_type == "PROXY_batch_write"
            && event.event_metadata.is_none()
    }));
}
