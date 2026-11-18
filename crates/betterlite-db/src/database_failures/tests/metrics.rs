#![cfg(test)]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tokio_util::task::TaskTracker;

use crate::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use crate::database_failures::metrics::{
    DbEventMetadata, DbMetricCall, DbMetricError, DbService, safe_db_event_metadata,
};
use crate::database_faults::ErrorType;

use super::support::{
    RecordingResolver, RecordingSink, SinkFault, capture_logs, database_error, drain, failures,
    metric_call, run_metric, span, successes,
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
    let output = run_metric(async { Ok(7) }, &call, &sink, &resolver, &tasks)
        .await
        .expect("the metric does not change the result");
    assert_eq!(output, 7);
    assert!(successes(&sink).is_empty());
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].service, DbService::Postgres);
    assert_eq!(recorded[0].call_type, "query_raw");
    assert_eq!(recorded[0].parent_otel_span, Some(span(1)));
    assert!(recorded[0].duration_seconds >= 0.0);
    assert!(recorded[0].start_time <= recorded[0].end_time);
    assert_eq!(
        recorded[0].event_metadata,
        Some(DbEventMetadata {
            table_name: "users".to_owned()
        })
    );
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
    run_metric(
        async { Ok::<_, DbLookupError>(1) },
        &present,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    let missing = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: None,
        second_positional: None,
    };
    run_metric(
        async { Ok::<_, DbLookupError>(2) },
        &missing,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].parent_otel_span, Some(span(2)));
    assert_eq!(recorded[1].parent_otel_span, None);
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
        async { Ok::<_, DbLookupError>(1) },
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
        async { Ok::<_, DbLookupError>(1) },
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
    let kwargs = Json::Object(IndexMap::from([(
        "parent_otel_span".to_owned(),
        Json::Text("inner".to_owned()),
    )]));
    let lowercase = DbMetricCall {
        function_name: "proxy_write",
        parent_otel_span: Some(span(4)),
        table_name: None,
        second_positional: Some(&kwargs),
    };
    run_metric(
        async { Ok::<_, DbLookupError>(1) },
        &lowercase,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    let uppercase = DbMetricCall {
        function_name: "PROXY_write",
        parent_otel_span: Some(span(4)),
        table_name: None,
        second_positional: Some(&kwargs),
    };
    run_metric(
        async { Ok::<_, DbLookupError>(2) },
        &uppercase,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].service, DbService::Postgres);
    assert_eq!(recorded[0].parent_otel_span, Some(span(4)));
    assert_eq!(recorded[1].service, DbService::BatchWriteToDb);
    assert_eq!(recorded[1].parent_otel_span, Some(span(3)));
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
    let present = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: None,
        table_name: None,
        second_positional: Some(&kwargs),
    };
    run_metric(
        async { Ok::<_, DbLookupError>(1) },
        &present,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    let absent = metric_call("PROXY_batch_write");
    run_metric(
        async { Ok::<_, DbLookupError>(2) },
        &absent,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    let boolean = Json::Bool(true);
    let not_a_dictionary = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: None,
        table_name: None,
        second_positional: Some(&boolean),
    };
    run_metric(
        async { Ok::<_, DbLookupError>(3) },
        &not_a_dictionary,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
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

#[tokio::test]
async fn proxy_span_uses_shared_resolver() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let expected = IndexMap::from([(
        "parent_otel_span".to_owned(),
        Json::Text("inner".to_owned()),
    )]);
    let kwargs = Json::Object(expected.clone());
    let resolver = RecordingResolver {
        span: Some(span(6)),
        seen: Mutex::new(Vec::new()),
    };
    let call = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(7)),
        table_name: None,
        second_positional: Some(&kwargs),
    };
    run_metric(
        async { Ok::<_, DbLookupError>(1) },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    drain(&tasks).await;
    let recorded = successes(&sink);
    assert_eq!(recorded[0].parent_otel_span, Some(span(6)));
    let seen = resolver
        .seen
        .lock()
        .expect("the recording resolver is not poisoned");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0], expected);
}

#[tokio::test(start_paused = true)]
async fn proxy_success_zero_duration() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver {
        span: Some(span(3)),
        seen: Mutex::new(Vec::new()),
    };
    let kwargs = Json::Object(IndexMap::from([(
        "parent_otel_span".to_owned(),
        Json::Text("inner".to_owned()),
    )]));
    let call = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: None,
        table_name: None,
        second_positional: Some(&kwargs),
    };
    let operation = async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        Ok::<_, DbLookupError>(1)
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
        async { Ok::<_, DbLookupError>(7) },
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

#[tokio::test]
async fn success_duration_matches_event_window() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let call = metric_call("query_raw");
    let operation = async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok::<_, DbLookupError>(1)
    };
    run_metric(operation, &call, &sink, &resolver, &tasks)
        .await
        .expect("the successful call returns");
    drain(&tasks).await;
    let recorded = successes(&sink);
    let window = recorded[0]
        .end_time
        .signed_duration_since(recorded[0].start_time)
        .to_std()
        .expect("the event window is positive");
    assert!(window > Duration::ZERO);
    assert_eq!(
        recorded[0].duration_seconds.to_bits(),
        window.as_secs_f64().to_bits()
    );
}

#[tokio::test]
async fn proxy_missing_span_drops_success() {
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    let resolver = RecordingResolver::default();
    let kwargs = Json::Object(IndexMap::from([(
        "parent_otel_span".to_owned(),
        Json::Text("inner".to_owned()),
    )]));
    let call = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(7)),
        table_name: None,
        second_positional: Some(&kwargs),
    };
    let output = run_metric(
        async { Ok::<_, DbLookupError>(7) },
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
    let error = run_metric(
        async {
            Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
                ErrorType::Data,
                "unique violation",
            )))
        },
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect_err("the failed write is rethrown");
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
#[case::deadline(ErrorType::Deadline)]
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
        tokio::time::sleep(Duration::from_millis(50)).await;
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Connect,
            "down",
        )))
    };
    let cancelled = tokio::time::timeout(
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
    let kwargs = Json::Object(IndexMap::from([(
        "parent_otel_span".to_owned(),
        Json::Text("inner".to_owned()),
    )]));
    let call = DbMetricCall {
        function_name: "PROXY_batch_write",
        parent_otel_span: Some(span(6)),
        table_name: Some(&table),
        second_positional: Some(&kwargs),
    };
    let operation = async {
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Data,
            "conflict",
        )))
    };
    let error = run_metric(operation, &call, &sink, &resolver, &tasks)
        .await
        .expect_err("the database error is rethrown");
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
    let secrets = Json::Object(IndexMap::from([
        ("api_key".to_owned(), Json::Text("sk-secret".to_owned())),
        (
            "request_body".to_owned(),
            Json::Object(IndexMap::from([(
                "messages".to_owned(),
                Json::Text("hello".to_owned()),
            )])),
        ),
    ]));
    let success = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: Some(&table),
        second_positional: Some(&secrets),
    };
    run_metric(
        async { Ok::<_, DbLookupError>(1) },
        &success,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the successful call returns");
    let failure = DbMetricCall {
        function_name: "query_raw",
        parent_otel_span: None,
        table_name: None,
        second_positional: Some(&secrets),
    };
    run_metric(
        async {
            Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
                ErrorType::Data,
                "conflict",
            )))
        },
        &failure,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect_err("the failed write is rethrown");
    drain(&tasks).await;
    let succeeded = successes(&sink);
    assert_eq!(
        succeeded[0].event_metadata,
        Some(DbEventMetadata {
            table_name: "users".to_owned()
        })
    );
    let serialized =
        serde_json::to_string(&succeeded[0].event_metadata).expect("the event metadata serializes");
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("sk-secret"));
    assert!(!serialized.contains("request_body"));
    let failed = failures(&sink);
    assert_eq!(failed[0].event.event_metadata, None);
}
