use super::support::{DATASET_BODY, Reply, ScriptedRandom, logger, valid_kwargs};
use crate::argilla_logging::extraction::CallbackKwargs;
use crate::argilla_logging::transport::ArgillaMethod;

#[tokio::test]
async fn samples_with_the_argilla_rate_and_queues_the_record() {
    let built = logger(
        &[("ARGILLA_SAMPLING_RATE", "0")],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.async_log_failure_event(&valid_kwargs()).await;
    assert_eq!(built.queue.pending().len(), 1);
}

#[tokio::test]
async fn skips_when_the_draw_exceeds_the_rate() {
    let built = logger(
        &[("ARGILLA_SAMPLING_RATE", "0")],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.5_f64]),
    )
    .await;
    built.async_log_failure_event(&valid_kwargs()).await;
    assert!(built.queue.pending().is_empty());
}

#[tokio::test]
async fn queues_without_running_any_dataset_hook() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.async_log_failure_event(&valid_kwargs()).await;
    assert_eq!(built.queue.pending().len(), 1);
    assert_eq!(built.transport.calls().len(), 1);
}

#[tokio::test]
async fn drops_a_record_it_cannot_prepare() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built
        .async_log_failure_event(&CallbackKwargs::default())
        .await;
    assert!(built.queue.pending().is_empty());
}

#[tokio::test]
async fn flushes_with_a_put_at_the_threshold() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", "1")],
        &[Reply::ok(DATASET_BODY), Reply::ok("")],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.async_log_failure_event(&valid_kwargs()).await;
    assert!(built.queue.pending().is_empty());
    let calls = built.transport.calls();
    assert_eq!(calls[1].method, ArgillaMethod::Put);
}
