use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::logging_failure::handlers::async_failure_handler;
use crate::logging_failure::special::special_failure_handlers;
use crate::logging_failure::{FAILED_REQUEST_REASON, FailureRecord, FailureReport};
use crate::logging_state::LoggingInit;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::{CorrelationContext, CorrelationSettings, SecretRedaction};
use crate::logging_state::types::CallbackPolicy;

use super::support::{
    CountingFailureCounter, FakeLogger, Harness, RecordingFlush, build, build_in, call, callbacks,
    exception, init, object, params, registry, report, run_async_handler, text,
};

type Behavior = fn() -> Pin<Box<dyn Future<Output = ()> + Send>>;

fn claim(state: &LoggingState, name: &str) -> bool {
    state.model_call_details.get(name) == Some(&Json::Bool(true))
}

fn metadata_init(metadata: Json) -> LoggingInit {
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("metadata", metadata)]);
    logging_init
}

fn test_async_request_calls_the_async_callback() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut logging_init = init();
        logging_init.litellm_params = params(&[("acompletion", Json::Bool(true))]);
        let mut state = build(logging_init);
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert_eq!(logger.calls(), ["async"]);
        assert!(claim(&state, "has_logged_async_failure"));
    })
}

fn test_a_failing_callback_increments_the_first_counter_only()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first").failing_async());
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let first = CountingFailureCounter::new();
        let second = CountingFailureCounter::new();
        let mut deps = harness.deps();
        let first_counter: &(dyn CallbackFailureCounter + Send + Sync) = &first;
        let second_counter: &(dyn CallbackFailureCounter + Send + Sync) = &second;
        deps.counters = vec![first_counter, second_counter];
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        let mut failure_call = call(&mut state, &mut record, &mut context);
        async_failure_handler(
            &mut failure_call,
            &report("provider exploded"),
            &callbacks(&registry),
            &deps,
        )
        .await;
        assert_eq!(first.count(), 1);
        assert_eq!(second.count(), 0);
        assert_eq!(logger.calls(), ["async"]);
    })
}

fn test_a_failing_callback_does_not_stop_the_next() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let first = Arc::new(FakeLogger::new("first").failing_async());
        let second = Arc::new(FakeLogger::new("second"));
        let registry = registry(&[Arc::clone(&first), Arc::clone(&second)]);
        let harness = Harness::new();
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert_eq!(first.calls(), ["async"]);
        assert_eq!(second.calls(), ["async"]);
    })
}

fn test_the_first_async_failure_is_logged_once() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert_eq!(logger.calls(), ["async"]);
        assert_eq!(harness.payloads.calls(), 1);
    })
}

fn test_stream_failures_are_logged_each_time() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut logging_init = init();
        logging_init.stream = Some(true);
        let mut state = build(logging_init);
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert_eq!(logger.calls(), ["async", "async"]);
        assert!(!claim(&state, "has_logged_async_failure"));
    })
}

fn test_the_context_is_restored() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut context = CorrelationContext {
            trace_id: "outer-trace".to_owned(),
            session_id: "outer-session".to_owned(),
        };
        let settings = CorrelationSettings {
            supports_correlation_logging: true,
            request_correlation_in_logs: true,
            secret_redaction: SecretRedaction::Enabled,
        };
        let mut logging_init = init();
        logging_init.litellm_trace_id = Some("inner-trace".to_owned());
        let mut state = build_in(logging_init, &mut context, settings);
        let mut record = FailureRecord::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert_eq!(context.trace_id, "outer-trace");
        assert_eq!(context.session_id, "outer-session");
    })
}

fn test_the_baseline_estimate_is_invalidated() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let harness = Harness::new();
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(
            &mut state,
            &mut record,
            &mut context,
            &registry(&[]),
            &harness,
        )
        .await;
        assert_eq!(
            harness.baseline.reasons.lock().unwrap().as_slice(),
            [FAILED_REQUEST_REASON]
        );
    })
}

fn test_the_post_call_batches_are_flushed() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let harness = Harness::new();
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(
            &mut state,
            &mut record,
            &mut context,
            &registry(&[]),
            &harness,
        )
        .await;
        assert_eq!(harness.flush.flushed(), 1);
    })
}

fn test_a_flush_failure_is_swallowed() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let mut harness = Harness::new();
        harness.flush = RecordingFlush::failing();
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert_eq!(harness.flush.flushed(), 1);
        assert_eq!(logger.calls(), ["async"]);
    })
}

fn a_missing_payload_skips_the_callbacks_but_still_flushes()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::without_payload();
        let mut state = build(init());
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert!(logger.calls().is_empty());
        assert_eq!(harness.flush.flushed(), 1);
    })
}

fn test_a_preparation_error_stops_before_the_flush() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut logging_init = init();
        logging_init.litellm_params = params(&[("metadata", text("not-a-map"))]);
        let mut state = build(logging_init);
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        let mut facts = exception("provider exploded");
        let mut headers = indexmap::IndexMap::new();
        headers.insert("x-request-id".to_owned(), "request-1".to_owned());
        facts.headers = Some(headers);
        let failure = FailureReport {
            exception: Arc::new(facts),
            traceback: String::new(),
            start_time: None,
            end_time: None,
        };
        let mut failure_call = call(&mut state, &mut record, &mut context);
        async_failure_handler(
            &mut failure_call,
            &failure,
            &callbacks(&registry),
            &harness.deps(),
        )
        .await;
        assert!(logger.calls().is_empty());
        assert_eq!(harness.flush.flushed(), 0);
        assert!(!claim(&state, "has_logged_async_failure"));
    })
}

fn test_no_log_requests_skip_the_callback_but_still_flush()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut logging_init = init();
        logging_init.litellm_params = params(&[("no-log", Json::Bool(true))]);
        let mut state = build(logging_init);
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert!(logger.calls().is_empty());
        assert_eq!(harness.flush.flushed(), 1);
    })
}

fn test_a_proxy_logger_still_runs_on_a_no_log_request() -> Pin<Box<dyn Future<Output = ()> + Send>>
{
    Box::pin(async {
        let ordinary = Arc::new(FakeLogger::new("first"));
        let proxy = Arc::new(FakeLogger::new("_PROXY_MaxParallelRequestsHandler_v3"));
        let registry = registry(&[Arc::clone(&ordinary), Arc::clone(&proxy)]);
        let harness = Harness::new();
        let mut logging_init = init();
        logging_init.litellm_params = params(&[("no-log", Json::Bool(true))]);
        let mut state = build(logging_init);
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert!(ordinary.calls().is_empty());
        assert_eq!(proxy.calls(), ["async"]);
    })
}

fn test_single_deployment_metadata_notifies_the_listener()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let listener = Arc::new(FakeLogger::new("router"));
        let mut harness = Harness::new();
        harness.listen(&listener);
        let state = build(metadata_init(object(&[
            (
                "model_group_size",
                Json::Number(serde_json::Number::from(1_i64)),
            ),
            ("model_group", text("gpt-4o")),
        ])));
        special_failure_handlers(&state, &report("provider exploded"), &harness.listeners).await;
        let calls = listener.special_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.as_deref(), Some("gpt-4o"));
        assert_eq!(calls[0].1.as_deref(), Some("provider exploded"));
    })
}

fn test_the_no_deployments_message_notifies_the_listener()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let listener = Arc::new(FakeLogger::new("router"));
        let mut harness = Harness::new();
        harness.listen(&listener);
        let state = build(init());
        special_failure_handlers(
            &state,
            &report("No deployments available for selected model, Try again"),
            &harness.listeners,
        )
        .await;
        assert_eq!(listener.special_calls().len(), 1);
    })
}

fn test_other_failures_are_not_special() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let listener = Arc::new(FakeLogger::new("router"));
        let mut harness = Harness::new();
        harness.listen(&listener);
        let state = build(metadata_init(object(&[(
            "model_group_size",
            Json::Number(serde_json::Number::from(2_i64)),
        )])));
        special_failure_handlers(&state, &report("provider exploded"), &harness.listeners).await;
        assert!(listener.special_calls().is_empty());
    })
}

fn a_non_integer_group_size_is_not_a_single_deployment() -> Pin<Box<dyn Future<Output = ()> + Send>>
{
    Box::pin(async {
        let listener = Arc::new(FakeLogger::new("router"));
        let mut harness = Harness::new();
        harness.listen(&listener);
        let state = build(metadata_init(object(&[("model_group_size", text("1"))])));
        special_failure_handlers(&state, &report("provider exploded"), &harness.listeners).await;
        assert!(listener.special_calls().is_empty());
    })
}

fn test_a_failing_listener_does_not_stop_the_next() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let first = Arc::new(FakeLogger::new("first").failing_special());
        let second = Arc::new(FakeLogger::new("second"));
        let mut harness = Harness::new();
        harness.listen(&first);
        harness.listen(&second);
        let state = build(metadata_init(object(&[(
            "model_group_size",
            Json::Number(serde_json::Number::from(1_i64)),
        )])));
        special_failure_handlers(&state, &report("provider exploded"), &harness.listeners).await;
        assert_eq!(first.special_calls().len(), 1);
        assert_eq!(second.special_calls().len(), 1);
    })
}

fn test_a_disabled_no_log_policy_still_logs() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let logger = Arc::new(FakeLogger::new("first"));
        let registry = registry(&[Arc::clone(&logger)]);
        let harness = Harness::new();
        let mut logging_init = init();
        logging_init.litellm_params = params(&[("no-log", Json::Bool(true))]);
        logging_init.callback_policy = CallbackPolicy {
            global_disable_no_log_param: true,
        };
        let mut state = build(logging_init);
        let mut record = FailureRecord::default();
        let mut context = CorrelationContext::default();
        run_async_handler(&mut state, &mut record, &mut context, &registry, &harness).await;
        assert_eq!(logger.calls(), ["async"]);
    })
}

#[rstest]
#[case::async_request_calls_the_async_callback(test_async_request_calls_the_async_callback)]
#[case::a_failing_callback_increments_the_first_counter_only(
    test_a_failing_callback_increments_the_first_counter_only
)]
#[case::a_failing_callback_does_not_stop_the_next(test_a_failing_callback_does_not_stop_the_next)]
#[case::the_first_async_failure_is_logged_once(test_the_first_async_failure_is_logged_once)]
#[case::stream_failures_are_logged_each_time(test_stream_failures_are_logged_each_time)]
#[case::the_context_is_restored(test_the_context_is_restored)]
#[case::the_baseline_estimate_is_invalidated(test_the_baseline_estimate_is_invalidated)]
#[case::the_post_call_batches_are_flushed(test_the_post_call_batches_are_flushed)]
#[case::a_flush_failure_is_swallowed(test_a_flush_failure_is_swallowed)]
#[case::a_missing_payload_skips_the_callbacks_but_still_flushes(
    a_missing_payload_skips_the_callbacks_but_still_flushes
)]
#[case::a_preparation_error_stops_before_the_flush(test_a_preparation_error_stops_before_the_flush)]
#[case::no_log_requests_skip_the_callback_but_still_flush(
    test_no_log_requests_skip_the_callback_but_still_flush
)]
#[case::a_proxy_logger_still_runs_on_a_no_log_request(
    test_a_proxy_logger_still_runs_on_a_no_log_request
)]
#[case::single_deployment_metadata_notifies_the_listener(
    test_single_deployment_metadata_notifies_the_listener
)]
#[case::the_no_deployments_message_notifies_the_listener(
    test_the_no_deployments_message_notifies_the_listener
)]
#[case::other_failures_are_not_special(test_other_failures_are_not_special)]
#[case::a_non_integer_group_size_is_not_a_single_deployment(
    a_non_integer_group_size_is_not_a_single_deployment
)]
#[case::a_failing_listener_does_not_stop_the_next(test_a_failing_listener_does_not_stop_the_next)]
#[case::a_disabled_no_log_policy_still_logs(test_a_disabled_no_log_policy_still_logs)]
#[tokio::test]
async fn logging_failure_behavior_tests_behavior(#[case] behavior: Behavior) {
    behavior().await;
}
