use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::dynamic_callbacks::{DynamicCallback, DynamicCallbackLists};
use crate::logging_failure::FailureRecord;
use crate::logging_failure::callbacks::FailureCallbacks;
use crate::logging_failure::handlers::dispatch_failure_handlers;
use crate::logging_failure::handlers::{SyncFailureJob, run_sync_failure};
use crate::logging_state::LoggingInit;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;
use crate::registry::CallbackRegistry;

use super::support::{
    FakeLogger, Harness, RecordingExecutor, build, call, callbacks, init, params, payload,
    registry, registry_with_channel, report, stamp,
};

type Behavior = fn() -> Pin<Box<dyn Future<Output = ()> + Send>>;

fn claim(state: &LoggingState, name: &str) -> bool {
    state.model_call_details.get(name) == Some(&Json::Bool(true))
}

fn async_init() -> LoggingInit {
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("acompletion", Json::Bool(true))]);
    logging_init
}

async fn dispatch_setup(
    logging_init: LoggingInit,
    registry: &CallbackRegistry,
    harness: &Harness,
    prefer_async_handlers: bool,
) -> (LoggingState, FailureRecord) {
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    let mut failure_call = call(&mut state, &mut record, &mut context);
    dispatch_failure_handlers(
        &mut failure_call,
        &report("provider exploded"),
        &callbacks(registry),
        &harness.deps(),
        prefer_async_handlers,
    )
    .await;
    (state, record)
}

fn sync_sdk_dispatches_the_sync_handler() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let (state, _record) = dispatch_setup(init(), &registry, &harness, false).await;
        assert_eq!(logger.calls(), ["sync"]);
        assert_eq!(harness.executor.jobs(), 0);
        assert!(claim(&state, "has_logged_sync_failure"));
        assert!(!claim(&state, "has_logged_async_failure"));
    })
}

fn async_sdk_dispatches_the_async_handler() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let (state, _record) = dispatch_setup(async_init(), &registry, &harness, false).await;
        assert_eq!(logger.calls(), ["async"]);
        assert_eq!(harness.executor.jobs(), 0);
        assert!(claim(&state, "has_logged_async_failure"));
    })
}

fn prefer_async_dispatches_async_then_the_queued_sync_handler()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry_with_channel(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let (state, _record) = dispatch_setup(init(), &registry, &harness, true).await;
        assert_eq!(logger.calls(), ["async", "sync"]);
        assert_eq!(harness.executor.jobs(), 1);
        assert!(claim(&state, "has_logged_async_failure"));
        assert!(claim(&state, "has_logged_sync_failure"));
    })
}

fn passthrough_dispatches_the_async_handler() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut logging_init = init();
        logging_init.call_type = "pass_through_endpoint".to_owned();
        let (state, _record) = dispatch_setup(logging_init, &registry, &harness, false).await;
        assert_eq!(logger.calls(), ["async"]);
        assert_eq!(harness.executor.jobs(), 0);
        assert!(claim(&state, "has_logged_async_failure"));
    })
}

fn a_rejected_job_does_not_take_the_sync_claim() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry_with_channel(&[Arc::clone(&logger)]);
        let mut harness = Harness::new();
        harness.executor = RecordingExecutor::rejecting();
        let (state, _record) = dispatch_setup(init(), &registry, &harness, true).await;
        assert_eq!(harness.executor.jobs(), 1);
        assert_eq!(logger.calls(), ["async"]);
        assert!(!claim(&state, "has_logged_sync_failure"));
    })
}

fn the_predicate_is_false_for_loggers_only() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        assert!(!callbacks(&registry).should_run_sync_failure_callbacks_for_async_calls());
    })
}

fn the_predicate_is_true_for_channel_sinks() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry_with_channel(&[Arc::clone(&logger)]);
        assert!(callbacks(&registry).should_run_sync_failure_callbacks_for_async_calls());
    })
}

fn the_predicate_is_true_for_unresolved_dynamic_names() -> Pin<Box<dyn Future<Output = ()> + Send>>
{
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let dynamic = DynamicCallbackLists {
            input: None,
            success: None,
            async_success: None,
            failure: Some(vec![DynamicCallback::Name("legacy_function".to_owned())]),
            async_failure: None,
        };
        let failure_callbacks = FailureCallbacks {
            registry: &registry,
            dynamic: Some(&dynamic),
        };
        assert!(failure_callbacks.should_run_sync_failure_callbacks_for_async_calls());
    })
}

fn a_queued_job_for_an_async_sdk_request_calls_nothing() -> Pin<Box<dyn Future<Output = ()> + Send>>
{
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut state = build(async_init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        let failure_call = call(&mut state, &mut record, &mut context);
        let job = SyncFailureJob::prepare(&failure_call, &callbacks(&registry), &harness.deps());
        run_sync_failure(job);
        assert!(logger.calls().is_empty());
    })
}

fn a_queued_job_without_a_payload_calls_nothing() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let harness = Harness::without_payload();
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        let failure_call = call(&mut state, &mut record, &mut context);
        let job = SyncFailureJob {
            litellm_params: failure_call.state.litellm_params.clone(),
            call_type: failure_call.state.call_type.clone(),
            stream: false,
            dynamic_params: Arc::new(Json::Null),
            loggers: vec![Arc::<FakeLogger>::clone(&logger)],
            payload: None,
            callbacks_allowed: true,
            excluded_fields: Vec::new(),
            stamp: stamp(),
        };
        run_sync_failure(job);
        assert!(logger.calls().is_empty());
        assert_eq!(harness.executor.jobs(), 0);
    })
}

fn a_queued_job_with_callbacks_not_allowed_calls_nothing()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        let failure_call = call(&mut state, &mut record, &mut context);
        let job = SyncFailureJob {
            litellm_params: failure_call.state.litellm_params.clone(),
            call_type: failure_call.state.call_type.clone(),
            stream: false,
            dynamic_params: Arc::new(Json::Null),
            loggers: vec![Arc::<FakeLogger>::clone(&logger)],
            payload: Some(Arc::new(payload())),
            callbacks_allowed: false,
            excluded_fields: Vec::new(),
            stamp: stamp(),
        };
        run_sync_failure(job);
        assert!(logger.calls().is_empty());
    })
}

#[rstest]
#[case::sync_sdk_dispatches_the_sync_handler(sync_sdk_dispatches_the_sync_handler)]
#[case::async_sdk_dispatches_the_async_handler(async_sdk_dispatches_the_async_handler)]
#[case::prefer_async_dispatches_async_then_the_queued_sync_handler(
    prefer_async_dispatches_async_then_the_queued_sync_handler
)]
#[case::passthrough_dispatches_the_async_handler(passthrough_dispatches_the_async_handler)]
#[case::a_rejected_job_does_not_take_the_sync_claim(a_rejected_job_does_not_take_the_sync_claim)]
#[case::the_predicate_is_false_for_loggers_only(the_predicate_is_false_for_loggers_only)]
#[case::the_predicate_is_true_for_channel_sinks(the_predicate_is_true_for_channel_sinks)]
#[case::the_predicate_is_true_for_unresolved_dynamic_names(
    the_predicate_is_true_for_unresolved_dynamic_names
)]
#[case::a_queued_job_for_an_async_sdk_request_calls_nothing(
    a_queued_job_for_an_async_sdk_request_calls_nothing
)]
#[case::a_queued_job_without_a_payload_calls_nothing(a_queued_job_without_a_payload_calls_nothing)]
#[case::a_queued_job_with_callbacks_not_allowed_calls_nothing(
    a_queued_job_with_callbacks_not_allowed_calls_nothing
)]
#[tokio::test]
async fn logging_failure_behavior_tests_behavior(#[case] behavior: Behavior) {
    behavior().await;
}
