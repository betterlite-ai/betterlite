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

async fn record_success(call: &DbMetricCall<'_>, sink: &Arc<RecordingSink>, tasks: &TaskTracker) {
    let resolver = KwargsParentSpanResolver;
    let clock = SystemWallClock;
    let context = DbMetricContext {
        sink: Arc::clone(sink),
        resolver: &resolver,
        clock: &clock,
        tasks,
    };
    log_db_metrics(async { Ok::<i32, DbLookupError>(1) }, call, context)
        .await
        .expect("the successful call returns");
}

#[tokio::test]
async fn proxy_span_uses_shared_resolver() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let metadata_trace = "4bf92f3577b34da6a3ce929d0e0e4736";
    let params_trace = "0af7651916cd43dd8448eb211c80319c";
    let top_level_trace = "11111111111111111111111111111111";
    let both = Json::Object(IndexMap::from([
        ("metadata".to_owned(), spanned(metadata_trace)),
        (
            "litellm_params".to_owned(),
            Json::Object(IndexMap::from([(
                "metadata".to_owned(),
                spanned(params_trace),
            )])),
        ),
        (PARENT_SPAN_KEY.to_owned(), traceparent(top_level_trace)),
    ]));
    let params_only = Json::Object(IndexMap::from([
        (
            "litellm_params".to_owned(),
            Json::Object(IndexMap::from([(
                "metadata".to_owned(),
                spanned(params_trace),
            )])),
        ),
        (PARENT_SPAN_KEY.to_owned(), traceparent(top_level_trace)),
    ]));
    let top_level_only = Json::Object(IndexMap::from([(
        PARENT_SPAN_KEY.to_owned(),
        traceparent(top_level_trace),
    )]));
    let from_metadata = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(7)),
        table_name: None,
        second_positional: Some(&both),
    };
    let from_params = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(7)),
        table_name: None,
        second_positional: Some(&params_only),
    };
    let from_top_level = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(7)),
        table_name: None,
        second_positional: Some(&top_level_only),
    };
    record_success(&from_metadata, &sink, &tasks).await;
    record_success(&from_params, &sink, &tasks).await;
    record_success(&from_top_level, &sink, &tasks).await;
    tasks.close();
    tasks.wait().await;
    let recorded = sink
        .successes
        .lock()
        .expect("the recording sink is not poisoned")
        .clone();
    assert_eq!(recorded.len(), 3);
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
    let mut expected = vec![
        metadata_trace.to_owned(),
        params_trace.to_owned(),
        top_level_trace.to_owned(),
    ];
    resolved.sort_unstable();
    expected.sort_unstable();
    assert_eq!(resolved, expected);
    assert!(
        recorded
            .iter()
            .all(|event| event.parent_otel_span != Some(span(7)))
    );
    assert!(
        recorded
            .iter()
            .all(|event| event.service == DbService::BatchWriteToDb)
    );
    assert!(
        recorded
            .iter()
            .all(|event| event.call_type == "PROXY_batch_write")
    );
    assert!(recorded.iter().all(|event| event.event_metadata.is_none()));
}
