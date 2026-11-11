#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::clickhouse::clickhouse_logging::context::LensAnalysis;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::phases::{CallbackList, CallbackPhase};
use tokio_util::task::TaskTracker;

use super::error::TraceLifecycleError;
use super::ports::ReceiverHandle as _;
use super::session::Started;
use super::spend_logger::{CALLBACK_NAME, ClickHouseSpendLoggers};
use super::start;

mod support;

use support::{
    BuildBehavior, FailingLoggerError, FailingLoggers, FakeFactory, FakeReceiver,
    FakeReceiverError, RecordingStorage, StartBehavior, assert_only_identity, capture_log, event,
    event_with_session, log_text, register_operator, setup, spend_loggers, spend_settings,
};

async fn start_tracing(
    factory: &FakeFactory,
    registry: &mut CallbackRegistry,
) -> Option<Started<FakeReceiver>> {
    let loggers = spend_loggers(spend_settings(10, 3));
    start(true, factory, &loggers, registry).await.unwrap()
}

fn started_loggers(registry: &CallbackRegistry, phase: CallbackPhase) -> Arc<dyn CallbackLogger> {
    registry
        .loggers(phase)
        .first()
        .cloned()
        .expect("the started lifecycle registers its spend logger")
}

#[tokio::test]
async fn disabled_never_builds() {
    let (factory, _storage, mut registry) = setup();
    let loggers = spend_loggers(spend_settings(10, 3));
    let started = start(false, &factory, &loggers, &mut registry)
        .await
        .unwrap();
    assert!(started.is_none());
    assert_eq!(factory.build_count(), 0);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
}

#[tokio::test]
async fn factory_missing_setting() {
    let (log, guard) = capture_log();
    let (factory, _storage, mut registry) = setup();
    let failing = factory.with_build(BuildBehavior::MissingSetting);
    let started = start_tracing(&failing, &mut registry).await;
    assert!(started.is_none());
    assert_eq!(failing.build_count(), 1);
    assert_eq!(failing.start_count(), 0);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
    drop(guard);
    let text = log_text(&log);
    assert!(text.contains("Agent tracing unavailable"));
    assert!(text.contains("CLICKHOUSE_URL"));
    assert!(!text.contains("Agent tracing enabled"));
}

#[tokio::test]
async fn schema_io_failure() {
    let (log, guard) = capture_log();
    let (factory, _storage, mut registry) = setup();
    let broken = factory.with_start(StartBehavior::Io);
    let started = start_tracing(&broken, &mut registry).await;
    assert!(started.is_none());
    assert_eq!(broken.start_count(), 1);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
    assert!(registry.loggers(CallbackPhase::Failure).is_empty());
    drop(guard);
    let text = log_text(&log);
    assert!(text.contains("Agent tracing unavailable"));
    assert!(text.contains("schema setup failed"));
    assert!(!text.contains("Agent tracing enabled"));
}

#[tokio::test]
async fn start_runtime_failure() {
    let (log, guard) = capture_log();
    let (factory, _storage, mut registry) = setup();
    let broken = factory.with_start(StartBehavior::Runtime);
    let started = start_tracing(&broken, &mut registry).await;
    assert!(started.is_none());
    assert_eq!(broken.start_count(), 1);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
    drop(guard);
    let text = log_text(&log);
    assert!(text.contains("Agent tracing unavailable"));
    assert!(text.contains("the ingest semaphore is closed"));
}

#[tokio::test]
async fn invalid_start_value() {
    let (log, guard) = capture_log();
    let (factory, _storage, mut registry) = setup();
    let broken = factory.with_start(StartBehavior::InvalidValue);
    let started = start_tracing(&broken, &mut registry).await;
    assert!(started.is_none());
    assert_eq!(broken.start_count(), 1);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
    drop(guard);
    let text = log_text(&log);
    assert!(text.contains("Agent tracing unavailable"));
    assert!(text.contains("the concurrency limit is negative"));
}

#[tokio::test]
async fn unexpected_error_propagates() {
    let (factory, _storage, mut registry) = setup();
    let broken = factory.with_build(BuildBehavior::Unexpected);
    let loggers = spend_loggers(spend_settings(10, 3));
    let error = start(true, &broken, &loggers, &mut registry)
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        TraceLifecycleError::Receiver {
            source: FakeReceiverError::Unexpected
        }
    ));
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
}

#[tokio::test]
async fn logger_build_error_propagates() {
    let (factory, _storage, mut registry) = setup();
    let error = start(true, &factory, &FailingLoggers, &mut registry)
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        TraceLifecycleError::SpendLogger {
            source: FailingLoggerError
        }
    ));
    assert_eq!(factory.start_count(), 1);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
    assert!(registry.loggers(CallbackPhase::Failure).is_empty());
}

#[tokio::test(start_paused = true)]
async fn shared_storage_identity() {
    let (factory, storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    assert_eq!(factory.build_count(), 1);
    assert_eq!(factory.start_count(), 1);
    assert!(Arc::ptr_eq(&started.receiver.storage(), &storage));
    let logger = started_loggers(&registry, CallbackPhase::Success);
    logger
        .async_log_success_event(event("completion", "success"))
        .await
        .unwrap();
    drop(logger);
    started.session.close(&mut registry).await;
    let insertions = storage.insertions();
    assert_eq!(insertions.len(), 1);
    assert_eq!(insertions[0].table, "spend_logs");
    assert_eq!(insertions[0].rows.len(), 1);
    assert_eq!(insertions[0].rows[0].request_id, "chatcmpl-1");
}

#[tokio::test(start_paused = true)]
async fn five_lists_same_instance() {
    let (factory, _storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    let success = registry.loggers(CallbackPhase::Success);
    let failure = registry.loggers(CallbackPhase::Failure);
    let stream = registry.loggers(CallbackPhase::Stream);
    assert_eq!(success.len(), 1);
    assert_eq!(failure.len(), 1);
    assert_eq!(stream.len(), 1);
    assert!(Arc::ptr_eq(&success[0], &failure[0]));
    assert!(Arc::ptr_eq(&success[0], &stream[0]));
    assert_eq!(success[0].callback_name(), CALLBACK_NAME);
    assert_only_identity(&registry, &[CallbackList::Callbacks], CALLBACK_NAME);
    assert_only_identity(&registry, &[CallbackList::Success], CALLBACK_NAME);
    assert_only_identity(&registry, &[CallbackList::Failure], CALLBACK_NAME);
    started.session.close(&mut registry).await;
}

#[tokio::test(start_paused = true)]
async fn full_callbacks_list_keeps_the_phase_lists() {
    let storage = Arc::new(RecordingStorage::default());
    let factory = FakeFactory::new(Arc::clone(&storage));
    let mut registry = CallbackRegistry::new(1);
    register_operator(&mut registry, &[CallbackList::Callbacks]);
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("a full callbacks list does not fail startup");
    assert_only_identity(&registry, &[CallbackList::Callbacks], "clickhouse");
    assert_only_identity(&registry, &[CallbackList::Success], CALLBACK_NAME);
    assert_only_identity(&registry, &[CallbackList::Failure], CALLBACK_NAME);
    let success = registry.loggers(CallbackPhase::Success);
    let failure = registry.loggers(CallbackPhase::Failure);
    assert_eq!(success.len(), 2);
    assert_eq!(failure.len(), 2);
    let spend = success
        .iter()
        .find(|logger| logger.callback_name() == CALLBACK_NAME)
        .cloned()
        .expect("the spend logger answers the success phase");
    assert!(
        failure
            .iter()
            .any(|logger| logger.callback_name() == CALLBACK_NAME)
    );
    spend
        .async_log_success_event(event("completion", "success"))
        .await
        .unwrap();
    started.session.close(&mut registry).await;
    assert_eq!(storage.insertions().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn full_list_keeps_prior_entries() {
    let storage = Arc::new(RecordingStorage::default());
    let factory = FakeFactory::new(Arc::clone(&storage));
    let mut registry = CallbackRegistry::new(1);
    register_operator(&mut registry, &[CallbackList::Success]);
    let (log, guard) = capture_log();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("a full list does not fail startup");
    drop(guard);
    assert!(log_text(&log).contains("at capacity"));
    assert_only_identity(&registry, &[CallbackList::Success], "clickhouse");
    assert_only_identity(&registry, &[CallbackList::Failure], CALLBACK_NAME);
    assert_eq!(registry.list_identities(CallbackList::Callbacks).len(), 1);
    started.session.close(&mut registry).await;
    assert_only_identity(&registry, &[CallbackList::Success], "clickhouse");
    assert_eq!(
        registry.loggers(CallbackPhase::Success)[0].callback_name(),
        "clickhouse"
    );
}

#[tokio::test(start_paused = true)]
async fn failure_event_is_logged() {
    let (factory, storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    let logger = started_loggers(&registry, CallbackPhase::Failure);
    logger
        .async_log_failure_event(event("acompletion", "failure"))
        .await
        .unwrap();
    drop(logger);
    started.session.close(&mut registry).await;
    let insertions = storage.insertions();
    assert_eq!(insertions.len(), 1);
    assert_eq!(insertions[0].rows.len(), 1);
    assert_eq!(insertions[0].rows[0].status, "failure");
    assert_eq!(insertions[0].rows[0].call_type, "acompletion");
    assert_eq!(insertions[0].rows[0].request_id, "chatcmpl-1");
}

#[tokio::test(start_paused = true)]
async fn session_id_falls_back_to_the_payload_trace_id() {
    let (factory, storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    let logger = started_loggers(&registry, CallbackPhase::Success);
    logger
        .async_log_success_event(event_with_session("", "t1", "s2"))
        .await
        .unwrap();
    drop(logger);
    started.session.close(&mut registry).await;
    let insertions = storage.insertions();
    assert_eq!(insertions.len(), 1);
    assert_eq!(insertions[0].rows[0].session_id, "t1");
}

#[tokio::test(start_paused = true)]
async fn trace_ingest_event_is_not_logged() {
    let (factory, storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    let logger = started_loggers(&registry, CallbackPhase::Success);
    logger
        .async_log_success_event(event("/v1/traces/span", "success"))
        .await
        .unwrap();
    drop(logger);
    started.session.close(&mut registry).await;
    assert!(storage.insertions().is_empty());
}

#[tokio::test(start_paused = true)]
async fn unrelated_logger_survives() {
    let (factory, _storage, mut registry) = setup();
    register_operator(
        &mut registry,
        &[
            CallbackList::Callbacks,
            CallbackList::Success,
            CallbackList::Failure,
        ],
    );
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    started.session.close(&mut registry).await;
    assert_only_identity(
        &registry,
        &[
            CallbackList::Callbacks,
            CallbackList::Success,
            CallbackList::Failure,
        ],
        "clickhouse",
    );
    let loggers = registry.loggers(CallbackPhase::Success);
    assert_eq!(loggers.len(), 1);
    assert_eq!(loggers[0].callback_name(), "clickhouse");
}

#[tokio::test(start_paused = true)]
async fn unregister_before_flush() {
    let (factory, storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    let logger = started_loggers(&registry, CallbackPhase::Failure);
    storage.watch_strong_count(&logger);
    logger
        .async_log_success_event(event("completion", "success"))
        .await
        .unwrap();
    drop(logger);
    started.session.close(&mut registry).await;
    assert_eq!(storage.probe_counts(), vec![1]);
    assert_eq!(storage.insertions().len(), 1);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
    assert!(registry.loggers(CallbackPhase::Failure).is_empty());
}

#[tokio::test(start_paused = true)]
async fn exceptional_exit_flushes() {
    let (factory, storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    let logger = started_loggers(&registry, CallbackPhase::Failure);
    storage.fail_inserts(1);
    logger
        .async_log_failure_event(event("completion", "failure"))
        .await
        .unwrap();
    drop(logger);
    started.session.close(&mut registry).await;
    assert_eq!(storage.attempts(), 2);
    assert_eq!(storage.insertions().len(), 1);
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
}

#[tokio::test]
async fn enabled_is_logged_after_registration() {
    let (log, guard) = capture_log();
    let (factory, _storage, mut registry) = setup();
    let started = start_tracing(&factory, &mut registry)
        .await
        .expect("the lifecycle starts");
    drop(guard);
    assert!(log_text(&log).contains("Agent tracing enabled (store=clickhouse)"));
    started.session.close(&mut registry).await;
}

#[tokio::test]
async fn disabled_logs_nothing() {
    let (log, guard) = capture_log();
    let (factory, _storage, mut registry) = setup();
    let loggers = spend_loggers(spend_settings(10, 3));
    let started = start(false, &factory, &loggers, &mut registry)
        .await
        .unwrap();
    assert!(started.is_none());
    drop(guard);
    assert!(!log_text(&log).contains("Agent tracing enabled"));
}

#[tokio::test(start_paused = true)]
async fn lens_scope_marks_the_row() {
    let (factory, storage, mut registry) = setup();
    let lens = LensAnalysis::default();
    let loggers = ClickHouseSpendLoggers {
        settings: spend_settings(10, 3),
        tasks: TaskTracker::new(),
        lens: lens.clone(),
    };
    let started = start(true, &factory, &loggers, &mut registry)
        .await
        .expect("the lifecycle starts")
        .expect("the lifecycle keeps its receiver");
    let logger = started_loggers(&registry, CallbackPhase::Success);
    let scope = lens.lens_analysis();
    logger
        .async_log_success_event(event("completion", "success"))
        .await
        .unwrap();
    drop(scope);
    drop(logger);
    started.session.close(&mut registry).await;
    let insertions = storage.insertions();
    assert_eq!(insertions.len(), 1);
    assert!(
        insertions[0].rows[0]
            .metadata
            .contains(r#""litellm_lens_internal":true"#)
    );
}
