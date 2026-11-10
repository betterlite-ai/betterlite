use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::worker::{Admission, Callback as _, LoggingWorker, Rejection};
use betterlite_models::decisions::Json;

use super::support::{
    Fixture, base_hit, cached, hit_kwargs, logging_state, params, recorded_state, success_request,
    text, worker_settings,
};
use crate::caching::hit_callbacks::context::CacheHitError;
use crate::caching::hit_callbacks::job::{
    CacheHitSuccessJob, log_cache_hit_on_callbacks, run_cache_hit_success,
};

#[test]
fn a_cache_hit_job_names_the_cache_hit_success_event() {
    let fixture = Fixture::new(false);
    let job = CacheHitSuccessJob {
        handler: fixture.owned_handler(),
        state: logging_state(None),
        correlation: CorrelationContext::default(),
        request: success_request(SuccessResult::Chat(cached())),
    };
    assert_eq!(job.name(), "cache_hit_success");
}

#[tokio::test]
async fn a_cache_hit_is_queued_and_runs_the_async_handler() {
    let fixture = Fixture::new(false);
    let cached_result = cached();
    let before = cached_result.clone();
    let kwargs = hit_kwargs();
    let hit = base_hit(&kwargs, &cached_result);
    let worker = LoggingWorker::start(worker_settings(8, 2)).expect("the worker starts");
    let admission = log_cache_hit_on_callbacks(&worker, &hit, fixture.callbacks(&cached_result))
        .expect("the cache hit is recorded");
    assert_eq!(admission, Admission::Accepted);
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "async"]);
    assert_eq!(cached_result, before);
}

#[tokio::test]
async fn a_failing_async_callback_is_counted_and_leaves_the_result_alone() {
    let fixture = Fixture::new(true);
    let cached_result = cached();
    let before = cached_result.clone();
    let kwargs = hit_kwargs();
    let hit = base_hit(&kwargs, &cached_result);
    let worker = LoggingWorker::start(worker_settings(8, 2)).expect("the worker starts");
    let admission = log_cache_hit_on_callbacks(&worker, &hit, fixture.callbacks(&cached_result))
        .expect("the cache hit is recorded");
    assert_eq!(admission, Admission::Accepted);
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(fixture.counter.calls(), vec!["langfuse".to_owned()]);
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "async"]);
    assert_eq!(cached_result, before);
}

#[tokio::test]
async fn a_full_queue_rejects_the_cache_hit_callbacks() {
    let fixture = Fixture::new(false);
    let cached_result = cached();
    let before = cached_result.clone();
    let kwargs = hit_kwargs();
    let hit = base_hit(&kwargs, &cached_result);
    let worker = LoggingWorker::start(worker_settings(1, 1)).expect("the worker starts");
    let first = log_cache_hit_on_callbacks(&worker, &hit, fixture.callbacks(&cached_result))
        .expect("the cache hit is recorded");
    assert_eq!(first, Admission::Accepted);
    let second = log_cache_hit_on_callbacks(&worker, &hit, fixture.callbacks(&cached_result))
        .expect("the cache hit is recorded");
    assert_eq!(second, Admission::Rejected(Rejection::QueueFull));
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert_eq!(worker.stats().rejected_full, 1);
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "async"]);
    assert_eq!(cached_result, before);
}

#[tokio::test]
async fn a_refused_value_returns_a_typed_error_without_queueing() {
    let fixture = Fixture::new(false);
    let cached_result = cached();
    let kwargs = params(&[("stream", text("yes"))]);
    let hit = base_hit(&kwargs, &cached_result);
    let worker = LoggingWorker::start(worker_settings(8, 2)).expect("the worker starts");
    let error = log_cache_hit_on_callbacks(&worker, &hit, fixture.callbacks(&cached_result))
        .expect_err("the value is refused");
    assert_eq!(error, CacheHitError::FieldType { field: "stream" });
    assert_eq!(worker.stats().accepted, 0);
    assert!(fixture.calls.events().is_empty());
}

#[tokio::test]
async fn the_runner_awaits_the_async_handler_before_the_sync_callbacks() {
    let fixture = Fixture::new(false);
    let mut state = recorded_state(&cached());
    state.dynamic_callbacks.success = Some(vec!["legacy_hook".to_owned()]);
    let owned = fixture.owned_handler();
    let mut correlation = CorrelationContext::default();
    // a null result makes the payload build at event time, so a sync-first run would record build-after-sync
    let request = success_request(SuccessResult::Other(Json::Null));
    run_cache_hit_success(&owned, &mut state, &mut correlation, &request).await;
    assert_eq!(
        state.model_call_details.get("has_logged_async_success"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        state.model_call_details.get("has_logged_sync_success"),
        Some(&Json::Bool(true))
    );
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "async"]);
}
