use rstest::rstest;

use super::support::{DATASET_BODY, Reply, ScriptedRandom, logger, valid_kwargs};
use crate::argilla_logging::transport::ArgillaMethod;

#[rstest]
#[case::rate_zero_skips(&[("LANGSMITH_SAMPLING_RATE", "0")], 0.5_f64, false)]
#[case::rate_zero_logs_at_zero(&[("LANGSMITH_SAMPLING_RATE", "0")], 0.0_f64, true)]
#[case::digits_scale(&[("LANGSMITH_SAMPLING_RATE", "1")], 0.9_f64, true)]
#[case::decimal_falls_back(&[("LANGSMITH_SAMPLING_RATE", "0.5")], 0.9_f64, true)]
#[case::negative_falls_back(&[("LANGSMITH_SAMPLING_RATE", "-1")], 0.9_f64, true)]
#[case::argilla_rate_is_not_read(&[("ARGILLA_SAMPLING_RATE", "0")], 0.9_f64, true)]
#[tokio::test]
async fn samples_with_the_langsmith_rate(
    #[case] pairs: &[(&str, &str)],
    #[case] draw: f64,
    #[case] logged: bool,
) {
    let built = logger(
        pairs,
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[draw]),
    )
    .await;
    built.log_success_event(&valid_kwargs()).await;
    assert_eq!(!built.queue.pending().is_empty(), logged);
}

#[tokio::test]
async fn draws_a_fresh_sample_for_every_event() {
    let built = logger(
        &[("LANGSMITH_SAMPLING_RATE", "0")],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.5_f64, 0.0_f64]),
    )
    .await;
    built.log_success_event(&valid_kwargs()).await;
    built.log_success_event(&valid_kwargs()).await;
    assert_eq!(built.queue.pending().len(), 1);
}

#[tokio::test]
async fn sends_at_the_batch_size_threshold() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", "1")],
        &[Reply::ok(DATASET_BODY), Reply::ok("")],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.log_success_event(&valid_kwargs()).await;
    assert!(built.queue.pending().is_empty());
    let calls = built.transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].method, ArgillaMethod::Post);
}
