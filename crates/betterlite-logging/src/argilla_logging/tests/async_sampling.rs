use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    DATASET_BODY, HookEffect, RecordingHook, Reply, ScriptedRandom, logger, valid_kwargs,
};
use crate::argilla_logging::hooks::DatasetHook;

#[rstest]
#[case::unset(&[], 0.9_f64, true)]
#[case::rate_zero_skips(&[("ARGILLA_SAMPLING_RATE", "0")], 0.5_f64, false)]
#[case::rate_zero_logs_at_zero(&[("ARGILLA_SAMPLING_RATE", "0")], 0.0_f64, true)]
#[case::decimal_falls_back(&[("ARGILLA_SAMPLING_RATE", "0.5")], 0.9_f64, true)]
#[case::langsmith_rate_is_not_read(&[("LANGSMITH_SAMPLING_RATE", "0")], 0.9_f64, true)]
#[tokio::test]
async fn samples_with_the_argilla_rate(
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
    built.async_log_success_event(&valid_kwargs(), &[]).await;
    assert_eq!(!built.queue.pending().is_empty(), logged);
}

#[tokio::test]
async fn chains_every_hook_before_queueing() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    let first = RecordingHook::new(HookEffect::Rewrite);
    let second = RecordingHook::new(HookEffect::Pass);
    let hooks: [&dyn DatasetHook; 2] = [&first, &second];
    built.async_log_success_event(&valid_kwargs(), &hooks).await;
    let queued = built.queue.pending();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].fields["input"],
        Json::Text("rewritten".to_owned())
    );
    assert_eq!(first.seen().len(), 1);
    assert_eq!(
        second.seen()[0].fields["input"],
        Json::Text("rewritten".to_owned())
    );
    assert_eq!(queued[0], second.seen()[0]);
}

#[tokio::test]
async fn a_filtering_hook_drops_the_record_and_stops_the_chain() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    let first = RecordingHook::new(HookEffect::Pass);
    let filter = RecordingHook::new(HookEffect::Filter);
    let after = RecordingHook::new(HookEffect::Pass);
    let hooks: [&dyn DatasetHook; 3] = [&first, &filter, &after];
    built.async_log_success_event(&valid_kwargs(), &hooks).await;
    assert!(built.queue.pending().is_empty());
    assert_eq!(filter.seen().len(), 1);
    assert!(after.seen().is_empty());
}

#[tokio::test]
async fn a_not_implemented_hook_is_skipped() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    let declining = RecordingHook::new(HookEffect::NotImplemented);
    let after = RecordingHook::new(HookEffect::Pass);
    let hooks: [&dyn DatasetHook; 2] = [&declining, &after];
    built.async_log_success_event(&valid_kwargs(), &hooks).await;
    assert_eq!(built.queue.pending().len(), 1);
    assert_eq!(after.seen().len(), 1);
}

#[tokio::test]
async fn a_failing_hook_drops_the_record() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    let failing = RecordingHook::new(HookEffect::Fail);
    let hooks: [&dyn DatasetHook; 1] = [&failing];
    built.async_log_success_event(&valid_kwargs(), &hooks).await;
    assert!(built.queue.pending().is_empty());
    assert_eq!(failing.seen().len(), 1);
}
