use betterlite_models::decisions::Json;

use super::support::{DATASET_BODY, Reply, ScriptedRandom, logger, valid_kwargs};
use crate::argilla_logging::transport::ArgillaMethod;

const BATCH_SIZE: &str = "1";

#[tokio::test]
async fn posts_the_queue_as_a_json_array_and_clears_it() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", BATCH_SIZE)],
        &[Reply::ok(DATASET_BODY), Reply::ok("")],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.log_success_event(&valid_kwargs()).await;
    assert!(built.queue.pending().is_empty());
    let calls = built.transport.calls();
    assert_eq!(calls[1].method, ArgillaMethod::Post);
    assert_eq!(
        calls[1].url,
        "http://localhost:6900//api/v1/datasets/ds-1/records/bulk"
    );
    assert_eq!(calls[1].api_key, "argilla-key");
    assert_eq!(calls[1].timeout, None);
    let bodies = built.transport.bodies();
    let Json::List(records) = &bodies[0] else {
        panic!("the sync body is a JSON array");
    };
    assert_eq!(records.len(), 1);
    assert!(matches!(records[0], Json::Object(_)));
}

#[tokio::test]
async fn clears_the_queue_after_an_error_status() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", BATCH_SIZE)],
        &[Reply::ok(DATASET_BODY), Reply::status(500)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.log_success_event(&valid_kwargs()).await;
    assert!(built.queue.pending().is_empty());
    assert_eq!(built.transport.calls().len(), 2);
}

#[tokio::test]
async fn keeps_the_queue_when_the_post_never_answers() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", BATCH_SIZE)],
        &[Reply::ok(DATASET_BODY), Reply::ok("")],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.transport.start_failing();
    built.log_success_event(&valid_kwargs()).await;
    assert_eq!(built.queue.pending().len(), 1);
}
