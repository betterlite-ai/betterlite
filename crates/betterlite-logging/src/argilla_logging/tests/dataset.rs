use rstest::rstest;

use super::support::{
    DATASET_BODY, Reply, ScriptedRandom, ScriptedTransport, logger, settings, valid_kwargs,
};
use crate::argilla_logging::credentials::DatasetError;
use crate::argilla_logging::logger::ArgillaLogger;
use crate::argilla_logging::transport::ArgillaMethod;

#[tokio::test]
async fn keeps_the_first_dataset_id() {
    let built = logger(&[], &[Reply::ok(DATASET_BODY)], ScriptedRandom::new(&[])).await;
    assert_eq!(built.credentials.dataset_name, "ds-1");
}

#[rstest]
#[case::empty_list(r#"{"items": []}"#)]
#[case::absent_items(r#"{"data": []}"#)]
#[case::null_items(r#"{"items": null}"#)]
#[case::items_not_a_list(r#"{"items": {"id": "ds-2"}}"#)]
#[tokio::test]
async fn keeps_the_name_when_the_lookup_returns_no_item(#[case] body: &str) {
    let built = logger(&[], &[Reply::ok(body)], ScriptedRandom::new(&[])).await;
    assert_eq!(built.credentials.dataset_name, "litellm-completion");
}

#[tokio::test]
async fn fails_when_the_first_item_has_no_id() {
    let transport = ScriptedTransport::answering(&[Reply::ok(r#"{"items": [{"name": "x"}]}"#)]);
    let result = ArgillaLogger::connect(
        settings(&[("ARGILLA_API_KEY", "key")]),
        transport,
        ScriptedRandom::new(&[]),
    )
    .await;
    assert!(matches!(result, Err(DatasetError::Id)));
}

#[tokio::test]
async fn fails_when_the_lookup_answer_is_not_json() {
    let transport = ScriptedTransport::answering(&[Reply::ok("not json")]);
    let result = ArgillaLogger::connect(
        settings(&[("ARGILLA_API_KEY", "key")]),
        transport,
        ScriptedRandom::new(&[]),
    )
    .await;
    assert!(matches!(result, Err(DatasetError::Body { .. })));
}

#[tokio::test]
async fn fails_when_the_lookup_does_not_answer() {
    let transport = ScriptedTransport::answering(&[]);
    transport.start_failing();
    let result = ArgillaLogger::connect(
        settings(&[("ARGILLA_API_KEY", "key")]),
        transport,
        ScriptedRandom::new(&[]),
    )
    .await;
    assert!(matches!(result, Err(DatasetError::Transport { .. })));
}

#[tokio::test]
async fn resolves_the_dataset_once_per_session() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64, 0.0_f64]),
    )
    .await;
    built.log_success_event(&valid_kwargs()).await;
    built.log_success_event(&valid_kwargs()).await;
    built.async_log_failure_event(&valid_kwargs()).await;
    let calls = built.transport.calls();
    let lookups = calls
        .iter()
        .filter(|call| call.method == ArgillaMethod::Get)
        .count();
    assert_eq!(lookups, 1);
    assert_eq!(built.queue.pending().len(), 3);
}
