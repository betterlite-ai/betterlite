#![cfg(test)]

use std::future::ready;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use opentelemetry::trace::SpanContext;
use rstest::rstest;
use tokio::time::{sleep, timeout};
use tokio_util::task::TaskTracker;

use crate::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use crate::database_failures::metrics::{
    DbEventMetadata, DbMetricCall, DbMetricContext, DbMetricError, DbService, DbServiceEvent,
    SystemWallClock, WallClock, log_db_metrics, safe_db_event_metadata,
};
use crate::database_faults::ErrorType;

use super::support::{
    RecordingResolver, RecordingSink, RecordingWallClock, SinkFault, capture_logs, database_error,
    drain, failures, metric_call, run_metric, span, successes,
};

#[tokio::test]
async fn ordinary_success_scheduled() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let table = Json::Text("users".to_owned());
    let call = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: Some(span(1)),
        table_name: Some(&table),
        second_positional: None,
    };
    let output = run_metric(
        async { Ok::<i32, DbLookupError>(7) },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the metric does not change the result");
    assert_eq!(output, 7);
    assert!(successes(&sink).is_empty());
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded.len(), 1);
    assert_query_success(&recorded[0]);
    assert!(failures(&sink).is_empty());
    assert!(
        resolver
            .seen
            .lock()
            .expect("the recording resolver is not poisoned")
            .is_empty()
    );
}

#[test]
fn db_service_wire_value_postgres() {
    assert_eq!(DbService::Postgres.as_str(), "postgres");
    assert_eq!(DbService::BatchWriteToDb.as_str(), "batch_write_to_db");
}

#[tokio::test]
async fn exact_keyword_parent_span() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver {
        span: Some(span(9)),
        seen: Mutex::new(Vec::new()),
    };
    let present = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: Some(span(2)),
        table_name: None,
        second_positional: None,
    };
    let missing = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: None,
        second_positional: None,
    };
    run_named_call(&tasks, &sink, &resolver, &present).await;
    run_named_call(&tasks, &sink, &resolver, &missing).await;
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded.len(), 2);
    assert_keyword_spans(&recorded);
    assert!(
        resolver
            .seen
            .lock()
            .expect("the recording resolver is not poisoned")
            .is_empty()
    );
}

#[tokio::test]
async fn table_only_metadata() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let table = Json::Text("LiteLLM_SpendLogs".to_owned());
    let call = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: Some(&table),
        second_positional: None,
    };
    run_metric(
        async { Ok::<i32, DbLookupError>(1) },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(
        recorded[0].event_metadata,
        Some(DbEventMetadata {
            table_name: "LiteLLM_SpendLogs".to_owned()
        })
    );
}

#[tokio::test]
async fn empty_table_name_retained() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let table = Json::Text(String::new());
    let call = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: Some(&table),
        second_positional: None,
    };
    run_metric(
        async { Ok::<i32, DbLookupError>(1) },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(
        recorded[0].event_metadata,
        Some(DbEventMetadata {
            table_name: String::new()
        })
    );
}

#[rstest]
#[case::number(Json::Number(serde_json::Number::from(7_i32)))]
#[case::float(Json::Number(serde_json::Number::from_f64(7.5).expect("the number is finite")))]
#[case::boolean(Json::Bool(true))]
#[case::null(Json::Null)]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn nonstring_table_name_dropped(#[case] table: Json) {
    assert_eq!(safe_db_event_metadata(Some(&table)), None);
    assert_eq!(safe_db_event_metadata(None), None);
}

#[tokio::test]
async fn proxy_substring_case_sensitive() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver {
        span: Some(span(3)),
        seen: Mutex::new(Vec::new()),
    };
    let kwargs = parent_kwargs();
    let lowercase = named_proxy_call("proxy_write", span(4), &kwargs);
    let uppercase = named_proxy_call("PROXY_write", span(4), &kwargs);
    run_named_call(&tasks, &sink, &resolver, &lowercase).await;
    run_named_call(&tasks, &sink, &resolver, &uppercase).await;
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded.len(), 2);
    assert!(recorded.iter().any(|event| {
        event.service == DbService::Postgres && event.parent_otel_span == Some(span(4))
    }));
    assert!(recorded.iter().any(|event| {
        event.service == DbService::BatchWriteToDb && event.parent_otel_span == Some(span(3))
    }));
    assert_eq!(
        resolver
            .seen
            .lock()
            .expect("the recording resolver is not poisoned")
            .len(),
        1
    );
}

#[tokio::test]
async fn proxy_args_second_dictionary() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver {
        span: Some(span(5)),
        seen: Mutex::new(Vec::new()),
    };
    let kwargs = Json::Object(IndexMap::from([(
        "user_api_key".to_owned(),
        Json::Text("sk-key".to_owned()),
    )]));
    let boolean = Json::Bool(true);
    let present = positional_call(Some(&kwargs));
    let absent = positional_call(None);
    let not_a_dictionary = positional_call(Some(&boolean));
    run_named_call(&tasks, &sink, &resolver, &present).await;
    run_named_call(&tasks, &sink, &resolver, &absent).await;
    run_named_call(&tasks, &sink, &resolver, &not_a_dictionary).await;
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].service, DbService::BatchWriteToDb);
    assert_eq!(
        resolver
            .seen
            .lock()
            .expect("the recording resolver is not poisoned")
            .len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn proxy_success_zero_duration() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver {
        span: Some(span(3)),
        seen: Mutex::new(Vec::new()),
    };
    let kwargs = parent_kwargs();
    let call = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: None,
        table_name: None,
        second_positional: Some(&kwargs),
    };
    let operation = async {
        sleep(Duration::from_millis(250)).await;
        Ok::<i32, DbLookupError>(1)
    };
    run_metric(operation, &call, &sink, &resolver, &tasks)
        .await
        .expect("the successful call returns");
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded[0].duration_seconds.to_bits(), 0.0_f64.to_bits());
    assert!(recorded[0].start_time <= recorded[0].end_time);
}

#[tokio::test]
async fn success_hook_failure_logged() {
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    sink.refuse_success.store(true, Ordering::SeqCst);
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let output = run_metric(
        async { Ok::<i32, DbLookupError>(7) },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the metric failure does not fail the operation");
    assert_eq!(output, 7);
    drain(&tasks).await;
    assert!(logs.text().contains("the database success metric failed"));
}

#[test]
fn system_wall_clock_reads_real_time() {
    let now = SystemWallClock.now();
    assert!(now.timestamp() > 1_700_000_000);
}

#[tokio::test]
async fn success_duration_uses_wall_clock_window() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let start: DateTime<Utc> =
        DateTime::from_timestamp(1_700_000_000, 0).expect("the start time is valid");
    let end: DateTime<Utc> =
        DateTime::from_timestamp(1_700_000_002, 500_000_000).expect("the end time is valid");
    let clock = RecordingWallClock::new(vec![start, end]);
    let call = metric_call("query_raw");
    let output = log_db_metrics(
        async { Ok::<i32, DbLookupError>(1) },
        &call,
        DbMetricContext {
            sink: Arc::clone(&sink),
            resolver: &resolver,
            clock: &clock,
            tasks: &tasks,
        },
    )
    .await
    .expect("the successful call returns");
    assert_eq!(output, 1);
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded[0].start_time, start);
    assert_eq!(recorded[0].end_time, end);
    assert_eq!(recorded[0].duration_seconds.to_bits(), 2.5_f64.to_bits());
}

#[tokio::test]
async fn backward_clock_keeps_signed_duration() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let start: DateTime<Utc> =
        DateTime::from_timestamp(1_700_000_010, 0).expect("the start time is valid");
    let end: DateTime<Utc> =
        DateTime::from_timestamp(1_700_000_000, 0).expect("the end time is valid");
    let clock = RecordingWallClock::new(vec![start, end]);
    let call = metric_call("query_raw");
    let error = log_db_metrics(
        ready(failed_data_write("unique violation")),
        &call,
        DbMetricContext {
            sink: Arc::clone(&sink),
            resolver: &resolver,
            clock: &clock,
            tasks: &tasks,
        },
    )
    .await
    .expect_err("the failed write is rethrown");
    assert!(matches!(
        error,
        DbMetricError::Lookup(DbLookupError::Database(_))
    ));
    let recorded = failures(&sink);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].event.start_time, start);
    assert_eq!(recorded[0].event.end_time, end);
    assert_eq!(
        recorded[0].event.duration_seconds.to_bits(),
        (-10.0_f64).to_bits()
    );
}

#[tokio::test]
async fn proxy_missing_span_drops_success() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let kwargs = parent_kwargs();
    let call = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(7)),
        table_name: None,
        second_positional: Some(&kwargs),
    };
    let output = run_metric(
        async { Ok::<i32, DbLookupError>(7) },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the result survives a dropped metric");
    assert_eq!(output, 7);
    drain(&tasks).await;
    assert!(successes(&sink).is_empty());
    assert_eq!(
        resolver
            .seen
            .lock()
            .expect("the recording resolver is not poisoned")
            .len(),
        1
    );
}

#[tokio::test]
async fn db_data_error_is_failure_metric() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let table = Json::Text("LiteLLM_VerificationToken".to_owned());
    let call = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: Some(span(8)),
        table_name: Some(&table),
        second_positional: None,
    };
    let error = run_data_error(&tasks, &sink, &resolver, &call, "unique violation").await;
    let DbMetricError::Lookup(DbLookupError::Database(db_error)) = &error else {
        panic!("the data error is carried, got {error:?}");
    };
    assert_eq!(db_error.error_type, ErrorType::Data);
    let recorded = failures(&sink);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].event.service, DbService::Postgres);
    assert_eq!(recorded[0].event.call_type, "query_raw");
    assert_eq!(recorded[0].event.parent_otel_span, Some(span(8)));
    assert_eq!(&recorded[0].error, db_error);
    assert_eq!(
        recorded[0].event.event_metadata,
        Some(DbEventMetadata {
            table_name: "LiteLLM_VerificationToken".to_owned()
        })
    );
    assert!(successes(&sink).is_empty());
}

#[rstest]
#[case::connect(ErrorType::Connect)]
#[case::connect_timeout(ErrorType::ConnectTimeout)]
#[case::read_timeout(ErrorType::ReadTimeout)]
#[case::engine_connection(ErrorType::EngineConnection)]
#[case::prisma(ErrorType::Prisma)]
#[tokio::test]
async fn connect_and_timeout_error_metrics(#[case] kind: ErrorType) {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let error =
        run_metric(
            async move {
                Err::<i32, DbLookupError>(DbLookupError::Database(database_error(kind, "down")))
            },
            &call,
            &sink,
            &resolver,
            &tasks,
        )
        .await
        .expect_err("the database error is rethrown");
    assert!(matches!(
        error,
        DbMetricError::Lookup(DbLookupError::Database(_))
    ));
    let recorded = failures(&sink);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].error.error_type, kind);
}

#[rstest]
#[case::read(ErrorType::Read)]
#[case::deadline(ErrorType::Deadline)]
#[case::no_db_connection(ErrorType::NoDbConnection)]
#[tokio::test]
async fn excluded_error_no_metric(#[case] kind: ErrorType) {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let error = run_metric(
        async move {
            Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
                kind,
                "not a metric family",
            )))
        },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect_err("the excluded error is rethrown");
    let DbMetricError::Lookup(DbLookupError::Database(db_error)) = &error else {
        panic!("the excluded error is carried, got {error:?}");
    };
    assert_eq!(db_error.error_type, kind);
    assert!(failures(&sink).is_empty());
    assert!(successes(&sink).is_empty());
}

#[tokio::test]
async fn unrelated_error_no_metric() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let application = async {
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Other,
            "application bug",
        )))
    };
    let application_error = run_metric(application, &call, &sink, &resolver, &tasks)
        .await
        .expect_err("the application error is rethrown");
    assert!(matches!(
        application_error,
        DbMetricError::Lookup(DbLookupError::Database(_))
    ));
    let deadline = DbLookupDeadlineExceeded {
        lookup: "inner".to_owned(),
        deadline_seconds: 2.0,
        cause: None,
    };
    let timed_out = async move { Err::<i32, DbLookupError>(DbLookupError::Deadline(deadline)) };
    let deadline_error = run_metric(timed_out, &call, &sink, &resolver, &tasks)
        .await
        .expect_err("the deadline error is rethrown");
    assert!(matches!(
        deadline_error,
        DbMetricError::Lookup(DbLookupError::Deadline(_))
    ));
    drain(&tasks).await;
    assert!(failures(&sink).is_empty());
}

#[tokio::test(start_paused = true)]
async fn cancellation_no_failure_metric() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let operation = async {
        sleep(Duration::from_millis(50)).await;
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Connect,
            "down",
        )))
    };
    let cancelled = timeout(
        Duration::from_millis(1),
        run_metric(operation, &call, &sink, &resolver, &tasks),
    )
    .await;
    assert!(cancelled.is_err());
    drain(&tasks).await;
    assert!(failures(&sink).is_empty());
    assert!(successes(&sink).is_empty());
}

#[tokio::test]
async fn failure_hook_awaited_before_rethrow() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let operation = async {
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Connect,
            "down",
        )))
    };
    let error = run_metric(operation, &call, &sink, &resolver, &tasks)
        .await
        .expect_err("the database error is rethrown");
    let recorded = failures(&sink);
    assert_eq!(recorded.len(), 1);
    assert!(tasks.is_empty());
    assert!(matches!(
        error,
        DbMetricError::Lookup(DbLookupError::Database(_))
    ));
}

#[tokio::test]
async fn proxy_failure_still_db_service() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver {
        span: Some(span(5)),
        seen: Mutex::new(Vec::new()),
    };
    let table = Json::Text("users".to_owned());
    let kwargs = parent_kwargs();
    let call = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(6)),
        table_name: Some(&table),
        second_positional: Some(&kwargs),
    };
    let error = run_data_error(&tasks, &sink, &resolver, &call, "conflict").await;
    assert!(matches!(
        error,
        DbMetricError::Lookup(DbLookupError::Database(_))
    ));
    let recorded = failures(&sink);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].event.service, DbService::Postgres);
    assert_eq!(recorded[0].event.call_type, "PROXY_batch_write");
    assert_eq!(recorded[0].event.parent_otel_span, Some(span(6)));
    assert_eq!(
        recorded[0].event.event_metadata,
        Some(DbEventMetadata {
            table_name: "users".to_owned()
        })
    );
    assert!(
        resolver
            .seen
            .lock()
            .expect("the recording resolver is not poisoned")
            .is_empty()
    );
}

#[tokio::test]
async fn failure_hook_error_replaces_original() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    sink.refuse_failure.store(true, Ordering::SeqCst);
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let operation = async {
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Connect,
            "down",
        )))
    };
    let error = run_metric(operation, &call, &sink, &resolver, &tasks)
        .await
        .expect_err("the sink error replaces the original");
    assert!(matches!(error, DbMetricError::Metric(SinkFault)));
    assert_eq!(failures(&sink).len(), 1);
}

#[tokio::test]
async fn sensitive_metadata_absent() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let table = Json::Text("users".to_owned());
    let secrets = secret_kwargs();
    let success = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: Some(&table),
        second_positional: Some(&secrets),
    };
    run_named_call(&tasks, &sink, &resolver, &success).await;
    let failure = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: None,
        second_positional: Some(&secrets),
    };
    run_data_error(&tasks, &sink, &resolver, &failure, "conflict").await;
    drain(&tasks).await;
    let succeeded = successes(&sink);
    assert_eq!(
        succeeded[0].event_metadata,
        Some(DbEventMetadata {
            table_name: "users".to_owned()
        })
    );
    assert_no_secrets(succeeded[0].event_metadata.as_ref());
    let failed = failures(&sink);
    assert_eq!(failed[0].event.event_metadata, None);
}

fn parent_kwargs() -> Json {
    Json::Object(IndexMap::from([(
        "parent_otel_span".to_owned(),
        Json::Text("inner".to_owned()),
    )]))
}

fn secret_kwargs() -> Json {
    Json::Object(IndexMap::from([
        ("api_key".to_owned(), Json::Text("sk-secret".to_owned())),
        (
            "request_body".to_owned(),
            Json::Object(IndexMap::from([(
                "messages".to_owned(),
                Json::Text("hello".to_owned()),
            )])),
        ),
    ]))
}

fn named_proxy_call<'call>(
    name: &'call str,
    parent: SpanContext,
    kwargs: &'call Json,
) -> DbMetricCall<'call> {
    DbMetricCall {
        function_name: name,
        parent_otel_span: Some(parent),
        table_name: None,
        second_positional: Some(kwargs),
    }
}

fn positional_call(second: Option<&Json>) -> DbMetricCall<'_> {
    DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: None,
        table_name: None,
        second_positional: second,
    }
}

fn failed_data_write(message: &str) -> Result<i32, DbLookupError> {
    Err(DbLookupError::Database(database_error(
        ErrorType::Data,
        message,
    )))
}

async fn run_named_call(
    tasks: &TaskTracker,
    sink: &Arc<RecordingSink>,
    resolver: &RecordingResolver,
    call: &DbMetricCall<'_>,
) -> i32 {
    run_metric(
        async { Ok::<i32, DbLookupError>(1) },
        call,
        sink,
        resolver,
        tasks,
    )
    .await
    .expect("the successful call returns")
}

async fn run_data_error(
    tasks: &TaskTracker,
    sink: &Arc<RecordingSink>,
    resolver: &RecordingResolver,
    call: &DbMetricCall<'_>,
    message: &str,
) -> DbMetricError<SinkFault> {
    run_metric(
        ready(failed_data_write(message)),
        call,
        sink,
        resolver,
        tasks,
    )
    .await
    .expect_err("the failed write is rethrown")
}

fn assert_query_success(event: &DbServiceEvent) {
    assert_eq!(event.service, DbService::Postgres);
    assert_eq!(event.call_type, "query_raw");
    assert_eq!(event.parent_otel_span, Some(span(1)));
    assert!(event.duration_seconds >= 0.0);
    assert!(event.start_time <= event.end_time);
    assert_eq!(
        event.event_metadata,
        Some(DbEventMetadata {
            table_name: "users".to_owned()
        })
    );
}

fn assert_no_secrets(metadata: Option<&DbEventMetadata>) {
    let serialized = serde_json::to_string(&metadata).expect("the event metadata serializes");
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("sk-secret"));
    assert!(!serialized.contains("request_body"));
}

fn assert_keyword_spans(recorded: &[DbServiceEvent]) {
    assert!(
        recorded
            .iter()
            .any(|event| event.parent_otel_span == Some(span(2)))
    );
    assert!(
        recorded
            .iter()
            .any(|event| event.parent_otel_span.is_none())
    );
}
