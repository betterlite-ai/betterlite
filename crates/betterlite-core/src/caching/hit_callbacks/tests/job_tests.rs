use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::worker::{Admission, LoggingWorker, Rejection};
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{Fixture, borrowed_handler, cached, state, success_request, worker_settings};
use crate::caching::hit_callbacks::job::{LoggingSyncDispatch, SyncSuccessDispatch as _};

#[tokio::test]
async fn a_cache_hit_runs_the_async_handler_then_the_sync_callbacks_once() {
    let fixture = Fixture::new(false);
    let cached_result = cached();
    let before = cached_result.clone();
    let worker = LoggingWorker::start(worker_settings(8, 2)).expect("the worker starts");
    assert_eq!(
        worker.enqueue(fixture.cache_hit_job(&cached_result)),
        Admission::Accepted
    );
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(
        fixture.calls.events(),
        ["build-with-cache-hit", "async", "sync"]
    );
    assert_eq!(cached_result, before);
}

#[tokio::test]
async fn a_failing_async_callback_is_counted_and_leaves_the_result_alone() {
    let fixture = Fixture::new(true);
    let cached_result = cached();
    let before = cached_result.clone();
    let worker = LoggingWorker::start(worker_settings(8, 2)).expect("the worker starts");
    assert_eq!(
        worker.enqueue(fixture.cache_hit_job(&cached_result)),
        Admission::Accepted
    );
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(fixture.counter.calls(), vec!["langfuse".to_owned()]);
    assert_eq!(
        fixture.calls.events(),
        ["build-with-cache-hit", "async", "sync"]
    );
    assert_eq!(cached_result, before);
}

#[tokio::test]
async fn a_full_queue_rejects_the_cache_hit_callbacks() {
    let fixture = Fixture::new(false);
    let cached_result = cached();
    let before = cached_result.clone();
    let worker = LoggingWorker::start(worker_settings(1, 1)).expect("the worker starts");
    assert_eq!(
        worker.enqueue(fixture.cache_hit_job(&cached_result)),
        Admission::Accepted
    );
    assert_eq!(
        worker.enqueue(fixture.cache_hit_job(&cached_result)),
        Admission::Rejected(Rejection::QueueFull)
    );
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert_eq!(worker.stats().rejected_full, 1);
    assert_eq!(
        fixture.calls.events(),
        ["build-with-cache-hit", "async", "sync"]
    );
    assert_eq!(cached_result, before);
}

#[rstest]
#[case::legacy_callable(Some("legacy_hook"), true)]
#[case::no_sync_callbacks(None, false)]
fn the_logging_sync_dispatch_reports_whether_it_dispatched(
    #[case] dynamic: Option<&str>,
    #[case] expected: bool,
) {
    let fixture = Fixture::new(false);
    let owned = fixture.owned_handler();
    let handler = borrowed_handler(&owned);
    let mut correlation = CorrelationContext::default();
    let mut state = state(None);
    state
        .litellm_params
        .insert("acompletion".to_owned(), Json::Bool(true));
    state.dynamic_callbacks.success = dynamic.map(|name| vec![name.to_owned()]);
    let request = success_request(&cached());
    let dispatch = LoggingSyncDispatch;
    assert_eq!(
        dispatch.dispatch(&handler, &mut state, &mut correlation, &request),
        expected
    );
    assert_eq!(
        state.model_call_details.get("has_logged_sync_success"),
        expected.then(|| Json::Bool(true)).as_ref()
    );
}
