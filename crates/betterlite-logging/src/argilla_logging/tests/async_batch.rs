use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{DATASET_BODY, Reply, ScriptedRandom, logger, valid_kwargs};
use crate::argilla_logging::delivery::ASYNC_TIMEOUT;
use crate::argilla_logging::transport::ArgillaMethod;

#[tokio::test]
async fn puts_the_items_envelope_and_clears_on_success() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", "1")],
        &[Reply::ok(DATASET_BODY), Reply::ok("")],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.async_log_success_event(&valid_kwargs(), &[]).await;
    assert!(built.queue.pending().is_empty());
    let calls = built.transport.calls();
    assert_eq!(calls[1].method, ArgillaMethod::Put);
    assert_eq!(
        calls[1].url,
        "http://localhost:6900//api/v1/datasets/ds-1/records/bulk"
    );
    assert_eq!(calls[1].timeout, Some(ASYNC_TIMEOUT));
    assert_eq!(
        built.transport.bodies()[0],
        Json::Object(IndexMap::from([(
            "items".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([(
                "fields".to_owned(),
                Json::Object(IndexMap::from([(
                    "input".to_owned(),
                    super::support::messages_json(),
                )])),
            )]))]),
        )]))
    );
}

#[tokio::test]
async fn keeps_the_queue_on_an_error_status() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", "1")],
        &[Reply::ok(DATASET_BODY), Reply::status(500)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.async_log_success_event(&valid_kwargs(), &[]).await;
    assert_eq!(built.queue.pending().len(), 1);
}

#[tokio::test]
async fn keeps_the_queue_when_the_put_never_answers() {
    let built = logger(
        &[("ARGILLA_BATCH_SIZE", "1")],
        &[Reply::ok(DATASET_BODY), Reply::ok("")],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.transport.start_failing();
    built.async_log_success_event(&valid_kwargs(), &[]).await;
    assert_eq!(built.queue.pending().len(), 1);
}

#[tokio::test]
async fn sends_nothing_from_an_empty_flush() {
    let built = logger(&[], &[Reply::ok(DATASET_BODY)], ScriptedRandom::new(&[])).await;
    let report = built.flush_queue().await;
    assert_eq!(report.batch, 0);
    assert_eq!(built.transport.calls().len(), 1);
}
